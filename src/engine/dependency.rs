use crate::models::{EffectiveStatus, Node, NodeStatus};
use std::collections::HashMap;
use uuid::Uuid;

/// Compute the effective status for a specific node.
///
/// Spec §6: Simple uplink dependency model
/// - If node has a parent_node_id and that parent's status == Down → Suppressed
/// - Otherwise → node's own status converted to EffectiveStatus
/// - Only checks immediate parent (no deep graph traversal)
/// - No cycle detection (config must prevent cycles)
///
/// # Arguments
/// * `node_id` - ID of the node to compute effective status for
/// * `nodes` - Map of all nodes in the system
///
/// # Returns
/// The effective status for the node, considering parent dependency
pub fn compute_effective_status(
    node_id: &Uuid,
    nodes: &HashMap<Uuid, Node>,
) -> EffectiveStatus {
    // Look up the node
    let Some(node) = nodes.get(node_id) else {
        // Node not found - return Unknown as graceful fallback
        return EffectiveStatus::Unknown;
    };

    // Check if node has a parent
    if let Some(parent_id) = &node.parent_node_id {
        // Look up the parent
        if let Some(parent) = nodes.get(parent_id) {
            // If parent is Down → this node is Suppressed
            if parent.status == NodeStatus::Down {
                return EffectiveStatus::Suppressed;
            }
            // Parent is Up or Unknown → use node's own status
        }
        // Parent not found → gracefully use node's own status
    }

    // No parent or parent is not Down → use node's own status
    node.status.into()
}

