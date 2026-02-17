use crate::models::status::NodeStatus;

/// Result of processing a poll result through the status engine
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusTransition {
    /// New status after applying the poll result
    pub new_status: NodeStatus,

    /// New consecutive failure count
    pub new_consecutive_failures: u32,

    /// New consecutive success count
    pub new_consecutive_successes: u32,

    /// Whether the status actually changed
    pub changed: bool,

    /// Previous status before transition
    pub previous_status: NodeStatus,
}

/// Compute the next status given current state and a new poll result.
///
/// This is a pure, deterministic function implementing the state machine from spec §4.
/// Status is reproducible from counters + thresholds only - no time windows or decay logic.
///
/// # State Machine Rules (Spec §4)
///
/// ## On Poll Success:
/// - consecutive_successes += 1
/// - consecutive_failures = 0
/// - Unknown → Up (after first success, i.e. consecutive_successes >= 1)
/// - Down → Up (if consecutive_successes >= recovery_threshold)
/// - Up → Up (no change)
///
/// ## On Poll Failure:
/// - consecutive_failures += 1
/// - consecutive_successes = 0
/// - If consecutive_failures >= failure_threshold → Down
/// - Otherwise: Unknown → Unknown, Up → Up (no change)
pub fn compute_status_transition(
    current_status: NodeStatus,
    consecutive_failures: u32,
    consecutive_successes: u32,
    failure_threshold: u32,
    recovery_threshold: u32,
    poll_success: bool,
) -> StatusTransition {
    let previous_status = current_status;

    if poll_success {
        // Poll succeeded
        let new_consecutive_successes = consecutive_successes + 1;
        let new_consecutive_failures = 0;

        let new_status = match current_status {
            NodeStatus::Unknown => {
                // Unknown → Up after first success (consecutive_successes >= 1)
                NodeStatus::Up
            }
            NodeStatus::Down => {
                // Down → Up if we've reached recovery threshold
                if new_consecutive_successes >= recovery_threshold {
                    NodeStatus::Up
                } else {
                    NodeStatus::Down
                }
            }
            NodeStatus::Up => {
                // Up → Up (no change)
                NodeStatus::Up
            }
        };

        StatusTransition {
            new_status,
            new_consecutive_failures,
            new_consecutive_successes,
            changed: new_status != previous_status,
            previous_status,
        }
    } else {
        // Poll failed
        let new_consecutive_failures = consecutive_failures + 1;
        let new_consecutive_successes = 0;

        let new_status = if new_consecutive_failures >= failure_threshold {
            // Any state → Down if we've hit the failure threshold
            NodeStatus::Down
        } else {
            // Below threshold: status doesn't change
            // Unknown → Unknown, Up → Up
            current_status
        };

        StatusTransition {
            new_status,
            new_consecutive_failures,
            new_consecutive_successes,
            changed: new_status != previous_status,
            previous_status,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unknown_to_up_on_first_success() {
        let transition = compute_status_transition(
            NodeStatus::Unknown,
            0, // consecutive_failures
            0, // consecutive_successes
            3, // failure_threshold
            1, // recovery_threshold
            true, // poll_success
        );

        assert_eq!(transition.new_status, NodeStatus::Up);
        assert_eq!(transition.new_consecutive_successes, 1);
        assert_eq!(transition.new_consecutive_failures, 0);
        assert!(transition.changed);
        assert_eq!(transition.previous_status, NodeStatus::Unknown);
    }

    #[test]
    fn test_unknown_stays_unknown_on_failure_below_threshold() {
        let transition = compute_status_transition(
            NodeStatus::Unknown,
            1, // consecutive_failures
            0, // consecutive_successes
            3, // failure_threshold
            1, // recovery_threshold
            false, // poll_success
        );

        assert_eq!(transition.new_status, NodeStatus::Unknown);
        assert_eq!(transition.new_consecutive_failures, 2);
        assert_eq!(transition.new_consecutive_successes, 0);
        assert!(!transition.changed);
    }

    #[test]
    fn test_unknown_to_down_on_failure_at_threshold() {
        let transition = compute_status_transition(
            NodeStatus::Unknown,
            2, // consecutive_failures
            0, // consecutive_successes
            3, // failure_threshold
            1, // recovery_threshold
            false, // poll_success
        );

        assert_eq!(transition.new_status, NodeStatus::Down);
        assert_eq!(transition.new_consecutive_failures, 3);
        assert_eq!(transition.new_consecutive_successes, 0);
        assert!(transition.changed);
    }

    #[test]
    fn test_up_stays_up_on_failure_below_threshold() {
        let transition = compute_status_transition(
            NodeStatus::Up,
            1, // consecutive_failures
            0, // consecutive_successes
            3, // failure_threshold
            1, // recovery_threshold
            false, // poll_success
        );

        assert_eq!(transition.new_status, NodeStatus::Up);
        assert_eq!(transition.new_consecutive_failures, 2);
        assert!(!transition.changed);
    }

    #[test]
    fn test_up_to_down_on_failure_at_threshold() {
        let transition = compute_status_transition(
            NodeStatus::Up,
            2, // consecutive_failures
            0, // consecutive_successes
            3, // failure_threshold
            1, // recovery_threshold
            false, // poll_success
        );

        assert_eq!(transition.new_status, NodeStatus::Down);
        assert_eq!(transition.new_consecutive_failures, 3);
        assert!(transition.changed);
    }

    #[test]
    fn test_down_stays_down_on_success_below_recovery() {
        let transition = compute_status_transition(
            NodeStatus::Down,
            0, // consecutive_failures
            0, // consecutive_successes
            3, // failure_threshold
            3, // recovery_threshold (need 3 successes to recover)
            true, // poll_success
        );

        assert_eq!(transition.new_status, NodeStatus::Down);
        assert_eq!(transition.new_consecutive_successes, 1);
        assert_eq!(transition.new_consecutive_failures, 0);
        assert!(!transition.changed);
    }

    #[test]
    fn test_down_to_up_on_success_at_recovery_threshold() {
        let transition = compute_status_transition(
            NodeStatus::Down,
            0, // consecutive_failures
            2, // consecutive_successes (2 previous successes)
            3, // failure_threshold
            3, // recovery_threshold
            true, // poll_success (this makes it 3 total)
        );

        assert_eq!(transition.new_status, NodeStatus::Up);
        assert_eq!(transition.new_consecutive_successes, 3);
        assert_eq!(transition.new_consecutive_failures, 0);
        assert!(transition.changed);
    }

    #[test]
    fn test_counters_reset_on_direction_change() {
        // When we get a success, failures reset
        let transition = compute_status_transition(
            NodeStatus::Up,
            2, // consecutive_failures (had some failures)
            0, // consecutive_successes
            3, // failure_threshold
            1, // recovery_threshold
            true, // poll_success
        );

        assert_eq!(transition.new_consecutive_failures, 0);
        assert_eq!(transition.new_consecutive_successes, 1);

        // When we get a failure, successes reset
        let transition = compute_status_transition(
            NodeStatus::Down,
            0, // consecutive_failures
            2, // consecutive_successes (had some successes)
            3, // failure_threshold
            3, // recovery_threshold
            false, // poll_failure
        );

        assert_eq!(transition.new_consecutive_failures, 1);
        assert_eq!(transition.new_consecutive_successes, 0);
    }

    #[test]
    fn test_failure_threshold_of_1() {
        // With threshold=1, first failure should mark Down
        let transition = compute_status_transition(
            NodeStatus::Up,
            0, // consecutive_failures
            10, // consecutive_successes (doesn't matter)
            1, // failure_threshold=1
            1, // recovery_threshold
            false, // poll_failure
        );

        assert_eq!(transition.new_status, NodeStatus::Down);
        assert_eq!(transition.new_consecutive_failures, 1);
        assert!(transition.changed);
    }

    #[test]
    fn test_recovery_threshold_of_1() {
        // With recovery_threshold=1, first success from Down should mark Up
        let transition = compute_status_transition(
            NodeStatus::Down,
            5, // consecutive_failures (doesn't matter anymore)
            0, // consecutive_successes
            3, // failure_threshold
            1, // recovery_threshold=1
            true, // poll_success
        );

        assert_eq!(transition.new_status, NodeStatus::Up);
        assert_eq!(transition.new_consecutive_successes, 1);
        assert!(transition.changed);
    }

    #[test]
    fn test_high_thresholds() {
        // Test with failure_threshold=5
        let mut status = NodeStatus::Up;
        let mut failures = 0;
        let mut successes = 0;

        // 4 failures should keep us Up
        for _ in 0..4 {
            let transition = compute_status_transition(
                status,
                failures,
                successes,
                5, // failure_threshold
                3, // recovery_threshold
                false,
            );
            status = transition.new_status;
            failures = transition.new_consecutive_failures;
            successes = transition.new_consecutive_successes;
        }
        assert_eq!(status, NodeStatus::Up);
        assert_eq!(failures, 4);

        // 5th failure should mark Down
        let transition = compute_status_transition(
            status,
            failures,
            successes,
            5,
            3,
            false,
        );
        assert_eq!(transition.new_status, NodeStatus::Down);
        assert_eq!(transition.new_consecutive_failures, 5);
        assert!(transition.changed);
    }
}
