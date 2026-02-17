use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use uuid::Uuid;

/// Protocol type for polling
/// Spec §3.2: ICMP ping or TCP connect check
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum PollProtocol {
    /// ICMP echo request (ping)
    Icmp,
    /// TCP connection attempt
    TcpConnect { port: u16 },
}

/// Polling configuration for a node
/// Spec §3.2: Defines how and when to poll a node
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Poll {
    /// Unique identifier for this poll configuration
    pub id: Uuid,

    /// Node this poll applies to
    pub node_id: Uuid,

    /// Protocol to use for polling
    pub protocol: PollProtocol,

    /// How often to poll (spec §5: scheduler interval)
    #[serde(with = "duration_serde")]
    pub interval: Duration,

    /// Maximum time to wait for response
    #[serde(with = "duration_serde")]
    pub timeout: Duration,

    /// Number of retry attempts on failure
    pub retries: u32,

    /// Number of consecutive failures before marking Down (spec §4)
    pub failure_threshold: u32,

    /// Number of consecutive successes before marking Up from Down (spec §4)
    pub recovery_threshold: u32,
}

impl Poll {
    /// Create a new poll configuration
    pub fn new(
        node_id: Uuid,
        protocol: PollProtocol,
        interval: Duration,
        timeout: Duration,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            node_id,
            protocol,
            interval,
            timeout,
            retries: 0,
            failure_threshold: 3,
            recovery_threshold: 1,
        }
    }
}

/// Immutable result of a poll execution
/// Spec §3.3: Timestamped, append-only poll result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PollResult {
    /// Unique identifier for this result
    pub id: Uuid,

    /// Poll configuration that generated this result
    pub poll_id: Uuid,

    /// Node that was polled
    pub node_id: Uuid,

    /// When the poll was executed
    pub timestamp: DateTime<Utc>,

    /// Whether the poll succeeded
    pub success: bool,

    /// Response latency if successful
    #[serde(
        with = "option_duration_serde",
        skip_serializing_if = "Option::is_none"
    )]
    pub latency: Option<Duration>,

    /// Error message if failed
    pub error: Option<String>,
}

impl PollResult {
    /// Create a new successful poll result
    pub fn success(poll_id: Uuid, node_id: Uuid, latency: Duration) -> Self {
        Self {
            id: Uuid::new_v4(),
            poll_id,
            node_id,
            timestamp: Utc::now(),
            success: true,
            latency: Some(latency),
            error: None,
        }
    }

    /// Create a new failed poll result
    pub fn failure(poll_id: Uuid, node_id: Uuid, error: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            poll_id,
            node_id,
            timestamp: Utc::now(),
            success: false,
            latency: None,
            error: Some(error.into()),
        }
    }
}

// Custom serde for Duration (serialize as seconds)
mod duration_serde {
    use serde::{Deserialize, Deserializer, Serializer};
    use std::time::Duration;

    pub fn serialize<S>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_u64(duration.as_secs())
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Duration, D::Error>
    where
        D: Deserializer<'de>,
    {
        let secs = u64::deserialize(deserializer)?;
        Ok(Duration::from_secs(secs))
    }
}

mod option_duration_serde {
    use serde::{Deserialize, Deserializer, Serializer};
    use std::time::Duration;

    pub fn serialize<S>(duration: &Option<Duration>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match duration {
            Some(d) => serializer.serialize_some(&d.as_secs_f64()),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<Duration>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let opt: Option<f64> = Option::deserialize(deserializer)?;
        Ok(opt.map(Duration::from_secs_f64))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_poll_new() {
        let node_id = Uuid::new_v4();
        let poll = Poll::new(
            node_id,
            PollProtocol::Icmp,
            Duration::from_secs(30),
            Duration::from_secs(5),
        );
        assert_eq!(poll.node_id, node_id);
        assert_eq!(poll.protocol, PollProtocol::Icmp);
        assert_eq!(poll.interval, Duration::from_secs(30));
        assert_eq!(poll.timeout, Duration::from_secs(5));
        assert_eq!(poll.failure_threshold, 3);
        assert_eq!(poll.recovery_threshold, 1);
    }

    #[test]
    fn test_poll_result_success() {
        let poll_id = Uuid::new_v4();
        let node_id = Uuid::new_v4();
        let result = PollResult::success(poll_id, node_id, Duration::from_millis(25));
        assert_eq!(result.poll_id, poll_id);
        assert_eq!(result.node_id, node_id);
        assert!(result.success);
        assert_eq!(result.latency, Some(Duration::from_millis(25)));
        assert!(result.error.is_none());
    }

    #[test]
    fn test_poll_result_failure() {
        let poll_id = Uuid::new_v4();
        let node_id = Uuid::new_v4();
        let result = PollResult::failure(poll_id, node_id, "Connection timeout");
        assert_eq!(result.poll_id, poll_id);
        assert_eq!(result.node_id, node_id);
        assert!(!result.success);
        assert!(result.latency.is_none());
        assert_eq!(result.error, Some("Connection timeout".to_string()));
    }
}
