pub mod icmp;
pub mod tcp;

use async_trait::async_trait;
use std::time::Duration;

/// Outcome of a single poll execution attempt.
pub struct PollOutcome {
    pub success: bool,
    pub latency: Option<Duration>,
    pub error: Option<String>,
}

/// Trait implemented by each protocol executor.
#[async_trait]
pub trait PollExecutor: Send + Sync {
    async fn execute(&self, address: &str, timeout: Duration) -> PollOutcome;
}

/// Execute a poll with up to `retries` additional attempts on failure.
/// Returns the first success, or the last failure if all attempts fail.
pub async fn execute_with_retries(
    executor: &dyn PollExecutor,
    address: &str,
    timeout: Duration,
    retries: u32,
) -> PollOutcome {
    let mut last_outcome = executor.execute(address, timeout).await;
    for _ in 0..retries {
        if last_outcome.success {
            return last_outcome;
        }
        last_outcome = executor.execute(address, timeout).await;
    }
    last_outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    struct CountingExecutor {
        fail_count: Arc<AtomicU32>,
        succeed_after: u32,
    }

    #[async_trait]
    impl PollExecutor for CountingExecutor {
        async fn execute(&self, _address: &str, _timeout: Duration) -> PollOutcome {
            let calls = self.fail_count.fetch_add(1, Ordering::SeqCst) + 1;
            if calls > self.succeed_after {
                PollOutcome { success: true, latency: Some(Duration::from_millis(1)), error: None }
            } else {
                PollOutcome { success: false, latency: None, error: Some("fail".to_string()) }
            }
        }
    }

    #[tokio::test]
    async fn test_retry_succeeds_on_second_attempt() {
        let counter = Arc::new(AtomicU32::new(0));
        let executor = CountingExecutor { fail_count: counter.clone(), succeed_after: 1 };
        let outcome = execute_with_retries(&executor, "host", Duration::from_secs(1), 2).await;
        assert!(outcome.success);
        assert_eq!(counter.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn test_retry_returns_last_failure() {
        let counter = Arc::new(AtomicU32::new(0));
        let executor = CountingExecutor { fail_count: counter.clone(), succeed_after: 99 };
        let outcome = execute_with_retries(&executor, "host", Duration::from_secs(1), 2).await;
        assert!(!outcome.success);
        // 1 initial attempt + 2 retries = 3
        assert_eq!(counter.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn test_no_retry_on_immediate_success() {
        let counter = Arc::new(AtomicU32::new(0));
        let executor = CountingExecutor { fail_count: counter.clone(), succeed_after: 0 };
        let outcome = execute_with_retries(&executor, "host", Duration::from_secs(1), 3).await;
        assert!(outcome.success);
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }
}
