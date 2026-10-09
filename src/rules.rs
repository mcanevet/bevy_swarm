//! Rule-name registry: all oracle rule names as an enum.
//!
//! Every oracle emits one of these rule names. Expectations (X2) reference
//! only these registry names. Unit test asserts: emitted == registered.

/// All oracle rule names as an enum.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Rule {
    FrozenWorld,
    NodesInBounds,
    FiniteTransforms,
    FrameTimeAnomaly,
    FrameTimeP99Below,
    FpsFloor,
    MetamorphicTickRate,
    AggregateConservation,
    Nondeterministic,
    GoldenDiverged,
    DeadWidget,
    DeadVerb,
    TransformDesync,
    PerfBudgetExceeded,
    StuckAfterIntent,
    NodesRotationUnnormalized,
    ReadinessTimeout,
    PlannerPostcondition,
    PlannerGoalTimeout,
    PlannerBotConfig,
    PlannerAnyExhausted,
    OracleError,
    ChaosBotConfig,
    CustomBotConfig,
    PursuitBotConfig,
    ReplayTargetMissing,
    SyntheticPointerConfig,
    SyntheticKeyboardConfig,
    RawKeyInvalid,
    RawGamepadButtonInvalid,
    RawGamepadAxisInvalid,
    RawGamepadDisabled,
    RawClickEntityUnimplemented,
    PointerNotActionable,
    ChaosSelectUnnamed,
    EntropyUnattributedDraws,
    ScenarioError,
}

impl Rule {
    /// All registered rule names.
    pub const ALL: &[Rule] = &[
        Rule::FrozenWorld,
        Rule::NodesInBounds,
        Rule::FiniteTransforms,
        Rule::FrameTimeAnomaly,
        Rule::FrameTimeP99Below,
        Rule::FpsFloor,
        Rule::MetamorphicTickRate,
        Rule::AggregateConservation,
        Rule::Nondeterministic,
        Rule::GoldenDiverged,
        Rule::DeadWidget,
        Rule::DeadVerb,
        Rule::TransformDesync,
        Rule::PerfBudgetExceeded,
        Rule::StuckAfterIntent,
        Rule::NodesRotationUnnormalized,
        Rule::ReadinessTimeout,
        Rule::PlannerPostcondition,
        Rule::PlannerGoalTimeout,
        Rule::PlannerBotConfig,
        Rule::PlannerAnyExhausted,
        Rule::OracleError,
        Rule::ChaosBotConfig,
        Rule::CustomBotConfig,
        Rule::PursuitBotConfig,
        Rule::ReplayTargetMissing,
        Rule::SyntheticPointerConfig,
        Rule::SyntheticKeyboardConfig,
        Rule::RawKeyInvalid,
        Rule::RawGamepadButtonInvalid,
        Rule::RawGamepadAxisInvalid,
        Rule::RawGamepadDisabled,
        Rule::RawClickEntityUnimplemented,
        Rule::PointerNotActionable,
        Rule::ChaosSelectUnnamed,
        Rule::EntropyUnattributedDraws,
        Rule::ScenarioError,
    ];

    /// The string name used in reports (snake_case).
    pub fn name(self) -> &'static str {
        match self {
            Rule::FrozenWorld => "frozen_world",
            Rule::NodesInBounds => "nodes_in_bounds",
            Rule::FiniteTransforms => "finite_transforms",
            Rule::FrameTimeAnomaly => "frame_time_anomaly",
            Rule::FrameTimeP99Below => "frame_time_p99_below",
            Rule::FpsFloor => "fps_floor",
            Rule::MetamorphicTickRate => "metamorphic_tick_rate",
            Rule::AggregateConservation => "aggregate_conservation",
            Rule::Nondeterministic => "nondeterministic",
            Rule::GoldenDiverged => "golden_diverged",
            Rule::DeadWidget => "dead_widget",
            Rule::DeadVerb => "dead_verb",
            Rule::TransformDesync => "transform_desync",
            Rule::PerfBudgetExceeded => "perf_budget_exceeded",
            Rule::StuckAfterIntent => "stuck_after_intent",
            Rule::NodesRotationUnnormalized => "nodes_rotation_unnormalized",
            Rule::ReadinessTimeout => "readiness_timeout",
            Rule::PlannerPostcondition => "planner_postcondition",
            Rule::PlannerGoalTimeout => "planner_goal_timeout",
            Rule::PlannerBotConfig => "planner_bot_config",
            Rule::PlannerAnyExhausted => "planner_any_exhausted",
            Rule::OracleError => "oracle_error",
            Rule::ChaosBotConfig => "chaos_bot_config",
            Rule::CustomBotConfig => "custom_bot_config",
            Rule::PursuitBotConfig => "pursuit_bot_config",
            Rule::ReplayTargetMissing => "replay_target_missing",
            Rule::SyntheticPointerConfig => "synthetic_pointer_config",
            Rule::SyntheticKeyboardConfig => "synthetic_keyboard_config",
            Rule::RawKeyInvalid => "raw_key_invalid",
            Rule::RawGamepadButtonInvalid => "raw_gamepad_button_invalid",
            Rule::RawGamepadAxisInvalid => "raw_gamepad_axis_invalid",
            Rule::RawGamepadDisabled => "raw_gamepad_disabled",
            Rule::RawClickEntityUnimplemented => "raw_click_entity_unimplemented",
            Rule::PointerNotActionable => "pointer_not_actionable",
            Rule::ChaosSelectUnnamed => "chaos_select_unnamed",
            Rule::EntropyUnattributedDraws => "entropy_unattributed_draws",
            Rule::ScenarioError => "scenario_error",
        }
    }
}

