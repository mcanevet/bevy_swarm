//! Rule-name registry: all oracle rule names as an enum.
//!
//! Every oracle emits one of these rule names. Expectations (X2) reference
//! only these registry names. The enum, `Rule::ALL`, and `name()` are all
//! generated from a single declarative list — there are no parallel
//! registries to drift.

/// Single source of truth: declares the `Rule` enum, the `Rule::ALL`
/// registry slice, and the snake_case `name()` mapping from one list.
macro_rules! declare_rules {
    ($( $variant:ident => $name:literal ),* $(,)?) => {
        /// All oracle rule names as an enum.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum Rule {
            $($variant),*
        }

        impl Rule {
            /// All registered rules (derived from the same list as the
            /// enum itself — cannot drift).
            pub const ALL: &[Rule] = &[$(Rule::$variant),*];

            /// The string name used in reports (snake_case).
            pub fn name(self) -> &'static str {
                match self {
                    $(Rule::$variant => $name),*
                }
            }
        }

        impl std::fmt::Display for Rule {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}", self.name())
            }
        }
    };
}

impl Rule {
    /// le3.10 (RF6): whether this rule is informational — reported
    /// but never gating the run status nor fingerprinted. Derived
    /// from the enum so the registry cannot drift from the list.
    /// Mirrors Violations::INFORMATIONAL_RULES (tested for parity).
    pub fn is_informational(self) -> bool {
        matches!(
            self,
            Rule::FrameTimeAnomaly
                | Rule::FrameTimeP99Below
                | Rule::FpsFloor
                | Rule::DeadWidget
                | Rule::PerfBudgetExceeded
                | Rule::EntropyUnattributedDraws
                | Rule::BevySuppressed
        )
    }
}

declare_rules! {
    FrozenWorld => "frozen_world",
    BevyError => "bevy_error",
    BevyWarning => "bevy_warning",
    LogError => "log_error",
    LogWarn => "log_warn",
    Panic => "panic",
    PlannerGoalUnfinished => "planner_goal_unfinished",
    BevySuppressed => "bevy_suppressed",
    NodesInBounds => "nodes_in_bounds",
    FiniteTransforms => "finite_transforms",
    FrameTimeAnomaly => "frame_time_anomaly",
    FrameTimeP99Below => "frame_time_p99_below",
    FpsFloor => "fps_floor",
    MetamorphicTickRate => "metamorphic_tick_rate",
    AggregateConservation => "aggregate_conservation",
    Nondeterministic => "nondeterministic",
    GoldenDiverged => "golden_diverged",
    DeadWidget => "dead_widget",
    DeadVerb => "dead_verb",
    TransformDesync => "transform_desync",
    PerfBudgetExceeded => "perf_budget_exceeded",
    StuckAfterIntent => "stuck_after_intent",
    PlannerSelectTargetMissing => "planner_select_target_missing",
    ResetHookPanicked => "reset_hook_panicked",
    CheatHookPanicked => "cheat_hook_panicked",
    NodesRotationUnnormalized => "nodes_rotation_unnormalized",
    ReadinessTimeout => "readiness_timeout",
    PlannerPostcondition => "planner_postcondition",
    PlannerGoalTimeout => "planner_goal_timeout",
    PlannerBotConfig => "planner_bot_config",
    PlannerAnyExhausted => "planner_any_exhausted",
    OracleError => "oracle_error",
    ChaosBotConfig => "chaos_bot_config",
    CustomBotConfig => "custom_bot_config",
    PursuitBotConfig => "pursuit_bot_config",
    ReplayTargetMissing => "replay_target_missing",
    SyntheticPointerConfig => "synthetic_pointer_config",
    SyntheticKeyboardConfig => "synthetic_keyboard_config",
    RawKeyInvalid => "raw_key_invalid",
    RawGamepadButtonInvalid => "raw_gamepad_button_invalid",
    RawGamepadAxisInvalid => "raw_gamepad_axis_invalid",
    RawGamepadDisabled => "raw_gamepad_disabled",
    RawClickTargetOffscreen => "raw_click_target_offscreen",
    PointerNotActionable => "pointer_not_actionable",
    ChaosSelectUnnamed => "chaos_select_unnamed",
    EntropyUnattributedDraws => "entropy_unattributed_draws",
    ScenarioError => "scenario_error",
}

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
        for &rule in Rule::ALL {
            let name = rule.name();
            assert!(!name.is_empty(), "Empty rule name");
            assert!(
                name.chars()
                    .all(|c| c.is_ascii_lowercase() || c == '_' || c.is_ascii_digit()),
                "Invalid rule name format: {}",
                name
            );
        }
    }

    /// le3.10 (RF6): Rule::is_informational() parity with
    /// Violations::INFORMATIONAL_RULES — the string list and the enum
    /// method must agree exactly (no drift).
    #[test]
    fn test_informational_parity() {
        for &rule in Rule::ALL {
            let name = rule.name();
            let in_list = crate::state::Violations::INFORMATIONAL_RULES
                .iter()
                .any(|r| r.name() == name);
            let via_enum = rule.is_informational();
            assert_eq!(
                in_list, via_enum,
                "informational drift for {name}: list={in_list} enum={via_enum}"
            );
        }
        for rule in crate::state::Violations::INFORMATIONAL_RULES {
            assert!(
                Rule::ALL.contains(rule),
                "INFORMATIONAL_RULES lists orphan rule '{}' (never emitted)",
                rule.name()
            );
        }
    }

    /// le3.14 (RF18): docs/rules.md must document EVERY registered
    /// rule — no missing, no orphan rows.
    #[test]
    fn docs_list_all_rules() {
        let docs = include_str!("../docs/rules.md");
        for &rule in Rule::ALL {
            let row = format!("| `{}` |", rule.name());
            assert!(
                docs.contains(&row),
                "docs/rules.md is missing a row for rule '{}'",
                rule.name()
            );
        }
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
            let informational = crate::state::Violations::INFORMATIONAL_RULES
                .iter()
                .any(|r| r.name() == name);
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
