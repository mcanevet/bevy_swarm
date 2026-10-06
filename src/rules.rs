//! Rule-name registry: all oracle rule names as constants.
//!
//! Every oracle emits one of these rule names. Expectations (X2) reference
//! only these registry names.

/// Frozen-world liveness oracle
pub const FROZEN_WORLD: &str = "frozen_world";

/// Nodes-in-bounds oracle
pub const NODES_IN_BOUNDS: &str = "nodes_in_bounds";

/// Finite-transforms oracle
pub const FINITE_TRANSFORMS: &str = "finite_transforms";

/// Frame-time anomaly oracle
pub const FRAME_TIME_ANOMALY: &str = "frame_time_anomaly";

/// Frame-time p99 threshold
pub const FRAME_TIME_P99_BELOW: &str = "frame_time_p99_below";

/// FPS floor
pub const FPS_FLOOR: &str = "fps_floor";

/// Metamorphic tick-rate rule
pub const METAMORPHIC_TICK_RATE: &str = "metamorphic_tick_rate";

/// Aggregate conservation rule
pub const AGGREGATE_CONSERVATION: &str = "aggregate_conservation";

/// Nondeterministic behavior detected
pub const NONDETERMINISTIC: &str = "nondeterministic";

/// Golden test diverged
pub const GOLDEN_DIVERGED: &str = "golden_diverged";

/// Dead widget detected
pub const DEAD_WIDGET: &str = "dead_widget";

/// Dead verb detected
pub const DEAD_VERB: &str = "dead_verb";

/// Performance budget exceeded (generic)
pub const PERF_BUDGET_EXCEEDED: &str = "perf_budget_exceeded";

/// All registered rule names
pub const ALL_RULES: &[&str] = &[
    FROZEN_WORLD,
    NODES_IN_BOUNDS,
    FINITE_TRANSFORMS,
    FRAME_TIME_ANOMALY,
    FRAME_TIME_P99_BELOW,
    FPS_FLOOR,
    METAMORPHIC_TICK_RATE,
    AGGREGATE_CONSERVATION,
    NONDETERMINISTIC,
    GOLDEN_DIVERGED,
    DEAD_WIDGET,
    DEAD_VERB,
    PERF_BUDGET_EXCEEDED,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_all_rules_unique() {
        let mut sorted = ALL_RULES.to_vec();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), ALL_RULES.len(), "Duplicate rule names found");
    }

    #[test]
    fn test_rule_names_valid_format() {
        for &rule in ALL_RULES {
            assert!(!rule.is_empty(), "Empty rule name");
            assert!(
                rule.chars().all(|c| c.is_alphanumeric() || c == '_'),
                "Invalid rule name format: {}",
                rule
            );
        }
    }
}
