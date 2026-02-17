use crate::models::{Node, NodeStatus, Poll, PollProtocol, PollResult};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, Result};
use std::collections::HashMap;
use uuid::Uuid;

/// Insert a new node into the database
pub fn insert_node(conn: &Connection, node: &Node) -> Result<()> {
    let created_at = Utc::now().to_rfc3339();
    let updated_at = created_at.clone();

    let addresses_json = serde_json::to_string(&node.addresses).unwrap();
    let metadata_json = node.metadata.as_ref().map(|m| serde_json::to_string(m).unwrap());
    let tags_json = serde_json::to_string(&node.tags).unwrap();
    let status_str = format!("{:?}", node.status);
    let parent_id_str = node.parent_node_id.map(|id| id.to_string());

    conn.execute(
        "INSERT INTO nodes (id, name, addresses, polling_profile, status, parent_node_id, metadata, tags, consecutive_failures, consecutive_successes, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            node.id.to_string(),
            node.name,
            addresses_json,
            node.polling_profile,
            status_str,
            parent_id_str,
            metadata_json,
            tags_json,
            node.consecutive_failures,
            node.consecutive_successes,
            created_at,
            updated_at,
        ],
    )?;

    Ok(())
}

/// Get a single node by ID
pub fn get_node(conn: &Connection, id: &Uuid) -> Result<Option<Node>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, addresses, polling_profile, status, parent_node_id, metadata, tags, consecutive_failures, consecutive_successes
         FROM nodes WHERE id = ?1"
    )?;

    let result = stmt.query_row([id.to_string()], |row| {
        let addresses_json: String = row.get(2)?;
        let status_str: String = row.get(4)?;
        let parent_id_str: Option<String> = row.get(5)?;
        let metadata_json: Option<String> = row.get(6)?;
        let tags_json: String = row.get(7)?;

        let addresses: Vec<String> = serde_json::from_str(&addresses_json).unwrap();
        let metadata: Option<HashMap<String, String>> = metadata_json.and_then(|j| serde_json::from_str(&j).ok());
        let tags: Vec<String> = serde_json::from_str(&tags_json).unwrap_or_default();
        let parent_node_id = parent_id_str.and_then(|s| Uuid::parse_str(&s).ok());

        let status = match status_str.as_str() {
            "Up" => NodeStatus::Up,
            "Down" => NodeStatus::Down,
            _ => NodeStatus::Unknown,
        };

        Ok(Node {
            id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap(),
            name: row.get(1)?,
            addresses,
            polling_profile: row.get(3)?,
            status,
            effective_status: status.into(),  // Will be recomputed later
            parent_node_id,
            metadata,
            tags,
            consecutive_failures: row.get(8)?,
            consecutive_successes: row.get(9)?,
        })
    });

    match result {
        Ok(node) => Ok(Some(node)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e),
    }
}

/// Get all nodes
pub fn get_all_nodes(conn: &Connection) -> Result<Vec<Node>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, addresses, polling_profile, status, parent_node_id, metadata, tags, consecutive_failures, consecutive_successes
         FROM nodes"
    )?;

    let nodes = stmt.query_map([], |row| {
        let addresses_json: String = row.get(2)?;
        let status_str: String = row.get(4)?;
        let parent_id_str: Option<String> = row.get(5)?;
        let metadata_json: Option<String> = row.get(6)?;
        let tags_json: String = row.get(7)?;

        let addresses: Vec<String> = serde_json::from_str(&addresses_json).unwrap();
        let metadata: Option<HashMap<String, String>> = metadata_json.and_then(|j| serde_json::from_str(&j).ok());
        let tags: Vec<String> = serde_json::from_str(&tags_json).unwrap_or_default();
        let parent_node_id = parent_id_str.and_then(|s| Uuid::parse_str(&s).ok());

        let status = match status_str.as_str() {
            "Up" => NodeStatus::Up,
            "Down" => NodeStatus::Down,
            _ => NodeStatus::Unknown,
        };

        Ok(Node {
            id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap(),
            name: row.get(1)?,
            addresses,
            polling_profile: row.get(3)?,
            status,
            effective_status: status.into(),
            parent_node_id,
            metadata,
            tags,
            consecutive_failures: row.get(8)?,
            consecutive_successes: row.get(9)?,
        })
    })?.collect::<Result<Vec<_>>>()?;

    Ok(nodes)
}

