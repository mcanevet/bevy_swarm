//! zp2: Rust builder API for scenarios. Writing raw JSON strings is
//! error-prone (no autocomplete, no typo checking until runtime);
//! `ScenarioBuilder` lets authors compose scenarios in code with the
//! compiler checking structure. Produces JSON identical to the
//! hand-written examples (unit-tested in tests/scenario_builder.rs).
//!
//! Complements the future `#[swarm::test]` attribute macro (jvi):
//! the builder covers "scenario as data" flows (loading from a set,
//! mixing with JSON fixtures), while the macro will cover "scenario
//! as test" flows.
//!
//! ```
//! use bevy_playtest::builder::ScenarioBuilder;
//!
//! let scenario = ScenarioBuilder::chaos()
//!     .name("menu-navigation")
//!     .duration_s(10.0)
//!     .custom_invariant("hp-floor", "TestApi.hp", "above", 10)
//!     .build();
//! ```

use crate::scenario::{
    BotConfig, CheatCall, Invariant, KeyPressInput, PointerClickInput, ReplayInput, ResetCall,
    Scenario, Setup,
};

/// Default seed/rate/deadzone mirror scenario.rs so builder JSON
/// round-trips identically to hand-written JSON with omitted fields.
fn default_seed() -> u64 {
    42
}
fn default_rate() -> u32 {
    10
}
fn default_deadzone() -> f32 {
    10.0
}

#[derive(Debug, Clone)]
pub struct ScenarioBuilder {
    inner: Scenario,
}

impl ScenarioBuilder {
    fn base(bot: BotConfig) -> Self {
        Self {
            inner: Scenario {
                name: None,
                bot,
                tps: 60,
                simulated_time: true,
                single_threaded: true,
                deny_ambiguities: false,
                allow_ambiguities: Vec::new(),
                strict_warnings: false,
                duration_s: 15.0,
                invariants: vec![],
                setup: Setup::default(),
                oracles: None,
                liveness: Default::default(),
                check_determinism: None,
                compare_golden: None,
                collision_policy: Default::default(),
            },
        }
    }

    /// Chaos bot over the default raw surface (WASD + arrows).
    pub fn chaos() -> Self {
        Self::base(BotConfig {
            bot_type: crate::enums::BotType::Chaos,
            seed: default_seed(),
            input_rate_hz: default_rate(),
            agent_target: None,
            target: None,
            deadzone: default_deadzone(),
            inputs: vec![],
            pointer_clicks: vec![],
            persona: None,
            key_presses: vec![],
            raw_surface: None,
            goals: None,
            policy: None,
            choices: None,
            continuation: Default::default(),
            actuator: None,
        })
    }

    /// Synthetic keyboard bot pressing `key` (bevy-style "KeyW",
    /// "Space", ...) at `frame`, held for `hold_frames`.
    pub fn keyboard() -> KeyboardBuilder {
        KeyboardBuilder {
            bot: BotConfig {
                bot_type: crate::enums::BotType::SyntheticKeyboard,
                seed: default_seed(),
                input_rate_hz: default_rate(),
                agent_target: None,
                target: None,
                deadzone: default_deadzone(),
                inputs: vec![],
                pointer_clicks: vec![],
                persona: None,
                key_presses: vec![],
                raw_surface: None,
                goals: None,
                policy: None,
                choices: None,
                continuation: Default::default(),
                actuator: None,
            },
            done: Self::base(BotConfig {
                bot_type: crate::enums::BotType::SyntheticKeyboard,
                seed: default_seed(),
                input_rate_hz: default_rate(),
                agent_target: None,
                target: None,
                deadzone: default_deadzone(),
                inputs: vec![],
                pointer_clicks: vec![],
                persona: None,
                key_presses: vec![],
                raw_surface: None,
                goals: None,
                policy: None,
                choices: None,
                continuation: Default::default(),
                actuator: None,
            }),
        }
    }

