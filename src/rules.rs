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
    /// One-line meaning (what invariant of a healthy game this rule protects).
    pub fn meaning(self) -> &'static str {
        match self {
            Rule::FrozenWorld => "Entities must keep changing: a world that stops updating while the game is running is a soft-lock.",
            Rule::UndrivableGame => "The bot's actions had no observable effect; the game may not be drivable by the configured input.",
            Rule::HarnessCouldNotDriveGame => "The harness could not drive the game: enough actions were injected but none had any effect.",
            Rule::BevyError => "Bevy reported an error (bevy_log/LogPlugin error stream).",
            Rule::BevyWarning => "Bevy reported a warning (bevy_log/LogPlugin warn stream); \
informational by default, escalate with scenario strict_warnings.",
            Rule::LogError => "Game code logged at error level (tracing subscriber).",
            Rule::LogWarn => "Game code logged at warn level (tracing subscriber); \
informational by default, escalate with scenario strict_warnings.",
            Rule::Panic => "A system panicked during the run (caught by the panic hook).",
            Rule::PlannerGoalUnfinished => "A planner goal tree was abandoned without achieving its goal.",
            Rule::BevySuppressed => "A known-benign Bevy warning was suppressed by the harness's filter list.",
            Rule::NodesInBounds => "Gameplay entity positions stay inside declared play-space bounds.",
            Rule::FiniteTransforms => "Every entity's Transform holds finite values (no NaN/inf).",
            Rule::FrameTimeAnomaly => "Frame time deviated abnormally from the run's Welford mean (smoothed, transient spikes ignored).",
            Rule::FrameTimeP99Below => "Rolling per-frame p99 stayed below a configured floor — suspiciously fast, likely a frozen loop.",
            Rule::FpsFloor => "Measured FPS fell below a configured floor.",
            Rule::MetamorphicTickRate => "Behavior differs between tick rates — a determinism/timing bug surfaced by metamorphic comparison.",
            Rule::AggregateConservation => "A declared conserved aggregate (total HP, currency, mass) changed outside allowed flows.",
            Rule::Nondeterministic => "Same seed produced different per-frame state digests across repeated runs.",
            Rule::GoldenDiverged => "The run's state trajectory diverged from the recorded golden trajectory beyond tolerance.",
            Rule::DeadWidget => "A UI widget never reacted to any interaction during the whole run.",
            Rule::DeadVerb => "A declared IntentSurface verb was never observed to trigger any game effect.",
            Rule::UnusedInput => "A raw input in the declared surface was never observed to change any percept.",
            Rule::TransformDesync => "Transform and GlobalTransform disagreed beyond a tolerance (missing transform propagation).",
            Rule::PerfBudgetExceeded => "A performance budget (frame time, memory) exceeded its baseline.",
            Rule::StuckAfterIntent => "After an intent fired, no liveness signal followed (soft-lock with input).",
            Rule::PlannerSelectTargetMissing => "A planner 'select' referenced a target Name/StableId not found in the world.",
            Rule::ResetHookPanicked => "The game's ResetHooks handler panicked.",
            Rule::CheatHookPanicked => "The game's CheatHooks handler panicked.",
            Rule::NodesRotationUnnormalized => "A rotation quaternion drifted from unit length (accumulated float error).",
            Rule::ReadinessTimeout => "The game never reached a ready state within the configured startup budget.",
            Rule::PlannerPostcondition => "A planner goal marked achieved failed its postcondition shortly after.",
            Rule::PlannerGoalTimeout => "A planner goal exceeded its max_s deadline without achieving.",
            Rule::PlannerBotConfig => "The planner bot's scenario configuration is invalid or incomplete.",
            Rule::PlannerAnyExhausted => "A planner Any-combinator exhausted all children without success.",
            Rule::OracleError => "An oracle itself errored while evaluating (infrastructure failure, not a game bug).",
            Rule::ChaosBotConfig => "The chaos bot's scenario configuration is invalid or incomplete.",
            Rule::CustomBotConfig => "A custom named bot policy's configuration is invalid or unknown.",
            Rule::PursuitBotConfig => "The pursuit bot's scenario configuration is invalid or incomplete.",
            Rule::ReplayTargetMissing => "A replay step referenced a target entity that no longer exists.",
            Rule::SyntheticPointerConfig => "The synthetic_pointer bot's configuration is invalid (e.g. missing picking backend).",
            Rule::SyntheticKeyboardConfig => "The synthetic_keyboard bot's configuration is invalid.",
            Rule::RawKeyInvalid => "A raw-input key name did not parse to a KeyCode.",
            Rule::RawGamepadButtonInvalid => "A raw-input gamepad button name is unknown.",
            Rule::RawGamepadAxisInvalid => "A raw-input gamepad axis name is unknown.",
            Rule::RawGamepadDisabled => "Raw gamepad input was requested but the gamepad feature is off.",
            Rule::RawClickTargetOffscreen => "A raw click targeted a position outside the camera viewport.",
            Rule::PointerNotActionable => "A synthetic click completed but the picking map showed nothing receivable (masked/dead backend).",
            Rule::ChaosSelectUnnamed => "The chaos bot tried to select among entities but they carry no Name.",
            Rule::EntropyUnattributedDraws => "getrandom draws occurred outside a run's attribution window.",
            Rule::ScenarioError => "The scenario itself failed (rejected config, unloadable state) rather than the game misbehaving.",
        }
    }

    /// What concretely triggers this rule.
    pub fn detects(self) -> &'static str {
        match self {
            Rule::FrozenWorld => "Per-frame gameplay-entity digest unchanged for N consecutive frames while not paused.",
            Rule::UndrivableGame => "Post-run analysis: zero percept changes across the entire action log.",
            Rule::HarnessCouldNotDriveGame => "End-of-run vacuity check: >= 20 audited actions and an overall effect rate of exactly 0.",
            Rule::BevyError => "ERROR-level record routed through the injected Bevy log subscriber.",
            Rule::BevyWarning => "WARN-level record routed through the injected Bevy log subscriber.",
            Rule::LogError => "ERROR-level tracing event captured by the harness sink.",
            Rule::LogWarn => "WARN-level tracing event captured by the harness sink.",
            Rule::Panic => "std::panic hook fires during app.update() or setup.",
            Rule::PlannerGoalUnfinished => "Planner exhausts the tree with goals still unachieved at duration end.",
            Rule::BevySuppressed => "A log record matching the known-benign suppression list.",
            Rule::NodesInBounds => "Any Gameplay entity leaves [min,max] per axis of the nodes_in_bounds invariant.",
            Rule::FiniteTransforms => "Any Translation/Rotation/Scale component is NaN or infinite.",
            Rule::FrameTimeAnomaly => "Frame time exceeds the Welford running mean by k sigma (transient tolerated).",
            Rule::FrameTimeP99Below => "Rolling p99 frame time < configured floor for the window.",
            Rule::FpsFloor => "Smoothed FPS < configured floor.",
            Rule::MetamorphicTickRate => "Digest mismatch between the base run and a re-run at a different tps.",
            Rule::AggregateConservation => "Sum/derivative of the tracked aggregate violates the declared conservation law.",
            Rule::Nondeterministic => "Two runs with identical seed produce different digests at some frame.",
            Rule::GoldenDiverged => "State digest at frame k differs from the golden file beyond tolerance.",
            Rule::DeadWidget => "A picked UI node receives no effective interaction over the whole run.",
            Rule::DeadVerb => "No percept delta is attributable to a verb despite it being emitted.",
            Rule::UnusedInput => "Differential probing shows zero effect breadth for a raw input.",
            Rule::TransformDesync => "GlobalTransform recomputation disagrees with stored Transform beyond epsilon.",
            Rule::PerfBudgetExceeded => "Measured metric above its recorded baseline budget.",
            Rule::StuckAfterIntent => "No liveness signal within the timeout following a fired intent.",
            Rule::PlannerSelectTargetMissing => "Select resolution finds no Name/StableId match at fire time.",
            Rule::ResetHookPanicked => "Panic caught while invoking ResetHooks.",
            Rule::CheatHookPanicked => "Panic caught while invoking CheatHooks.",
            Rule::NodesRotationUnnormalized => "|quat.length() - 1| > epsilon on a Gameplay entity.",
            Rule::ReadinessTimeout => "Readiness predicate still false after readiness budget elapses.",
            Rule::PlannerPostcondition => "Postcondition predicate fails within the stability window after achievement.",
            Rule::PlannerGoalTimeout => "max_s elapses on a Seq/Repeat/Any node without child achievement.",
            Rule::PlannerBotConfig => "Planner bot loaded with missing goals, bad paths, or unknown predicates.",
            Rule::PlannerAnyExhausted => "All Any children timed out or failed.",
            Rule::OracleError => "Oracle system returned Err / panicked internally.",
            Rule::ChaosBotConfig => "Chaos bot with an empty surface, missing seed, or unsupported combination.",
            Rule::CustomBotConfig => "Named policy not registered via add_bot_policy.",
            Rule::PursuitBotConfig => "Pursuit bot with unresolved agent/target names.",
            Rule::ReplayTargetMissing => "Replay step target Name/StableId absent from live world.",
            Rule::SyntheticPointerConfig => "synthetic_pointer without a picking backend or camera.",
            Rule::SyntheticKeyboardConfig => "synthetic_keyboard with unparseable key names.",
            Rule::RawKeyInvalid => "parse_key_code returns None for a RawAction::Key.",
            Rule::RawGamepadButtonInvalid => "Gamepad button string not a known Gilrs button.",
            Rule::RawGamepadAxisInvalid => "Gamepad axis string not a known axis.",
            Rule::RawGamepadDisabled => "RawAction::Gamepad* while the gamepad feature is disabled.",
            Rule::RawClickTargetOffscreen => "Click pos fails inside the computed camera viewport rect.",
            Rule::PointerNotActionable => "require_actionable set and picking hover map empty after the gesture.",
            Rule::ChaosSelectUnnamed => "Chaos Select drawn while candidate entities lack Name components.",
            Rule::EntropyUnattributedDraws => "getrandom call outside an attributed run window.",
            Rule::ScenarioError => "Load-time or runtime ScenarioError paths.",
        }
    }

    /// Known false-positive modes (why a healthy game might trip this rule).
    pub fn false_positive_modes(self) -> &'static str {
        match self {
            Rule::FrozenWorld => "Legitimately static scenes (menus, cutscenes) — scope with the Gameplay marker or pause hooks.",
            Rule::UndrivableGame => "Games whose state only changes via time-based schedules with no input dependence.",
            Rule::HarnessCouldNotDriveGame => "Menus needing clicks the monkey cannot find; input gated behind state; broken headless build.",
            Rule::BevyError => "Upstream Bevy errors that are benign in headless mode (asset loader warnings, GPU absence).",
            Rule::BevyWarning => "Warnings that only apply to rendering backends absent in headless runs.",
            Rule::LogError => "Game logs transient recoverable errors by design (retry logic).",
            Rule::LogWarn => "Verbose games log warnings for normal operating conditions.",
            Rule::Panic => "Panic in unwinding recovery wrappers that the game catches elsewhere.",
            Rule::PlannerGoalUnfinished => "Goals achievable only via paths not exposed in the declared intents.",
            Rule::BevySuppressed => "By design: only known-benign records are suppressed; check the list when new warnings appear.",
            Rule::NodesInBounds => "Bounds declared tighter than real level extents (off-screen-but-valid movement).",
            Rule::FiniteTransforms => "Placeholder transforms at spawn before initialization.",
            Rule::FrameTimeAnomaly => "First-frame compile spikes; the smoothed detector mostly ignores them, but heavy headless scheduling can still trip it.",
            Rule::FrameTimeP99Below => "Trivially light scenes on fast machines.",
            Rule::FpsFloor => "Debug builds with heavy logging; headless on busy CI runners.",
            Rule::MetamorphicTickRate => "Games legitimately sensitive to tick rate (animation blending).",
            Rule::AggregateConservation => "Aggregates with legit external flows (level-up grants) — declare the flows.",
            Rule::Nondeterministic => "Float non-associativity across thread counts — use single_threaded mode.",
            Rule::GoldenDiverged => "Legitimate behavior changes after intentional edits — regenerate goldens.",
            Rule::DeadWidget => "Decorative widgets that correctly never react.",
            Rule::DeadVerb => "Verb effects invisible to the observable percepts (internal state only).",
            Rule::UnusedInput => "Input consumed deep in subsystems not visible via percepts.",
            Rule::TransformDesync => "Manually-set GlobalTransforms (cached by design, e.g. UI layout).",
            Rule::PerfBudgetExceeded => "Machine noise — always compare against same-platform baselines.",
            Rule::StuckAfterIntent => "Long deliberate animations before visible reaction; widen timeout_s.",
            Rule::PlannerSelectTargetMissing => "Entities named differently at runtime (dynamic names).",
            Rule::ResetHookPanicked => "Reset hooks assuming resources only present after full startup.",
            Rule::CheatHookPanicked => "Cheat hooks referencing dev-only entities.",
            Rule::NodesRotationUnnormalized => "None known; renormalization is cheap and should hold.",
            Rule::ReadinessTimeout => "Slow asset loading on cold CI caches — raise the budget.",
            Rule::PlannerPostcondition => "Postcondition windows shorter than latency in the game's response.",
            Rule::PlannerGoalTimeout => "max_s tuned for faster play than the game allows.",
            Rule::PlannerBotConfig => "None known — config errors should fail loudly.",
            Rule::PlannerAnyExhausted => "All alternatives genuinely requiring sequential play order.",
            Rule::OracleError => "Oracle infrastructure bugs — report them.",
            Rule::ChaosBotConfig => "None known — config errors should fail loudly.",
            Rule::CustomBotConfig => "Policy registered after scenario load in a different app instance.",
            Rule::PursuitBotConfig => "Names differing between record and replay scenes.",
            Rule::ReplayTargetMissing => "Replay recorded on a different scene/build.",
            Rule::SyntheticPointerConfig => "Games relying on custom picking backends the harness cannot probe.",
            Rule::SyntheticKeyboardConfig => "None known.",
            Rule::RawKeyInvalid => "Key aliases not in the common subset map — extend parse_key_code.",
            Rule::RawGamepadButtonInvalid => "Vendor-specific button naming.",
            Rule::RawGamepadAxisInvalid => "Vendor-specific axis naming.",
            Rule::RawGamepadDisabled => "Forgot the gamepad cargo feature.",
            Rule::RawClickTargetOffscreen => "Letterboxed viewports where logical pixels differ from camera coords.",
            Rule::PointerNotActionable => "Masked-but-functioning overlays; drop require_actionable if intentional.",
            Rule::ChaosSelectUnnamed => "Gameplay entities without Name by design — use StableId targeting.",
            Rule::EntropyUnattributedDraws => "Background threads drawing entropy concurrently with runs.",
            Rule::ScenarioError => "None known — the error text names the rejected field.",
        }
    }

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
                | Rule::UnusedInput
                | Rule::UndrivableGame
                // dbt: warning-severity log rules are informational by
                // default — headless runs routinely trip render/audio
                // warnings that are benign without a GPU or audio
                // device (docs/rules.md "False-positive modes").
                // Escalate opt-in via Scenario::strict_warnings.
                | Rule::BevyWarning
                | Rule::LogWarn
        )
    }
}

declare_rules! {
    FrozenWorld => "frozen_world",
    UndrivableGame => "undrivable_game",
    HarnessCouldNotDriveGame => "harness_could_not_drive_game",
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
    UnusedInput => "unused_input",
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
