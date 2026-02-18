use crate::db::repository::{
    delete_node, get_all_nodes, get_polls_for_node, insert_node, insert_poll, insert_poll_result,
    update_node_status,
};
use crate::engine::dependency::recompute_all_effective_statuses;
use crate::engine::scheduler::Scheduler;
use crate::engine::status::compute_status_transition;
use crate::models::{EffectiveStatus, Node, NodeStatus, Poll, PollResult};
use rusqlite::Connection;
use std::collections::HashMap;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

// ── Error type ────────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("node not found: {0}")]
    NodeNotFound(Uuid),
    #[error("node has no addresses")]
    NoAddress,
}

// ── Command channel ───────────────────────────────────────────────────────────

/// Commands sent from external callers (e.g. API handlers) to the engine loop.
///
/// Each variant carries a `reply` oneshot so the caller can await the result.
pub enum EngineCommand {
    AddNode {
        node: Node,
        reply: oneshot::Sender<Result<(), EngineError>>,
    },
    RemoveNode {
        node_id: Uuid,
        reply: oneshot::Sender<Result<(), EngineError>>,
    },
    GetNode {
        node_id: Uuid,
        reply: oneshot::Sender<Option<Node>>,
    },
    GetAllNodes {
        reply: oneshot::Sender<Vec<Node>>,
    },
    AddPoll {
        poll: Poll,
        reply: oneshot::Sender<Result<(), EngineError>>,
    },
    GetStatus {
        node_id: Uuid,
        reply: oneshot::Sender<Option<(NodeStatus, EffectiveStatus)>>,
    },
    Shutdown,
}

// ── EngineHandle ──────────────────────────────────────────────────────────────

/// Cheap, cloneable handle for interacting with a running engine.
///
/// `EngineHandle` is `Send`; it contains only a channel sender and a
/// `CancellationToken`.  The engine itself is `!Send` (due to
/// `rusqlite::Connection`) and should be driven via
/// `tokio::task::spawn_local` within a `LocalSet`.
#[derive(Clone)]
pub struct EngineHandle {
    cmd_tx: mpsc::Sender<EngineCommand>,
    shutdown: CancellationToken,
}

impl EngineHandle {
    /// Request a graceful shutdown.
    pub fn shutdown(&self) {
        self.shutdown.cancel();
    }

    /// Add a node to the engine and persist it.
    pub async fn add_node(&self, node: Node) -> Result<(), EngineError> {
        let (tx, rx) = oneshot::channel();
        let _ = self.cmd_tx.send(EngineCommand::AddNode { node, reply: tx }).await;
        rx.await.unwrap_or(Err(EngineError::NodeNotFound(Uuid::nil())))
    }

    /// Remove a node (and its scheduled polls) from the engine.
    pub async fn remove_node(&self, node_id: Uuid) -> Result<(), EngineError> {
        let (tx, rx) = oneshot::channel();
        let _ = self.cmd_tx.send(EngineCommand::RemoveNode { node_id, reply: tx }).await;
        rx.await.unwrap_or(Err(EngineError::NodeNotFound(node_id)))
    }

    /// Retrieve a node by ID.
    pub async fn get_node(&self, node_id: Uuid) -> Option<Node> {
        let (tx, rx) = oneshot::channel();
        let _ = self.cmd_tx.send(EngineCommand::GetNode { node_id, reply: tx }).await;
        rx.await.ok().flatten()
    }

    /// Retrieve all nodes.
    pub async fn get_all_nodes(&self) -> Vec<Node> {
        let (tx, rx) = oneshot::channel();
        let _ = self.cmd_tx.send(EngineCommand::GetAllNodes { reply: tx }).await;
        rx.await.unwrap_or_default()
    }

    /// Add a poll for an existing node and register it with the scheduler.
    pub async fn add_poll(&self, poll: Poll) -> Result<(), EngineError> {
        let (tx, rx) = oneshot::channel();
        let _ = self.cmd_tx.send(EngineCommand::AddPoll { poll, reply: tx }).await;
        rx.await.unwrap_or(Err(EngineError::NoAddress))
    }

    /// Get the current and effective status for a node.
    pub async fn get_status(
        &self,
        node_id: Uuid,
    ) -> Option<(NodeStatus, EffectiveStatus)> {
        let (tx, rx) = oneshot::channel();
        let _ = self.cmd_tx.send(EngineCommand::GetStatus { node_id, reply: tx }).await;
        rx.await.ok().flatten()
    }
}