/// Recompute effective_status for all nodes.
///
/// This is used to batch-update all nodes after status changes.
/// Only considers immediate parent relationships (no deep traversal).
///
/// # Arguments
/// * `nodes` - Map of all nodes
///
/// # Returns
/// Vector of (node_id, new_effective_status) pairs for all nodes
pub fn recompute_all_effective_statuses(
    nodes: &HashMap<Uuid, Node>,
) -> Vec<(Uuid, EffectiveStatus)> {
    nodes
        .keys()
        .map(|node_id| {
            let effective = compute_effective_status(node_id, nodes);
            (*node_id, effective)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_node_with_no_parent() {
        let mut nodes = HashMap::new();
        let node = Node::new("test-node", vec!["192.168.1.1".to_string()]);
        let node_id = node.id;
        nodes.insert(node_id, node);

        // Node with no parent should have effective = own status
        let effective = compute_effective_status(&node_id, &nodes);
        assert_eq!(effective, EffectiveStatus::Unknown);
    }

    #[test]
    fn test_node_with_up_parent() {
        let mut nodes = HashMap::new();

        // Create parent node with Up status
        let mut parent = Node::new("parent", vec!["192.168.1.1".to_string()]);
        parent.status = NodeStatus::Up;
        let parent_id = parent.id;
        nodes.insert(parent_id, parent);

        // Create child node with Up status, linked to parent
        let mut child = Node::new("child", vec!["192.168.1.2".to_string()]);
        child.status = NodeStatus::Up;
        child.parent_node_id = Some(parent_id);
        let child_id = child.id;
        nodes.insert(child_id, child);

        // Child's effective status should be its own status (Up)
        let effective = compute_effective_status(&child_id, &nodes);
        assert_eq!(effective, EffectiveStatus::Up);
    }

    #[test]
    fn test_node_with_down_parent() {
        let mut nodes = HashMap::new();

        // Create parent node with Down status
        let mut parent = Node::new("parent", vec!["192.168.1.1".to_string()]);
        parent.status = NodeStatus::Down;
        let parent_id = parent.id;
        nodes.insert(parent_id, parent);

        // Create child node with Up status, linked to parent
        let mut child = Node::new("child", vec!["192.168.1.2".to_string()]);
        child.status = NodeStatus::Up;  // Child itself is Up
        child.parent_node_id = Some(parent_id);
        let child_id = child.id;
        nodes.insert(child_id, child);

        // Child's effective status should be Suppressed (parent is Down)
        let effective = compute_effective_status(&child_id, &nodes);
        assert_eq!(effective, EffectiveStatus::Suppressed);
    }

    #[test]
    fn test_node_with_unknown_parent() {
        let mut nodes = HashMap::new();

        // Create parent node with Unknown status
        let mut parent = Node::new("parent", vec!["192.168.1.1".to_string()]);
        parent.status = NodeStatus::Unknown;
        let parent_id = parent.id;
        nodes.insert(parent_id, parent);

        // Create child node with Up status, linked to parent
        let mut child = Node::new("child", vec!["192.168.1.2".to_string()]);
        child.status = NodeStatus::Up;
        child.parent_node_id = Some(parent_id);
        let child_id = child.id;
        nodes.insert(child_id, child);

        // Child's effective status should be its own status (only Down suppresses)
        let effective = compute_effective_status(&child_id, &nodes);
        assert_eq!(effective, EffectiveStatus::Up);
    }

    #[test]
    fn test_missing_parent_node() {
        let mut nodes = HashMap::new();

        // Create child node with parent_id that doesn't exist
        let mut child = Node::new("child", vec!["192.168.1.2".to_string()]);
        child.status = NodeStatus::Up;
        child.parent_node_id = Some(Uuid::new_v4());  // Non-existent parent
        let child_id = child.id;
        nodes.insert(child_id, child);

        // Should gracefully return node's own status
        let effective = compute_effective_status(&child_id, &nodes);
        assert_eq!(effective, EffectiveStatus::Up);
    }

    #[test]
    fn test_child_internal_status_unchanged() {
        let mut nodes = HashMap::new();

        // Create parent node with Down status
        let mut parent = Node::new("parent", vec!["192.168.1.1".to_string()]);
        parent.status = NodeStatus::Down;
        let parent_id = parent.id;
        nodes.insert(parent_id, parent);

        // Create child node with Up status
        let mut child = Node::new("child", vec!["192.168.1.2".to_string()]);
        child.status = NodeStatus::Up;
        child.parent_node_id = Some(parent_id);
        let child_id = child.id;
        nodes.insert(child_id, child.clone());

        // Compute effective status
        let _effective = compute_effective_status(&child_id, &nodes);

        // Verify child's internal NodeStatus is NEVER modified
        let child_after = nodes.get(&child_id).unwrap();
        assert_eq!(child_after.status, NodeStatus::Up);
    }

    #[test]
    fn test_recompute_all() {
        let mut nodes = HashMap::new();

        // Create parent (Down)
        let mut parent = Node::new("parent", vec!["192.168.1.1".to_string()]);
        parent.status = NodeStatus::Down;
        let parent_id = parent.id;
        nodes.insert(parent_id, parent);

        // Create child1 (Up, has parent)
        let mut child1 = Node::new("child1", vec!["192.168.1.2".to_string()]);
        child1.status = NodeStatus::Up;
        child1.parent_node_id = Some(parent_id);
        let child1_id = child1.id;
        nodes.insert(child1_id, child1);

        // Create independent node (Up, no parent)
        let mut independent = Node::new("independent", vec!["192.168.1.3".to_string()]);
        independent.status = NodeStatus::Up;
        let independent_id = independent.id;
        nodes.insert(independent_id, independent);

        // Recompute all
        let results = recompute_all_effective_statuses(&nodes);

        // Should return 3 results
        assert_eq!(results.len(), 3);

        // Convert to HashMap for easier checking
        let result_map: HashMap<Uuid, EffectiveStatus> = results.into_iter().collect();

        // Parent should be Down (its own status)
        assert_eq!(result_map.get(&parent_id), Some(&EffectiveStatus::Down));

        // Child1 should be Suppressed (parent is Down)
        assert_eq!(result_map.get(&child1_id), Some(&EffectiveStatus::Suppressed));

        // Independent should be Up (its own status, no parent)
        assert_eq!(result_map.get(&independent_id), Some(&EffectiveStatus::Up));
    }

    #[test]
    fn test_down_child_of_down_parent() {
        let mut nodes = HashMap::new();

        // Parent is Down
        let mut parent = Node::new("parent", vec!["192.168.1.1".to_string()]);
        parent.status = NodeStatus::Down;
        let parent_id = parent.id;
        nodes.insert(parent_id, parent);

        // Child is also Down
        let mut child = Node::new("child", vec!["192.168.1.2".to_string()]);
        child.status = NodeStatus::Down;
        child.parent_node_id = Some(parent_id);
        let child_id = child.id;
        nodes.insert(child_id, child);

        // Even though child is Down, effective should be Suppressed
        // (parent being Down takes precedence)
        let effective = compute_effective_status(&child_id, &nodes);
        assert_eq!(effective, EffectiveStatus::Suppressed);
    }
}