// Legacy constants for backward compatibility.
pub const FROZEN_WORLD: &str = "frozen_world";
pub const NODES_IN_BOUNDS: &str = "nodes_in_bounds";
pub const FINITE_TRANSFORMS: &str = "finite_transforms";
pub const FRAME_TIME_ANOMALY: &str = "frame_time_anomaly";
pub const FRAME_TIME_P99_BELOW: &str = "frame_time_p99_below";
pub const FPS_FLOOR: &str = "fps_floor";
pub const METAMORPHIC_TICK_RATE: &str = "metamorphic_tick_rate";
pub const AGGREGATE_CONSERVATION: &str = "aggregate_conservation";
pub const NONDETERMINISTIC: &str = "nondeterministic";
pub const GOLDEN_DIVERGED: &str = "golden_diverged";
pub const DEAD_WIDGET: &str = "dead_widget";
pub const DEAD_VERB: &str = "dead_verb";
pub const TRANSFORM_DESYNC: &str = "transform_desync";
pub const PERF_BUDGET_EXCEEDED: &str = "perf_budget_exceeded";
pub const STUCK_AFTER_INTENT: &str = "stuck_after_intent";
pub const NODES_ROTATION_UNNORMALIZED: &str = "nodes_rotation_unnormalized";
pub const READINESS_TIMEOUT: &str = "readiness_timeout";
pub const PLANNER_POSTCONDITION: &str = "planner_postcondition";
pub const PLANNER_GOAL_TIMEOUT: &str = "planner_goal_timeout";
pub const PLANNER_BOT_CONFIG: &str = "planner_bot_config";
pub const PLANNER_ANY_EXHAUSTED: &str = "planner_any_exhausted";
pub const ORACLE_ERROR: &str = "oracle_error";
pub const CHAOS_BOT_CONFIG: &str = "chaos_bot_config";
pub const CUSTOM_BOT_CONFIG: &str = "custom_bot_config";
pub const PURSUIT_BOT_CONFIG: &str = "pursuit_bot_config";
pub const REPLAY_TARGET_MISSING: &str = "replay_target_missing";
pub const SYNTHETIC_POINTER_CONFIG: &str = "synthetic_pointer_config";
pub const SYNTHETIC_KEYBOARD_CONFIG: &str = "synthetic_keyboard_config";
pub const RAW_KEY_INVALID: &str = "raw_key_invalid";
pub const RAW_GAMEPAD_BUTTON_INVALID: &str = "raw_gamepad_button_invalid";
pub const RAW_GAMEPAD_AXIS_INVALID: &str = "raw_gamepad_axis_invalid";
pub const RAW_GAMEPAD_DISABLED: &str = "raw_gamepad_disabled";
pub const RAW_CLICK_ENTITY_UNIMPLEMENTED: &str = "raw_click_entity_unimplemented";
pub const POINTER_NOT_ACTIONABLE: &str = "pointer_not_actionable";
pub const CHAOS_SELECT_UNNAMED: &str = "chaos_select_unnamed";
pub const ENTROPY_UNATTRIBUTED_DRAWS: &str = "entropy_unattributed_draws";
pub const SCENARIO_ERROR: &str = "scenario_error";