    /// Synthetic pointer bot clicking named entities at frames.
    pub fn pointer() -> PointerBuilder {
        PointerBuilder {
            bot: BotConfig {
                bot_type: crate::enums::BotType::SyntheticPointer,
                seed: default_seed(),
                input_rate_hz: default_rate(),
                agent_target: None,
                target: None,
                deadzone: default_deadzone(),
                inputs: vec![],
                pointer_clicks: vec![],
                persona: None,
                key_presses: vec![],
                raw_surface: None,
                goals: None,
                policy: None,
                choices: None,
                continuation: Default::default(),
                actuator: None,
            },
            done: Self::base(BotConfig {
                bot_type: crate::enums::BotType::SyntheticPointer,
                seed: default_seed(),
                input_rate_hz: default_rate(),
                agent_target: None,
                target: None,
                deadzone: default_deadzone(),
                inputs: vec![],
                pointer_clicks: vec![],
                persona: None,
                key_presses: vec![],
                raw_surface: None,
                goals: None,
                policy: None,
                choices: None,
                continuation: Default::default(),
                actuator: None,
            }),
        }
    }

    /// Replay bot with frame-indexed typed intents.
    pub fn replay(inputs: Vec<ReplayInput>) -> Self {
        let mut b = Self::chaos();
        b.inner.bot.bot_type = crate::enums::BotType::Replay;
        b.inner.bot.inputs = inputs;
        b
    }

    /// Planner bot with a declarative goal tree.
    pub fn planner(goals: crate::planner::GoalNode) -> Self {
        let mut b = Self::chaos();
        b.inner.bot.bot_type = crate::enums::BotType::Planner;
        b.inner.bot.goals = Some(goals);
        b
    }

    // -----------------------------------------------------------------
    // Common knobs
    // -----------------------------------------------------------------

