use crate::models::poll::{Poll, PollProtocol, PollResult};
use crate::net::{execute_with_retries, PollExecutor};
use crate::net::icmp::IcmpExecutor;
use crate::net::tcp::TcpExecutor;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use uuid::Uuid;

/// State tracked per scheduled poll.
pub struct ScheduledPoll {
    pub poll: Poll,
    /// The address (IP or hostname) to poll.  Stored here because the
    /// address lives on `Node`, not `Poll`.
    pub address: String,
    pub next_run_at: Instant,
    pub running: bool,
}

/// Statistics snapshot from the scheduler.
#[derive(Debug, Clone)]
pub struct SchedulerStats {
    pub total_polls: usize,
    pub active_count: usize,
    pub overdue_polls: usize,
}

/// Message sent by a completed poll task back to the scheduler.
struct PollDone {
    poll_id: Uuid,
}

/// Async scheduler — spec §5.
///
/// The scheduler owns a map of `ScheduledPoll` entries.  On each `tick()` call
/// it fires any polls whose `next_run_at` has passed, respects the
/// concurrency cap, and advances `next_run_at` by the fixed interval
/// (drift-prevention rule from spec §5).
///
/// Completed poll tasks signal back via an internal `done_tx` channel.
/// `tick()` drains that channel first so `running` flags are cleared before
/// deciding which polls to launch.
pub struct Scheduler {
    polls: HashMap<Uuid, ScheduledPoll>,
    max_concurrent: usize,
    active_count: Arc<AtomicUsize>,
    result_tx: mpsc::Sender<PollResult>,
    done_tx: mpsc::Sender<PollDone>,
    done_rx: mpsc::Receiver<PollDone>,
}

impl Scheduler {
    /// Create a new scheduler.
    ///
    /// `result_tx` — channel through which completed `PollResult`s are forwarded
    ///               to the engine main loop.
    /// `max_concurrent` — maximum polls that may run simultaneously (spec §5
    ///                    recommends 128).
    pub fn new(result_tx: mpsc::Sender<PollResult>, max_concurrent: usize) -> Self {
        let (done_tx, done_rx) = mpsc::channel(max_concurrent.max(1) * 2);
        Self {
            polls: HashMap::new(),
            max_concurrent,
            active_count: Arc::new(AtomicUsize::new(0)),
            result_tx,
            done_tx,
            done_rx,
        }
    }

    // ── Management ────────────────────────────────────────────────────────────

    /// Register a new poll. Schedules it to run immediately on the first tick.
    pub fn add_poll(&mut self, poll: Poll, address: String) {
        let scheduled = ScheduledPoll {
            next_run_at: Instant::now(),
            running: false,
            address,
            poll,
        };
        self.polls.insert(scheduled.poll.id, scheduled);
    }

    /// Unregister a poll by ID.
    pub fn remove_poll(&mut self, poll_id: &Uuid) {
        self.polls.remove(poll_id);
    }

    /// Update an existing poll's configuration (interval, timeout, etc.).
    /// Resets `next_run_at` to now so the new settings take effect promptly.
    pub fn update_poll(&mut self, poll: Poll, address: String) {
        if let Some(entry) = self.polls.get_mut(&poll.id) {
            entry.poll = poll;
            entry.address = address;
            entry.next_run_at = Instant::now();
            entry.running = false;
        }
    }

    /// Return a statistics snapshot.
    pub fn get_stats(&self) -> SchedulerStats {
        let now = Instant::now();
        let active_count = self.active_count.load(Ordering::SeqCst);
        let overdue_polls = self
            .polls
            .values()
            .filter(|sp| now >= sp.next_run_at && !sp.running)
            .count();
        SchedulerStats {
            total_polls: self.polls.len(),
            active_count,
            overdue_polls,
        }
    }

    // ── Tick ──────────────────────────────────────────────────────────────────

