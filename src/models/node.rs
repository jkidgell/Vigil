use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

use super::status::{EffectiveStatus, NodeStatus};

/// Represents a monitored network node.
/// Spec §3.1: Core domain model for a network device/service
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    /// Unique identifier for the node
    pub id: Uuid,

    /// Human-readable name
    pub name: String,

    /// IP addresses or hostnames to poll
    pub addresses: Vec<String>,

    /// Reference to polling configuration profile
    pub polling_profile: String,

    /// Current status based on poll results (spec §4)
    pub status: NodeStatus,

    /// Effective status including dependency propagation (spec §6)
    pub effective_status: EffectiveStatus,

    /// Optional parent node for dependency propagation (spec §6: single uplink)
    pub parent_node_id: Option<Uuid>,

    /// Optional arbitrary metadata
    pub metadata: Option<HashMap<String, String>>,

    /// Tags for categorization/filtering
    pub tags: Vec<String>,

    /// Counter for consecutive poll failures (spec §4)
    pub consecutive_failures: u32,

    /// Counter for consecutive poll successes (spec §4)
    pub consecutive_successes: u32,
}

impl Node {
    /// Create a new node with default values
    pub fn new(name: impl Into<String>, addresses: Vec<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            addresses,
            polling_profile: "default".to_string(),
            status: NodeStatus::Unknown,
            effective_status: EffectiveStatus::Unknown,
            parent_node_id: None,
            metadata: None,
            tags: Vec::new(),
            consecutive_failures: 0,
            consecutive_successes: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_node_new() {
        let node = Node::new("test-router", vec!["192.168.1.1".to_string()]);
        assert_eq!(node.name, "test-router");
        assert_eq!(node.addresses, vec!["192.168.1.1".to_string()]);
        assert_eq!(node.status, NodeStatus::Unknown);
        assert_eq!(node.effective_status, EffectiveStatus::Unknown);
        assert_eq!(node.consecutive_failures, 0);
        assert_eq!(node.consecutive_successes, 0);
    }
}