    /// Stable scenario name (golden-trajectory file naming).
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.inner.name = Some(name.into());
        self
    }

    /// Simulation ticks per second (default 60).
    pub fn tps(mut self, tps: u32) -> Self {
        self.inner.tps = tps;
        self
    }

    /// Run length in seconds (default 15).
    pub fn duration_s(mut self, s: f32) -> Self {
        self.inner.duration_s = s;
        self
    }

    /// PRNG seed (default 42).
    pub fn seed(mut self, seed: u64) -> Self {
        self.inner.bot.seed = seed;
        self
    }

    /// Input events per second (default 10).
    pub fn input_rate_hz(mut self, hz: u32) -> Self {
        self.inner.bot.input_rate_hz = hz;
        self
    }

    /// Escalate warning-class violations (bevy_warning, log_warn) to
    /// gating severity.
    pub fn strict_warnings(mut self) -> Self {
        self.inner.strict_warnings = true;
        self
    }

    /// Restrict to named typed oracles (None = all registered).
    pub fn oracles(mut self, names: Vec<String>) -> Self {
        self.inner.oracles = Some(names);
        self
    }

    /// Run N times with the same seed and compare state digests
    /// (opt-in determinism check). Panics on N < 2.
    pub fn check_determinism(mut self, n: usize) -> Self {
        assert!(n >= 2, "check_determinism requires n >= 2");
        self.inner.check_determinism = Some(n);
        self
    }

    /// Register a reset call invoked before the run.
    pub fn reset(mut self, kind: impl Into<String>) -> Self {
        self.inner
            .setup
            .resets
            .push(ResetCall { kind: kind.into() });
        self
    }

    /// Schedule a cheat at `frame` (audited in the action log).
    pub fn cheat(mut self, frame: u64, kind: impl Into<String>) -> Self {
        self.inner.setup.cheats.push(CheatCall {
            frame,
            kind: kind.into(),
        });
        self
    }

    /// Raw input surface for chaos bots: sample from exactly these keys.
    pub fn raw_keys(mut self, keys: Vec<String>) -> Self {
        self.inner.bot.raw_surface = Some(crate::scenario::RawSurface {
            keys,
            ..Default::default()
        });
        self
    }

    // -----------------------------------------------------------------
    // Invariants
    // -----------------------------------------------------------------

    /// Custom TestApi invariant: numeric/string path compared with
    /// `check` ("gt" | "ge" | "lt" | "le" | "equals" | "ne") against
    /// `value`. Panics on an unknown check name (compile-time-ish
    /// feedback for the typo case the JSON path only catches at load).
    pub fn custom_invariant(
        mut self,
        name: impl Into<String>,
        path: impl Into<String>,
        check: &str,
        value: impl Into<serde_json::Value>,
    ) -> Self {
        let op = parse_check(check)
            .unwrap_or_else(|| panic!("unknown check '{check}': expected gt|ge|lt|le|equals|ne"));
        self.inner.invariants.push(Invariant {
            name: name.into(),
            rule: crate::enums::InvariantRule::Custom,
            min_x: None,
            max_x: None,
            min_y: None,
            max_y: None,
            min_z: None,
            max_z: None,
            targets: vec![],
            value: Some(value.into()),
            path: Some(path.into()),
            check: Some(op),
            after_s: None,
            before_s: None,
            max_delta_per_sec: None,
            differential: None,
            eventually_s: None,
            query: None,
            requires_path: None,
            requires_check: None,
            requires_value: None,
            scale: None,
        });
        self
    }

    /// fps_floor invariant: report when FPS drops below `fps`.
    pub fn fps_floor(mut self, name: impl Into<String>, fps: f64) -> Self {
        self.inner.invariants.push(Invariant {
            name: name.into(),
            rule: crate::enums::InvariantRule::FpsFloor,
            min_x: None,
            max_x: None,
            min_y: None,
            max_y: None,
            min_z: None,
            max_z: None,
            targets: vec![],
            value: Some(serde_json::json!(fps)),
            path: None,
            check: None,
            after_s: None,
            before_s: None,
            max_delta_per_sec: None,
            differential: None,
            eventually_s: None,
            query: None,
            requires_path: None,
            requires_check: None,
            requires_value: None,
            scale: None,
        });
        self
    }

    /// Query-count invariant: the number of entities matching the
    /// component filter must satisfy `check` vs `count`. ECS analogue
    /// of Playwright's `toHaveCount`.
    pub fn count_invariant(
        mut self,
        name: impl Into<String>,
        with: Vec<String>,
        check: &str,
        count: i64,
    ) -> Self {
        let op = parse_check(check)
            .unwrap_or_else(|| panic!("unknown check '{check}': expected gt|ge|lt|le|equals|ne"));
        self.inner.invariants.push(Invariant {
            name: name.into(),
            rule: crate::enums::InvariantRule::Custom,
            min_x: None,
            max_x: None,
            min_y: None,
            max_y: None,
            min_z: None,
            max_z: None,
            targets: vec![],
            value: Some(serde_json::json!(count)),
            path: None,
            check: Some(op),
            after_s: None,
            before_s: None,
            max_delta_per_sec: None,
            differential: None,
            eventually_s: None,
            query: Some(crate::scenario::QueryTarget {
                with,
                without: vec![],
            }),
            requires_path: None,
            requires_check: None,
            requires_value: None,
            scale: None,
        });
        self
    }

    /// Eventually-mode invariant: `path` must satisfy `check` vs
    /// `value` at least once within `deadline_s` seconds.
    pub fn eventually(
        mut self,
        name: impl Into<String>,
        path: impl Into<String>,
        check: &str,
        value: impl Into<serde_json::Value>,
        deadline_s: f32,
    ) -> Self {
        let op = parse_check(check)
            .unwrap_or_else(|| panic!("unknown check '{check}': expected gt|ge|lt|le|equals|ne"));
        self.inner.invariants.push(Invariant {
            name: name.into(),
            rule: crate::enums::InvariantRule::Custom,
            min_x: None,
            max_x: None,
            min_y: None,
            max_y: None,
            min_z: None,
            max_z: None,
            targets: vec![],
            value: Some(value.into()),
            path: Some(path.into()),
            check: Some(op),
            after_s: None,
            before_s: None,
            max_delta_per_sec: None,
            differential: None,
            eventually_s: Some(deadline_s),
            query: None,
            requires_path: None,
            requires_check: None,
            requires_value: None,
            scale: None,
        });
        self
    }

    // -----------------------------------------------------------------
    // Terminal
    // -----------------------------------------------------------------

    /// Finish: validates and returns the scenario. Panics on invalid
    /// configuration (mirrors the JSON path's load-time rejection, but
    /// at author time).
    pub fn build(self) -> Scenario {
        if let Err(e) = crate::scenario::validate_scenario(&self.inner) {
            panic!("builder produced an invalid scenario: {e}");
        }
        self.inner
    }

    /// Finish without panicking on validation errors.
    pub fn try_build(self) -> Result<Scenario, crate::scenario::ScenarioError> {
        crate::scenario::validate_scenario(&self.inner)?;
        Ok(self.inner)
    }
}