// ── Engine ────────────────────────────────────────────────────────────────────

/// The engine is the single logical writer for all monitoring state.
///
/// It owns:
/// - An in-memory map of nodes and polls (authoritative runtime state)
/// - A `Scheduler` that fires poll tasks and feeds results back
/// - A `rusqlite::Connection` for persistence
///
/// Because `rusqlite::Connection` is `!Send`, the engine must be driven
/// inside a `tokio::task::LocalSet` (e.g. `tokio::task::spawn_local`).
/// The `EngineHandle` that callers hold is `Send`.
pub struct Engine {
    nodes: HashMap<Uuid, Node>,
    polls: HashMap<Uuid, Poll>,
    scheduler: Scheduler,
    db: Connection,
    result_rx: mpsc::Receiver<PollResult>,
    cmd_rx: mpsc::Receiver<EngineCommand>,
    shutdown: CancellationToken,
}

impl Engine {
    /// Initialise the engine from an open database connection.
    ///
    /// Loads all nodes and their polls from the DB, registers every poll
    /// with the scheduler, and returns both the engine and an
    /// `EngineHandle` for external callers.
    pub fn new(
        db: Connection,
        max_concurrent: usize,
    ) -> Result<(Self, EngineHandle), EngineError> {
        let (result_tx, result_rx) = mpsc::channel(1024);
        let (cmd_tx, cmd_rx) = mpsc::channel(256);
        let shutdown = CancellationToken::new();

        let mut scheduler = Scheduler::new(result_tx, max_concurrent);

        // Load nodes from DB.
        let node_list = get_all_nodes(&db)?;
        let mut nodes: HashMap<Uuid, Node> = node_list.into_iter().map(|n| (n.id, n)).collect();

        // Recompute effective statuses from persistent state.
        let eff_updates = recompute_all_effective_statuses(&nodes);
        for (id, eff) in eff_updates {
            if let Some(n) = nodes.get_mut(&id) {
                n.effective_status = eff;
            }
        }

        // Load polls and register with scheduler.
        let mut polls: HashMap<Uuid, Poll> = HashMap::new();
        for node in nodes.values() {
            let node_polls = get_polls_for_node(&db, &node.id)?;
            for poll in node_polls {
                if let Some(address) = node.addresses.first() {
                    scheduler.add_poll(poll.clone(), address.clone());
                }
                polls.insert(poll.id, poll);
            }
        }

        let engine = Engine {
            nodes,
            polls,
            scheduler,
            db,
            result_rx,
            cmd_rx,
            shutdown: shutdown.clone(),
        };

        let handle = EngineHandle { cmd_tx, shutdown };

        Ok((engine, handle))
    }

    // ── Main loop ─────────────────────────────────────────────────────────────

    /// Run the engine until shutdown is requested.
    ///
    /// Drives three concurrent concerns:
    /// 1. Poll results arriving from the scheduler's tasks
    /// 2. Management commands from external callers via `EngineHandle`
    /// 3. Periodic scheduler tick (every 100 ms)
    pub async fn run(&mut self) -> Result<(), EngineError> {
        tracing::info!("Engine started");

        loop {
            tokio::select! {
                biased;

                // 1. Management commands have highest priority.
                Some(cmd) = self.cmd_rx.recv() => {
                    let stop = matches!(cmd, EngineCommand::Shutdown);
                    self.handle_command(cmd);
                    if stop { break; }
                }

                // 2. Poll results from scheduler tasks.
                Some(result) = self.result_rx.recv() => {
                    if let Err(e) = self.handle_poll_result(result) {
                        tracing::error!("Error handling poll result: {e}");
                    }
                }

                // 3. Periodic scheduler tick.
                _ = tokio::time::sleep(Duration::from_millis(100)) => {
                    self.scheduler.tick().await;
                }

                // 4. Cancellation token (SIGTERM / signal handler in main).
                _ = self.shutdown.cancelled() => {
                    tracing::info!("Engine shutting down");
                    break;
                }
            }
        }

        tracing::info!("Engine stopped");
        Ok(())
    }

    // ── Poll result handling ──────────────────────────────────────────────────

