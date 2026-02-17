use rusqlite::{Connection, Result};

/// SQL schema for the Vigil database
/// Spec §7: SQLite persistence with WAL mode

pub const SCHEMA_NODES: &str = r#"
CREATE TABLE IF NOT EXISTS nodes (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    addresses TEXT NOT NULL,
    polling_profile TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'Unknown',
    parent_node_id TEXT,
    metadata TEXT,
    tags TEXT NOT NULL DEFAULT '[]',
    consecutive_failures INTEGER NOT NULL DEFAULT 0,
    consecutive_successes INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
)"#;

pub const SCHEMA_POLLS: &str = r#"
CREATE TABLE IF NOT EXISTS polls (
    id TEXT PRIMARY KEY,
    node_id TEXT NOT NULL REFERENCES nodes(id),
    protocol TEXT NOT NULL,
    interval_secs INTEGER NOT NULL,
    timeout_ms INTEGER NOT NULL,
    retries INTEGER NOT NULL DEFAULT 0,
    failure_threshold INTEGER NOT NULL DEFAULT 3,
    recovery_threshold INTEGER NOT NULL DEFAULT 1
)"#;

pub const SCHEMA_POLL_RESULTS: &str = r#"
CREATE TABLE IF NOT EXISTS poll_results (
    id TEXT PRIMARY KEY,
    poll_id TEXT NOT NULL REFERENCES polls(id),
    node_id TEXT NOT NULL REFERENCES nodes(id),
    timestamp TEXT NOT NULL,
    success INTEGER NOT NULL,
    latency_us INTEGER,
    error TEXT
)"#;

pub const INDEX_POLL_RESULTS_NODE_TS: &str = r#"
CREATE INDEX IF NOT EXISTS idx_poll_results_node_ts
ON poll_results(node_id, timestamp)
"#;

pub const INDEX_POLL_RESULTS_TS: &str = r#"
CREATE INDEX IF NOT EXISTS idx_poll_results_ts
ON poll_results(timestamp)
"#;

/// Initialize a new database connection with schema
///
/// This function:
/// 1. Opens/creates the SQLite database at the specified path
/// 2. Enables WAL mode for better concurrency (spec §7)
/// 3. Enables foreign keys
/// 4. Creates all tables and indices
///
/// # Arguments
/// * `path` - Path to the SQLite database file
///
/// # Returns
/// A configured database connection ready for use
pub fn init_database(path: &str) -> Result<Connection> {
    let conn = Connection::open(path)?;

    // Enable WAL mode for better concurrency and crash safety (spec §7)
    conn.execute_batch("PRAGMA journal_mode=WAL")?;

    // Enable foreign key constraints
    conn.execute_batch("PRAGMA foreign_keys=ON")?;

    // Create all tables
    conn.execute_batch(SCHEMA_NODES)?;
    conn.execute_batch(SCHEMA_POLLS)?;
    conn.execute_batch(SCHEMA_POLL_RESULTS)?;

    // Create indices
    conn.execute_batch(INDEX_POLL_RESULTS_NODE_TS)?;
    conn.execute_batch(INDEX_POLL_RESULTS_TS)?;

    Ok(conn)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_init_database() {
        // Use in-memory database for testing
        let conn = init_database(":memory:").expect("Failed to init database");

        // Note: In-memory databases return "memory" for journal_mode, not "wal"
        // WAL mode works correctly for file-based databases
        let journal_mode: String = conn
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .expect("Failed to query journal_mode");
        // In-memory databases use "memory" journal mode
        assert!(journal_mode.to_lowercase() == "memory" || journal_mode.to_lowercase() == "wal");

        // Verify foreign keys are enabled
        let foreign_keys: i32 = conn
            .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
            .expect("Failed to query foreign_keys");
        assert_eq!(foreign_keys, 1);

        // Verify tables exist
        let tables: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<Vec<String>, _>>()
            .unwrap();

        assert!(tables.contains(&"nodes".to_string()));
        assert!(tables.contains(&"polls".to_string()));
        assert!(tables.contains(&"poll_results".to_string()));

        // Verify indices exist
        let indices: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='index' ORDER BY name")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<Vec<String>, _>>()
            .unwrap();

        assert!(indices.contains(&"idx_poll_results_node_ts".to_string()));
        assert!(indices.contains(&"idx_poll_results_ts".to_string()));
    }

    #[test]
    fn test_foreign_key_constraint() {
        let conn = init_database(":memory:").expect("Failed to init database");

        // Try to insert a poll with non-existent node_id
        let result = conn.execute(
            "INSERT INTO polls (id, node_id, protocol, interval_secs, timeout_ms)
             VALUES ('poll1', 'nonexistent', '{}', 30, 5000)",
            [],
        );

        // Should fail due to foreign key constraint
        assert!(result.is_err());
    }
}
