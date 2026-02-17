use crate::net::{PollExecutor, PollOutcome};
use async_trait::async_trait;
use std::time::{Duration, Instant};
use tokio::net::TcpStream;
use tokio::time::timeout;

/// Polls a node by attempting a TCP connection to a given port.
pub struct TcpExecutor {
    pub port: u16,
}

impl TcpExecutor {
    pub fn new(port: u16) -> Self {
        Self { port }
    }
}

#[async_trait]
impl PollExecutor for TcpExecutor {
    async fn execute(&self, address: &str, timeout_duration: Duration) -> PollOutcome {
        let addr = format!("{}:{}", address, self.port);
        let start = Instant::now();

        match timeout(timeout_duration, TcpStream::connect(&addr)).await {
            Ok(Ok(_stream)) => PollOutcome {
                success: true,
                latency: Some(start.elapsed()),
                error: None,
            },
            Ok(Err(e)) => PollOutcome {
                success: false,
                latency: None,
                error: Some(e.to_string()),
            },
            Err(_) => PollOutcome {
                success: false,
                latency: None,
                error: Some(format!("TCP connect to {} timed out", addr)),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn test_tcp_connect_success() {
        // Bind a listener on a random port
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        // Accept in background so the connect doesn't hang
        tokio::spawn(async move {
            let _ = listener.accept().await;
        });

        let executor = TcpExecutor::new(port);
        let outcome = executor
            .execute("127.0.0.1", Duration::from_secs(2))
            .await;

        assert!(outcome.success);
        assert!(outcome.latency.is_some());
        assert!(outcome.error.is_none());
    }

    #[tokio::test]
    async fn test_tcp_connect_refused() {
        // Pick a port that should be unbound; bind then drop to get a free port
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener); // free the port — connect will be refused

        let executor = TcpExecutor::new(port);
        let outcome = executor
            .execute("127.0.0.1", Duration::from_secs(2))
            .await;

        assert!(!outcome.success);
        assert!(outcome.error.is_some());
    }

    #[tokio::test]
    async fn test_tcp_connect_timeout() {
        // 192.0.2.1 is TEST-NET (RFC 5737) — guaranteed non-routable, triggers timeout
        let executor = TcpExecutor::new(80);
        let outcome = executor
            .execute("192.0.2.1", Duration::from_millis(200))
            .await;

        assert!(!outcome.success);
        assert!(outcome.error.is_some());
    }
}