    /// Drive the scheduler forward one step.
    ///
    /// 1. Drain completion notifications so `running` flags are current.
    /// 2. For each poll whose `next_run_at <= now`:
    ///    - Advance `next_run_at += interval` (drift-prevention).
    ///    - If `running`: skip execution (overlap prevention).
    ///    - If `active_count >= max_concurrent`: skip (backpressure).
    ///    - Otherwise: mark `running = true`, spawn task, increment counter.
    pub async fn tick(&mut self) {
        // Step 1: drain completion channel — must happen before we inspect
        // `running` flags to avoid re-launching polls that just finished.
        self.drain_done();

        let now = Instant::now();

        // Collect IDs that are due to avoid holding a mutable borrow while
        // spawning tasks.
        let due_ids: Vec<Uuid> = self
            .polls
            .iter()
            .filter(|(_, sp)| now >= sp.next_run_at)
            .map(|(id, _)| *id)
            .collect();

        for id in due_ids {
            let sp = match self.polls.get_mut(&id) {
                Some(sp) => sp,
                None => continue,
            };

            // Always advance next_run_at by interval (spec §5 drift-prevention).
            sp.next_run_at += sp.poll.interval;

            if sp.running {
                // Overlap prevention: poll already in flight, skip this tick.
                continue;
            }

            if self.active_count.load(Ordering::SeqCst) >= self.max_concurrent {
                // Concurrency cap: backpressure, skip this poll this tick.
                continue;
            }

            // Launch the poll task.
            sp.running = true;
            self.active_count.fetch_add(1, Ordering::SeqCst);

            let poll = sp.poll.clone();
            let address = sp.address.clone();
            let result_tx = self.result_tx.clone();
            let done_tx = self.done_tx.clone();
            let active_count = Arc::clone(&self.active_count);

            tokio::spawn(async move {
                let result = run_poll(&poll, &address).await;

                // Send result to engine; ignore send error (engine may have shut down).
                let _ = result_tx.send(result).await;

                // Signal scheduler that this poll slot is free.
                active_count.fetch_sub(1, Ordering::SeqCst);
                let _ = done_tx.send(PollDone { poll_id: poll.id }).await;
            });
        }
    }

    /// Drain all pending completion messages and clear their `running` flags.
    fn drain_done(&mut self) {
        while let Ok(done) = self.done_rx.try_recv() {
            if let Some(sp) = self.polls.get_mut(&done.poll_id) {
                sp.running = false;
            }
        }
    }
}

// ── Poll execution ────────────────────────────────────────────────────────────