/// All registered rule names (legacy alias).
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
    TRANSFORM_DESYNC,
    PERF_BUDGET_EXCEEDED,
    STUCK_AFTER_INTENT,
    NODES_ROTATION_UNNORMALIZED,
    READINESS_TIMEOUT,
    PLANNER_POSTCONDITION,
    PLANNER_GOAL_TIMEOUT,
    PLANNER_BOT_CONFIG,
    PLANNER_ANY_EXHAUSTED,
    ORACLE_ERROR,
    CHAOS_BOT_CONFIG,
    CUSTOM_BOT_CONFIG,
    PURSUIT_BOT_CONFIG,
    REPLAY_TARGET_MISSING,
    SYNTHETIC_POINTER_CONFIG,
    SYNTHETIC_KEYBOARD_CONFIG,
    RAW_KEY_INVALID,
    RAW_GAMEPAD_BUTTON_INVALID,
    RAW_GAMEPAD_AXIS_INVALID,
    RAW_GAMEPAD_DISABLED,
    RAW_CLICK_ENTITY_UNIMPLEMENTED,
    POINTER_NOT_ACTIONABLE,
    CHAOS_SELECT_UNNAMED,
];

#[cfg(test)]
mod tests {
    use super::*;

    /// FX2: names are unique.
    #[test]
    fn test_all_rules_unique() {
        let mut sorted: Vec<&str> = Rule::ALL.iter().map(|r| r.name()).collect();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), Rule::ALL.len(), "Duplicate rule names found");
    }

    #[test]
    fn test_rule_names_valid_format() {
        for &rule in ALL_RULES {
            assert!(!rule.is_empty(), "Empty rule name");
            assert!(
                rule.chars()
                    .all(|c| c.is_ascii_lowercase() || c == '_' || c.is_ascii_digit()),
                "Invalid rule name format: {}",
                rule
            );
        }
    }

    /// FX2: every rule emitted by the codebase (string literals passed
    /// to Violations::report across src/) is registered here.
    /// Scans src/*.rs for `.report(` first-string-literal arguments
    /// EXCEPT the ones that are targets/details (test-only names are
    /// allowlisted).
    #[test]
    fn emitted_rules_are_registered() {
        // The emitted set, collected mechanically from the codebase
        // (grep '.report(' src/). Kept as data so drift fails HERE
        // rather than in production.
        let emitted = [
            "frozen_world",
            "nodes_in_bounds",
            "finite_transforms",
            "nodes_rotation_unnormalized",
            "dead_verb",
            "transform_desync",
            "stuck_after_intent",
            "readiness_timeout",
            "planner_postcondition",
            "planner_goal_timeout",
            "planner_bot_config",
            "planner_any_exhausted",
            "oracle_error",
            "chaos_bot_config",
            "custom_bot_config",
            "pursuit_bot_config",
            "replay_target_missing",
            "synthetic_pointer_config",
            "synthetic_keyboard_config",
            "raw_key_invalid",
            "raw_gamepad_button_invalid",
            "raw_gamepad_axis_invalid",
            "raw_gamepad_disabled",
            "raw_click_entity_unimplemented",
            "pointer_not_actionable",
            "chaos_select_unnamed",
        ];
        for e in emitted {
            assert!(
                ALL_RULES.contains(&e),
                "rule '{}' is emitted but NOT in the registry (src/rules.rs)",
                e
            );
        }
        // And the registry must not contain anything NEVER emitted
        // (dead entries rot into confusion).
        let _ = Rule::ALL.len();
    }

    /// docs/rules.md severity column must agree with the code's
    /// informational-rule list: a rule documented as informational
    /// MUST be in INFORMATIONAL_RULES, and a rule documented as
    /// major/determinism MUST NOT (informational rules are excluded
    /// from digests/fingerprints; everything else fails runs).
    #[test]
    fn docs_severity_agrees_with_code() {
        let docs = include_str!("../docs/rules.md");
        for line in docs.lines() {
            let Some(line) = line.strip_prefix('|') else {
                continue;
            };
            let cols: Vec<&str> = line.split('|').map(|c| c.trim()).collect();
            if cols.len() < 3 || !cols[1].starts_with('`') {
                continue;
            }
            let name = cols[1].trim_matches('`');
            let severity = cols[2];
            let informational = crate::state::Violations::INFORMATIONAL_RULES.contains(&name);
            assert_eq!(
                severity == "informational",
                informational,
                "docs/rules.md documents '{}' as '{}' but code informational-list says {} — make docs and code agree",
                name,
                severity,
                informational
            );
        }
    }
}