fn parse_check(name: &str) -> Option<crate::enums::CheckOp> {
    use crate::enums::CheckOp::*;
    Some(match name.to_ascii_lowercase().as_str() {
        "gt" | "above" => Gt,
        "ge" => Ge,
        "lt" => Lt,
        "le" | "below" => Le,
        "equals" | "eq" => Equals,
        "ne" => Ne,
        _ => return None,
    })
}

/// synthetic_keyboard bot with fluent key-press composition.
pub struct KeyboardBuilder {
    bot: BotConfig,
    done: ScenarioBuilder,
}

impl KeyboardBuilder {
    /// Press `key` (bevy-style: "KeyW", "Space", "Enter") at `frame`.
    pub fn press(mut self, key: &str, frame: u64) -> Self {
        self.bot.key_presses.push(KeyPressInput {
            frame,
            key: key.to_string(),
            hold_frames: 1,
        });
        self
    }

    /// Press `key` at `frame`, holding for `hold_frames`.
    pub fn hold(mut self, key: &str, frame: u64, hold_frames: u64) -> Self {
        self.bot.key_presses.push(KeyPressInput {
            frame,
            key: key.to_string(),
            hold_frames,
        });
        self
    }

    /// Apply common knobs (see [`ScenarioBuilder`]).
    pub fn configure(self, f: impl FnOnce(ScenarioBuilder) -> ScenarioBuilder) -> Self {
        Self {
            bot: self.bot,
            done: f(self.done),
        }
    }

    /// Finish: validates and returns the scenario.
    pub fn build(self) -> Scenario {
        let mut s = self.done;
        s.inner.bot = self.bot;
        s.build()
    }

    /// Finish without panicking on validation errors.
    pub fn try_build(self) -> Result<Scenario, crate::scenario::ScenarioError> {
        let mut s = self.done;
        s.inner.bot = self.bot;
        s.try_build()
    }
}

/// synthetic_pointer bot with fluent click composition.
pub struct PointerBuilder {
    bot: BotConfig,
    done: ScenarioBuilder,
}

impl PointerBuilder {
    /// Click the entity named `target` at `frame`.
    pub fn click(mut self, target: &str, frame: u64) -> Self {
        self.bot.pointer_clicks.push(PointerClickInput {
            frame,
            target: target.to_string(),
            button: Default::default(),
            require_actionable: false,
            hold_frames: 1,
        });
        self
    }

    /// Like `click` but require the click be receivable (Playwright
    /// actionability gate): occluded targets or a dead picking backend
    /// report pointer_not_actionable instead of silently missing.
    pub fn click_actionable(mut self, target: &str, frame: u64) -> Self {
        self.bot.pointer_clicks.push(PointerClickInput {
            frame,
            target: target.to_string(),
            button: Default::default(),
            require_actionable: true,
            hold_frames: 1,
        });
        self
    }

    /// Like `click` but hold the button for `hold_frames` (long-press
    /// interactions, drag-to-hold menus).
    pub fn click_hold(mut self, target: &str, frame: u64, hold_frames: u64) -> Self {
        self.bot.pointer_clicks.push(PointerClickInput {
            frame,
            target: target.to_string(),
            button: Default::default(),
            require_actionable: false,
            hold_frames,
        });
        self
    }

    /// Apply common knobs (see [`ScenarioBuilder`]).
    pub fn configure(self, f: impl FnOnce(ScenarioBuilder) -> ScenarioBuilder) -> Self {
        Self {
            bot: self.bot,
            done: f(self.done),
        }
    }

    /// Finish: validates and returns the scenario.
    pub fn build(self) -> Scenario {
        let mut s = self.done;
        s.inner.bot = self.bot;
        s.build()
    }

    /// Finish without panicking on validation errors.
    pub fn try_build(self) -> Result<Scenario, crate::scenario::ScenarioError> {
        let mut s = self.done;
        s.inner.bot = self.bot;
        s.try_build()
    }
}