    /// Process one completed poll result.
    ///
    /// Pipeline (spec §4, §5, §6):
    /// 1. Look up the poll to get failure/recovery thresholds.
    /// 2. Look up the node for current status and counters.
    /// 3. Compute the status transition (pure state machine, Stage 2).
    /// 4. Apply the transition to the in-memory node.
    /// 5. Recompute effective_status for all nodes (Stage 3 propagation).
    /// 6. Log any status change at info level.
    /// 7. Persist: update node row + append poll result row.
    pub fn handle_poll_result(&mut self, result: PollResult) -> Result<(), EngineError> {
        // Step 1 — thresholds from poll config.
        let (failure_threshold, recovery_threshold) = self
            .polls
            .get(&result.poll_id)
            .map(|p| (p.failure_threshold, p.recovery_threshold))
            .unwrap_or((3, 1));

        // Step 2 — current node state (read-only snapshot before mutation).
        let (current_status, consec_fail, consec_succ, node_id, node_name) = {
            let Some(node) = self.nodes.get(&result.node_id) else {
                tracing::warn!("Poll result for unknown node {}", result.node_id);
                return Ok(());
            };
            (
                node.status,
                node.consecutive_failures,
                node.consecutive_successes,
                node.id,
                node.name.clone(),
            )
        };

        // Step 3 — pure status transition.
        let transition = compute_status_transition(
            current_status,
            consec_fail,
            consec_succ,
            failure_threshold,
            recovery_threshold,
            result.success,
        );

        // Step 4 — apply in-memory.
        let node = self.nodes.get_mut(&result.node_id).unwrap();
        node.status = transition.new_status;
        node.consecutive_failures = transition.new_consecutive_failures;
        node.consecutive_successes = transition.new_consecutive_successes;

        // Step 5 — recompute effective statuses across all nodes.
        let eff_updates = recompute_all_effective_statuses(&self.nodes);
        for (id, eff) in eff_updates {
            if let Some(n) = self.nodes.get_mut(&id) {
                n.effective_status = eff;
            }
        }

        // Step 6 — log transitions.
        if transition.changed {
            tracing::info!(
                "Node {node_name}: {:?} → {:?}",
                transition.previous_status,
                transition.new_status
            );
        }

        // Step 7 — persist.
        update_node_status(
            &self.db,
            &node_id,
            transition.new_status,
            transition.new_consecutive_failures,
            transition.new_consecutive_successes,
        )?;
        insert_poll_result(&self.db, &result)?;

        Ok(())
    }

    // ── Command handling ──────────────────────────────────────────────────────

    fn handle_command(&mut self, cmd: EngineCommand) {
        match cmd {
            EngineCommand::AddNode { node, reply } => {
                let result = self.cmd_add_node(node);
                let _ = reply.send(result);
            }
            EngineCommand::RemoveNode { node_id, reply } => {
                let result = self.cmd_remove_node(&node_id);
                let _ = reply.send(result);
            }
            EngineCommand::GetNode { node_id, reply } => {
                let _ = reply.send(self.nodes.get(&node_id).cloned());
            }
            EngineCommand::GetAllNodes { reply } => {
                let _ = reply.send(self.nodes.values().cloned().collect());
            }
            EngineCommand::AddPoll { poll, reply } => {
                let result = self.cmd_add_poll(poll);
                let _ = reply.send(result);
            }
            EngineCommand::GetStatus { node_id, reply } => {
                let status = self.nodes.get(&node_id).map(|n| (n.status, n.effective_status));
                let _ = reply.send(status);
            }
            EngineCommand::Shutdown => {
                // Handled by the run loop directly; nothing to do here.
            }
        }
    }

    fn cmd_add_node(&mut self, node: Node) -> Result<(), EngineError> {
        insert_node(&self.db, &node)?;
        self.nodes.insert(node.id, node);
        // Recompute effective statuses in case this node is a child of another.
        let eff_updates = recompute_all_effective_statuses(&self.nodes);
        for (id, eff) in eff_updates {
            if let Some(n) = self.nodes.get_mut(&id) {
                n.effective_status = eff;
            }
        }
        Ok(())
    }

    fn cmd_remove_node(&mut self, node_id: &Uuid) -> Result<(), EngineError> {
        if !self.nodes.contains_key(node_id) {
            return Err(EngineError::NodeNotFound(*node_id));
        }
        // Remove all polls belonging to this node from scheduler + in-memory map.
        let poll_ids: Vec<Uuid> = self
            .polls
            .values()
            .filter(|p| &p.node_id == node_id)
            .map(|p| p.id)
            .collect();
        for pid in &poll_ids {
            self.scheduler.remove_poll(pid);
            self.polls.remove(pid);
        }
        // Delete polls from DB *before* deleting the node (FK: polls.node_id → nodes.id).
        self.db.execute(
            "DELETE FROM polls WHERE node_id = ?1",
            rusqlite::params![node_id.to_string()],
        )?;
        delete_node(&self.db, node_id)?;
        self.nodes.remove(node_id);
        Ok(())
    }