/// Update a node's status and counters
pub fn update_node_status(
    conn: &Connection,
    id: &Uuid,
    status: NodeStatus,
    consecutive_failures: u32,
    consecutive_successes: u32,
) -> Result<()> {
    let updated_at = Utc::now().to_rfc3339();
    let status_str = format!("{:?}", status);

    conn.execute(
        "UPDATE nodes SET status = ?1, consecutive_failures = ?2, consecutive_successes = ?3, updated_at = ?4
         WHERE id = ?5",
        params![status_str, consecutive_failures, consecutive_successes, updated_at, id.to_string()],
    )?;

    Ok(())
}

/// Delete a node
pub fn delete_node(conn: &Connection, id: &Uuid) -> Result<()> {
    conn.execute("DELETE FROM nodes WHERE id = ?1", [id.to_string()])?;
    Ok(())
}

/// Insert a poll configuration
pub fn insert_poll(conn: &Connection, poll: &Poll) -> Result<()> {
    let protocol_json = serde_json::to_string(&poll.protocol).unwrap();
    let interval_secs = poll.interval.as_secs() as i64;
    let timeout_ms = poll.timeout.as_millis() as i64;

    conn.execute(
        "INSERT INTO polls (id, node_id, protocol, interval_secs, timeout_ms, retries, failure_threshold, recovery_threshold)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            poll.id.to_string(),
            poll.node_id.to_string(),
            protocol_json,
            interval_secs,
            timeout_ms,
            poll.retries,
            poll.failure_threshold,
            poll.recovery_threshold,
        ],
    )?;

    Ok(())
}

/// Get all polls for a specific node
pub fn get_polls_for_node(conn: &Connection, node_id: &Uuid) -> Result<Vec<Poll>> {
    let mut stmt = conn.prepare(
        "SELECT id, node_id, protocol, interval_secs, timeout_ms, retries, failure_threshold, recovery_threshold
         FROM polls WHERE node_id = ?1"
    )?;

    let polls = stmt.query_map([node_id.to_string()], |row| {
        let protocol_json: String = row.get(2)?;
        let protocol: PollProtocol = serde_json::from_str(&protocol_json).unwrap();
        let interval_secs: i64 = row.get(3)?;
        let timeout_ms: i64 = row.get(4)?;

        Ok(Poll {
            id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap(),
            node_id: Uuid::parse_str(&row.get::<_, String>(1)?).unwrap(),
            protocol,
            interval: std::time::Duration::from_secs(interval_secs as u64),
            timeout: std::time::Duration::from_millis(timeout_ms as u64),
            retries: row.get(5)?,
            failure_threshold: row.get(6)?,
            recovery_threshold: row.get(7)?,
        })
    })?.collect::<Result<Vec<_>>>()?;

    Ok(polls)
}

/// Insert a poll result
pub fn insert_poll_result(conn: &Connection, result: &PollResult) -> Result<()> {
    let timestamp_str = result.timestamp.to_rfc3339();
    let success_int = if result.success { 1 } else { 0 };
    let latency_us = result.latency.map(|d| d.as_micros() as i64);

    conn.execute(
        "INSERT INTO poll_results (id, poll_id, node_id, timestamp, success, latency_us, error)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            result.id.to_string(),
            result.poll_id.to_string(),
            result.node_id.to_string(),
            timestamp_str,
            success_int,
            latency_us,
            result.error.as_ref(),
        ],
    )?;

    Ok(())
}

/// Get recent poll results for a node (limited)
pub fn get_poll_results(conn: &Connection, node_id: &Uuid, limit: usize) -> Result<Vec<PollResult>> {
    let mut stmt = conn.prepare(
        "SELECT id, poll_id, node_id, timestamp, success, latency_us, error
         FROM poll_results WHERE node_id = ?1
         ORDER BY timestamp DESC
         LIMIT ?2"
    )?;

    let results = stmt.query_map(params![node_id.to_string(), limit as i64], |row| {
        let timestamp_str: String = row.get(3)?;
        let timestamp = DateTime::parse_from_rfc3339(&timestamp_str).unwrap().with_timezone(&Utc);
        let success_int: i32 = row.get(4)?;
        let latency_us: Option<i64> = row.get(5)?;
        let latency = latency_us.map(|us| std::time::Duration::from_micros(us as u64));

        Ok(PollResult {
            id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap(),
            poll_id: Uuid::parse_str(&row.get::<_, String>(1)?).unwrap(),
            node_id: Uuid::parse_str(&row.get::<_, String>(2)?).unwrap(),
            timestamp,
            success: success_int != 0,
            latency,
            error: row.get(6)?,
        })
    })?.collect::<Result<Vec<_>>>()?;

    Ok(results)
}