/// Execute a single poll and return a `PollResult`.
async fn run_poll(poll: &Poll, address: &str) -> PollResult {
    let executor: Box<dyn PollExecutor> = match &poll.protocol {
        PollProtocol::Icmp => Box::new(IcmpExecutor::new()),
        PollProtocol::TcpConnect { port } => Box::new(TcpExecutor::new(*port)),
    };

    let outcome =
        execute_with_retries(executor.as_ref(), address, poll.timeout, poll.retries).await;

    if outcome.success {
        PollResult::success(
            poll.id,
            poll.node_id,
            outcome.latency.unwrap_or(Duration::ZERO),
        )
    } else {
        PollResult::failure(
            poll.id,
            poll.node_id,
            outcome.error.unwrap_or_else(|| "unknown error".to_string()),
        )
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::poll::PollProtocol;
    use std::time::Duration;
    use tokio::sync::mpsc;

    fn make_poll(interval: Duration) -> Poll {
        Poll {
            id: Uuid::new_v4(),
            node_id: Uuid::new_v4(),
            protocol: PollProtocol::TcpConnect { port: 80 },
            interval,
            timeout: Duration::from_millis(100),
            retries: 0,
            failure_threshold: 3,
            recovery_threshold: 1,
        }
    }

    #[tokio::test]
    async fn test_add_remove_poll() {
        let (tx, _rx) = mpsc::channel(16);
        let mut sched = Scheduler::new(tx, 128);

        let poll = make_poll(Duration::from_secs(30));
        let id = poll.id;
        sched.add_poll(poll, "127.0.0.1".to_string());
        assert_eq!(sched.polls.len(), 1);

        sched.remove_poll(&id);
        assert_eq!(sched.polls.len(), 0);
    }

    #[tokio::test]
    async fn test_update_poll() {
        let (tx, _rx) = mpsc::channel(16);
        let mut sched = Scheduler::new(tx, 128);

        let poll = make_poll(Duration::from_secs(30));
        let id = poll.id;
        sched.add_poll(poll, "127.0.0.1".to_string());

        let mut updated = make_poll(Duration::from_secs(60));
        updated.id = id;
        sched.update_poll(updated, "10.0.0.1".to_string());

        assert_eq!(sched.polls[&id].poll.interval, Duration::from_secs(60));
        assert_eq!(sched.polls[&id].address, "10.0.0.1");
    }

    #[tokio::test]
    async fn test_drift_prevention() {
        // next_run_at should advance by `interval` from the previous next_run_at,
        // not from `now`.  We push next_run_at into the past to simulate a late
        // tick, then verify it advanced by exactly one interval.
        let (tx, _rx) = mpsc::channel(16);
        let mut sched = Scheduler::new(tx, 0); // cap=0 → no tasks spawn

        let interval = Duration::from_millis(100);
        let poll = make_poll(interval);
        let id = poll.id;
        sched.add_poll(poll, "127.0.0.1".to_string());

        // Push next_run_at 300 ms into the past (3 intervals overdue).
        let past = Instant::now() - Duration::from_millis(300);
        sched.polls.get_mut(&id).unwrap().next_run_at = past;

        let before_tick = sched.polls[&id].next_run_at;
        sched.tick().await;
        let after_tick = sched.polls[&id].next_run_at;

        // next_run_at must advance by exactly one interval from the old value.
        let advanced = after_tick.duration_since(before_tick);
        assert_eq!(
            advanced, interval,
            "next_run_at must advance by interval, not by now+interval"
        );
    }

    #[tokio::test]
    async fn test_concurrency_cap_zero() {
        // With max_concurrent=0 nothing should ever be spawned.
        let (tx, _rx) = mpsc::channel(16);
        let mut sched = Scheduler::new(tx, 0);

        let poll = make_poll(Duration::from_millis(10));
        let id = poll.id;
        sched.add_poll(poll, "127.0.0.1".to_string());

        // Mark it due.
        sched.polls.get_mut(&id).unwrap().next_run_at =
            Instant::now() - Duration::from_millis(50);

        sched.tick().await;

        // The poll should NOT be marked running (cap blocked it).
        assert!(!sched.polls[&id].running);
        assert_eq!(sched.active_count.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn test_no_overlap() {
        // If a poll is already running, a second tick must not spawn another task.
        let (tx, _rx) = mpsc::channel(16);
        let mut sched = Scheduler::new(tx, 128);

        let poll = make_poll(Duration::from_millis(10));
        let id = poll.id;
        sched.add_poll(poll, "127.0.0.1".to_string());

        // Mark the poll as already running and set it due.
        sched.polls.get_mut(&id).unwrap().running = true;
        sched.active_count.fetch_add(1, Ordering::SeqCst);
        sched.polls.get_mut(&id).unwrap().next_run_at =
            Instant::now() - Duration::from_millis(50);

        sched.tick().await;

        // active_count must still be 1 (no second task spawned).
        assert_eq!(sched.active_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_get_stats() {
        let (tx, _rx) = mpsc::channel(16);
        let mut sched = Scheduler::new(tx, 128);

        let poll = make_poll(Duration::from_secs(60));
        sched.add_poll(poll, "127.0.0.1".to_string());

        let stats = sched.get_stats();
        assert_eq!(stats.total_polls, 1);
        assert_eq!(stats.active_count, 0);
        // next_run_at is set to Instant::now() at add time, so it may be
        // considered overdue by a tiny margin — 0 or 1 is acceptable.
        assert!(stats.overdue_polls <= 1);
    }

    #[tokio::test]
    async fn test_result_sent_on_completion() {
        // A TCP poll that connects to a real listener should send a PollResult.
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let _ = listener.accept().await;
        });

        let (tx, mut rx) = mpsc::channel(16);
        let mut sched = Scheduler::new(tx, 128);

        let mut poll = make_poll(Duration::from_secs(60));
        poll.protocol = PollProtocol::TcpConnect { port };
        let id = poll.id;
        sched.add_poll(poll, "127.0.0.1".to_string());

        // Make it due immediately.
        sched.polls.get_mut(&id).unwrap().next_run_at =
            Instant::now() - Duration::from_millis(10);

        sched.tick().await;

        // Wait for the result (up to 2 s).
        let result = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("timed out waiting for poll result")
            .expect("channel closed");

        assert_eq!(result.poll_id, id);
        assert!(result.success);
    }
}