    fn cmd_add_poll(&mut self, poll: Poll) -> Result<(), EngineError> {
        let address = self
            .nodes
            .get(&poll.node_id)
            .ok_or(EngineError::NodeNotFound(poll.node_id))?
            .addresses
            .first()
            .ok_or(EngineError::NoAddress)?
            .clone();

        insert_poll(&self.db, &poll)?;
        self.scheduler.add_poll(poll.clone(), address);
        self.polls.insert(poll.id, poll);
        Ok(())
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::{
        INDEX_POLL_RESULTS_NODE_TS, INDEX_POLL_RESULTS_TS, SCHEMA_NODES, SCHEMA_POLL_RESULTS,
        SCHEMA_POLLS,
    };
    use crate::models::poll::PollProtocol;
    use rusqlite::Connection;
    use std::time::Duration;

    fn in_memory_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(SCHEMA_NODES).unwrap();
        conn.execute_batch(SCHEMA_POLLS).unwrap();
        conn.execute_batch(SCHEMA_POLL_RESULTS).unwrap();
        conn.execute_batch(INDEX_POLL_RESULTS_NODE_TS).unwrap();
        conn.execute_batch(INDEX_POLL_RESULTS_TS).unwrap();
        conn
    }

    fn make_engine() -> Engine {
        let db = in_memory_db();
        let (engine, _handle) = Engine::new(db, 128).unwrap();
        engine
    }

    fn make_node(name: &str) -> Node {
        Node::new(name, vec!["127.0.0.1".to_string()])
    }

    fn make_poll(node_id: Uuid) -> Poll {
        Poll {
            id: Uuid::new_v4(),
            node_id,
            protocol: PollProtocol::TcpConnect { port: 80 },
            interval: Duration::from_secs(30),
            timeout: Duration::from_secs(5),
            retries: 0,
            failure_threshold: 3,
            recovery_threshold: 1,
        }
    }

    fn make_result(poll_id: Uuid, node_id: Uuid, success: bool) -> PollResult {
        if success {
            PollResult::success(poll_id, node_id, Duration::from_millis(10))
        } else {
            PollResult::failure(poll_id, node_id, "timeout")
        }
    }

    // ── handle_poll_result ───────────────────────────────────────────────────

    #[test]
    fn test_unknown_to_up_on_success() {
        let mut engine = make_engine();
        let node = make_node("host-a");
        let node_id = node.id;
        engine.cmd_add_node(node).unwrap();

        let poll = make_poll(node_id);
        let poll_id = poll.id;
        engine.cmd_add_poll(poll).unwrap();

        // One success should transition Unknown → Up.
        let result = make_result(poll_id, node_id, true);
        engine.handle_poll_result(result).unwrap();

        assert_eq!(engine.nodes[&node_id].status, NodeStatus::Up);
        assert_eq!(engine.nodes[&node_id].effective_status, EffectiveStatus::Up);
    }

    #[test]
    fn test_failure_threshold_triggers_down() {
        let mut engine = make_engine();
        let node = make_node("host-b");
        let node_id = node.id;
        engine.cmd_add_node(node).unwrap();

        let mut poll = make_poll(node_id);
        poll.failure_threshold = 2;
        let poll_id = poll.id;
        engine.cmd_add_poll(poll).unwrap();

        // Two failures → Down.
        engine.handle_poll_result(make_result(poll_id, node_id, false)).unwrap();
        assert_eq!(engine.nodes[&node_id].status, NodeStatus::Unknown);

        engine.handle_poll_result(make_result(poll_id, node_id, false)).unwrap();
        assert_eq!(engine.nodes[&node_id].status, NodeStatus::Down);
    }

    #[test]
    fn test_down_to_up_on_recovery() {
        let mut engine = make_engine();
        let node = make_node("host-c");
        let node_id = node.id;
        engine.cmd_add_node(node).unwrap();

        // Pre-set to Down.
        engine.nodes.get_mut(&node_id).unwrap().status = NodeStatus::Down;

        let mut poll = make_poll(node_id);
        poll.recovery_threshold = 2;
        let poll_id = poll.id;
        engine.cmd_add_poll(poll).unwrap();

        // First success: still Down (needs recovery_threshold=2 successes).
        engine.handle_poll_result(make_result(poll_id, node_id, true)).unwrap();
        assert_eq!(engine.nodes[&node_id].status, NodeStatus::Down);

        // Second success: Up.
        engine.handle_poll_result(make_result(poll_id, node_id, true)).unwrap();
        assert_eq!(engine.nodes[&node_id].status, NodeStatus::Up);
    }