/// Purge old poll results (spec §7: bounded history retention)
pub fn purge_old_results(conn: &Connection, retention_days: u32) -> Result<u64> {
    let cutoff = Utc::now() - chrono::Duration::days(retention_days as i64);
    let cutoff_str = cutoff.to_rfc3339();

    let deleted = conn.execute(
        "DELETE FROM poll_results WHERE timestamp < ?1",
        [cutoff_str],
    )?;

    Ok(deleted as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::init_database;

    #[test]
    fn test_insert_and_get_node() {
        let conn = init_database(":memory:").unwrap();
        let node = Node::new("test-router", vec!["192.168.1.1".to_string()]);
        let node_id = node.id;

        insert_node(&conn, &node).unwrap();

        let retrieved = get_node(&conn, &node_id).unwrap();
        assert!(retrieved.is_some());
        let retrieved = retrieved.unwrap();
        assert_eq!(retrieved.id, node_id);
        assert_eq!(retrieved.name, "test-router");
        assert_eq!(retrieved.addresses, vec!["192.168.1.1".to_string()]);
    }

    #[test]
    fn test_update_node_status() {
        let conn = init_database(":memory:").unwrap();
        let node = Node::new("test-router", vec!["192.168.1.1".to_string()]);
        let node_id = node.id;

        insert_node(&conn, &node).unwrap();

        update_node_status(&conn, &node_id, NodeStatus::Up, 0, 5).unwrap();

        let retrieved = get_node(&conn, &node_id).unwrap().unwrap();
        assert_eq!(retrieved.status, NodeStatus::Up);
        assert_eq!(retrieved.consecutive_successes, 5);
        assert_eq!(retrieved.consecutive_failures, 0);
    }

    #[test]
    fn test_insert_and_retrieve_poll_results() {
        let conn = init_database(":memory:").unwrap();
        let node = Node::new("test-router", vec!["192.168.1.1".to_string()]);
        let node_id = node.id;
        insert_node(&conn, &node).unwrap();

        let poll = Poll::new(
            node_id,
            PollProtocol::Icmp,
            std::time::Duration::from_secs(30),
            std::time::Duration::from_secs(5),
        );
        let poll_id = poll.id;
        insert_poll(&conn, &poll).unwrap();

        let result = PollResult::success(poll_id, node_id, std::time::Duration::from_millis(25));
        insert_poll_result(&conn, &result).unwrap();

        let results = get_poll_results(&conn, &node_id, 10).unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].success);
    }

    #[test]
    fn test_purge_old_results() {
        let conn = init_database(":memory:").unwrap();
        let node = Node::new("test-router", vec!["192.168.1.1".to_string()]);
        let node_id = node.id;
        insert_node(&conn, &node).unwrap();

        let poll = Poll::new(
            node_id,
            PollProtocol::Icmp,
            std::time::Duration::from_secs(30),
            std::time::Duration::from_secs(5),
        );
        let poll_id = poll.id;
        insert_poll(&conn, &poll).unwrap();

        // Insert an old result
        let mut old_result = PollResult::success(poll_id, node_id, std::time::Duration::from_millis(25));
        old_result.timestamp = Utc::now() - chrono::Duration::days(60);
        insert_poll_result(&conn, &old_result).unwrap();

        // Insert a recent result
        let recent_result = PollResult::success(poll_id, node_id, std::time::Duration::from_millis(25));
        insert_poll_result(&conn, &recent_result).unwrap();

        // Purge results older than 30 days
        let deleted = purge_old_results(&conn, 30).unwrap();
        assert_eq!(deleted, 1);

        // Verify only recent result remains
        let results = get_poll_results(&conn, &node_id, 10).unwrap();
        assert_eq!(results.len(), 1);
    }
}
