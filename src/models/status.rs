use serde::{Deserialize, Serialize};

/// Core status of a node based on polling results.
/// Spec §4: States are Unknown, Up, Down
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeStatus {
    /// Node status is not yet determined (initial state)
    Unknown,
    /// Node is reachable and responding
    Up,
    /// Node is unreachable or not responding
    Down,
}

impl Default for NodeStatus {
    fn default() -> Self {
        NodeStatus::Unknown
    }
}

/// Effective status includes dependency propagation.
/// Spec §6: Suppressed is an overlay from dependency propagation
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EffectiveStatus {
    /// Node status is not yet determined
    Unknown,
    /// Node is up and parent (if any) is up
    Up,
    /// Node is down
    Down,
    /// Node's parent is down (status suppressed)
    Suppressed,
}

impl Default for EffectiveStatus {
    fn default() -> Self {
        EffectiveStatus::Unknown
    }
}

impl From<NodeStatus> for EffectiveStatus {
    fn from(status: NodeStatus) -> Self {
        match status {
            NodeStatus::Unknown => EffectiveStatus::Unknown,
            NodeStatus::Up => EffectiveStatus::Up,
            NodeStatus::Down => EffectiveStatus::Down,
        }
    }
}