    #[test]
    fn test_poll_result_for_unknown_node_is_ignored() {
        let mut engine = make_engine();
        let result = make_result(Uuid::new_v4(), Uuid::new_v4(), true);
        // Should not error or panic.
        engine.handle_poll_result(result).unwrap();
    }

    #[test]
    fn test_poll_result_persisted_to_db() {
        let mut engine = make_engine();
        let node = make_node("host-d");
        let node_id = node.id;
        engine.cmd_add_node(node).unwrap();

        let poll = make_poll(node_id);
        let poll_id = poll.id;
        engine.cmd_add_poll(poll).unwrap();

        engine.handle_poll_result(make_result(poll_id, node_id, true)).unwrap();

        let results =
            crate::db::repository::get_poll_results(&engine.db, &node_id, 10).unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].success);
    }

    // ── Dependency propagation ───────────────────────────────────────────────

    #[test]
    fn test_child_suppressed_when_parent_down() {
        let mut engine = make_engine();

        let parent = make_node("parent");
        let parent_id = parent.id;
        engine.cmd_add_node(parent).unwrap();

        let mut child = make_node("child");
        child.parent_node_id = Some(parent_id);
        let child_id = child.id;
        engine.cmd_add_node(child).unwrap();

        // Push parent to Down with failure_threshold=1.
        let mut poll = make_poll(parent_id);
        poll.failure_threshold = 1;
        let poll_id = poll.id;
        engine.cmd_add_poll(poll).unwrap();
        engine.handle_poll_result(make_result(poll_id, parent_id, false)).unwrap();

        assert_eq!(engine.nodes[&parent_id].status, NodeStatus::Down);
        assert_eq!(
            engine.nodes[&child_id].effective_status,
            EffectiveStatus::Suppressed
        );
    }

    // ── Command handling ─────────────────────────────────────────────────────

    #[test]
    fn test_add_and_get_node() {
        let mut engine = make_engine();
        let node = make_node("router-1");
        let node_id = node.id;

        engine.cmd_add_node(node).unwrap();
        assert!(engine.nodes.contains_key(&node_id));
        assert_eq!(engine.nodes.len(), 1);
    }

    #[test]
    fn test_remove_node_removes_polls() {
        let mut engine = make_engine();
        let node = make_node("router-2");
        let node_id = node.id;
        engine.cmd_add_node(node).unwrap();

        let poll = make_poll(node_id);
        engine.cmd_add_poll(poll).unwrap();
        assert_eq!(engine.polls.len(), 1);

        engine.cmd_remove_node(&node_id).unwrap();
        assert!(engine.nodes.is_empty());
        assert!(engine.polls.is_empty());
    }

    #[test]
    fn test_remove_nonexistent_node_errors() {
        let mut engine = make_engine();
        let result = engine.cmd_remove_node(&Uuid::new_v4());
        assert!(matches!(result, Err(EngineError::NodeNotFound(_))));
    }

    #[test]
    fn test_add_poll_fails_without_node() {
        let mut engine = make_engine();
        let poll = make_poll(Uuid::new_v4());
        let result = engine.cmd_add_poll(poll);
        assert!(matches!(result, Err(EngineError::NodeNotFound(_))));
    }

    #[test]
    fn test_get_status() {
        let mut engine = make_engine();
        let node = make_node("switch-1");
        let node_id = node.id;
        engine.cmd_add_node(node).unwrap();

        let (status, eff) = engine
            .nodes
            .get(&node_id)
            .map(|n| (n.status, n.effective_status))
            .unwrap();

        assert_eq!(status, NodeStatus::Unknown);
        assert_eq!(eff, EffectiveStatus::Unknown);
    }

    #[test]
    fn test_engine_loads_nodes_from_db() {
        // Write a node to DB before constructing the engine.
        let db = in_memory_db();
        let node = make_node("pre-existing");
        let node_id = node.id;
        insert_node(&db, &node).unwrap();

        let (engine, _) = Engine::new(db, 128).unwrap();
        assert!(engine.nodes.contains_key(&node_id));
    }

}
