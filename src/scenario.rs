//! Scenario schema: DTOs, defaults, and load-time validation.

use serde::{Deserialize, Serialize};

fn default_tps() -> u32 {
    60
}
fn default_true() -> bool {
    true
}
use std::collections::HashSet;

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    pub bot: BotConfig,
    /// Simulation ticks per second. Each `app.update()` advances simulated
    /// time by exactly 1/tps when `simulated_time` is true.
    #[serde(default = "default_tps")]
    pub tps: u32,
    /// Drive `Time` with a fixed delta of 1/tps (deterministic, the default)
    /// instead of the wall clock. Turn off only for soak tests that want
    /// real-time behaviour.
    #[serde(default = "default_true")]
    pub simulated_time: bool,
    /// Run every schedule on Bevy's SingleThreadedExecutor (deterministic
    /// system order for a given schedule graph). Default true: playtests
    /// value reproducibility over throughput. Parallelism comes from
    /// running many scenarios at once, not from threading one App.
    #[serde(default = "default_true")]
    pub single_threaded: bool,
    /// Fail the run (ScenarioError::Rejected listing the ambiguities) when
    /// any schedule has conflicting systems with no order. Opt-in: many
    /// games have benign ambiguities.
    #[serde(default)]
    pub deny_ambiguities: bool,
    #[serde(default = "default_duration")]
    pub duration_s: f32,
    #[serde(default)]
    pub invariants: Vec<Invariant>,
    #[serde(default)]
    pub setup: Setup,
    /// I3: typed-oracle selection. None = run ALL registered typed
    /// oracles; Some(names) restricts to the listed ones. Unknown names
    /// reject at load time (ScenarioError::Rejected).
    #[serde(default)]
    pub oracles: Option<Vec<String>>,
    /// I4: liveness configuration (default 2s timeout, real-time mode).
    #[serde(default)]
    pub liveness: LivenessConfig,
}
/// Key name → KeyCode (common subset; extend as games need).
pub fn parse_key_code(name: &str) -> Option<bevy::input::keyboard::KeyCode> {
    use bevy::input::keyboard::KeyCode::*;
    let lower = name.to_ascii_lowercase();
    Some(match lower.as_str() {
        "space" => Space,
        "enter" | "return" => Enter,
        "escape" => Escape,
        "tab" => Tab,
        "backspace" => Backspace,
        "up" | "arrowup" => ArrowUp,
        "down" | "arrowdown" => ArrowDown,
        "left" | "arrowleft" => ArrowLeft,
        "right" | "arrowright" => ArrowRight,
        "shift" => ShiftLeft,
        "ctrl" => ControlLeft,
        "alt" => AltLeft,
        s => {
            let mut chars = s.chars();
            let c = chars.next()?;
            if chars.next().is_some() {
                return None;
            }
            match c {
                'a'..='z' => KEY_LETTERS.get(c as usize - 'a' as usize).copied(),
                '0'..='9' => KEY_DIGITS.get(c as usize - '0' as usize).copied(),
                _ => None,
            }?
        }
    })
}

const KEY_LETTERS: [bevy::input::keyboard::KeyCode; 26] = [
    bevy::input::keyboard::KeyCode::KeyA,
    bevy::input::keyboard::KeyCode::KeyB,
    bevy::input::keyboard::KeyCode::KeyC,
    bevy::input::keyboard::KeyCode::KeyD,
    bevy::input::keyboard::KeyCode::KeyE,
    bevy::input::keyboard::KeyCode::KeyF,
    bevy::input::keyboard::KeyCode::KeyG,
    bevy::input::keyboard::KeyCode::KeyH,
    bevy::input::keyboard::KeyCode::KeyI,
    bevy::input::keyboard::KeyCode::KeyJ,
    bevy::input::keyboard::KeyCode::KeyK,
    bevy::input::keyboard::KeyCode::KeyL,
    bevy::input::keyboard::KeyCode::KeyM,
    bevy::input::keyboard::KeyCode::KeyN,
    bevy::input::keyboard::KeyCode::KeyO,
    bevy::input::keyboard::KeyCode::KeyP,
    bevy::input::keyboard::KeyCode::KeyQ,
    bevy::input::keyboard::KeyCode::KeyR,
    bevy::input::keyboard::KeyCode::KeyS,
    bevy::input::keyboard::KeyCode::KeyT,
    bevy::input::keyboard::KeyCode::KeyU,
    bevy::input::keyboard::KeyCode::KeyV,
    bevy::input::keyboard::KeyCode::KeyW,
    bevy::input::keyboard::KeyCode::KeyX,
    bevy::input::keyboard::KeyCode::KeyY,
    bevy::input::keyboard::KeyCode::KeyZ,
];

const KEY_DIGITS: [bevy::input::keyboard::KeyCode; 10] = [
    bevy::input::keyboard::KeyCode::Digit0,
    bevy::input::keyboard::KeyCode::Digit1,
    bevy::input::keyboard::KeyCode::Digit2,
    bevy::input::keyboard::KeyCode::Digit3,
    bevy::input::keyboard::KeyCode::Digit4,
    bevy::input::keyboard::KeyCode::Digit5,
    bevy::input::keyboard::KeyCode::Digit6,
    bevy::input::keyboard::KeyCode::Digit7,
    bevy::input::keyboard::KeyCode::Digit8,
    bevy::input::keyboard::KeyCode::Digit9,
];

fn default_duration() -> f32 {
    15.0
}

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct BotConfig {
    #[serde(rename = "type")]
    pub bot_type: crate::enums::BotType,
    #[serde(default = "default_seed")]
    pub seed: u64,
    #[serde(default = "default_rate")]
    pub input_rate_hz: u32,
    /// pursuit: agent/target entity Names
    pub agent_target: Option<String>,
    pub target: Option<String>,
    #[serde(default = "default_deadzone")]
    pub deadzone: f32,
    /// replay: frame-indexed typed intents — genre-free
    #[serde(default)]
    pub inputs: Vec<ReplayInput>,
    /// synthetic_pointer: frame-indexed raw pointer clicks at named
    /// entities. Exercises the FULL input chain (pointer input →
    /// picking backend → PointerClick → game adapter) that
    /// intent-injection bots bypass. A missing mesh/sprite picking
    /// backend fails LOUDLY here instead of silently dead-clicking.
    #[serde(default)]
    pub pointer_clicks: Vec<PointerClickInput>,
    /// synthetic_keyboard: frame-indexed raw key presses. Exercises
    /// the FULL keyboard chain (`KeyboardInput` → `ButtonInput<KeyCode>`
    /// → game adapter) that intent-injection bots bypass. A dead
    /// adapter or wrong key mapping fails the scenario's invariants.
    /// Chaos persona: "uniform" (default), "aggressive" (2x rate, never
    /// Wait), "curious" (reserved for F1 coverage-guided curiosity;
    /// currently behaves as uniform), "idle" (mostly Wait, occasional
    /// jabs). REPLACES the legacy text: "curious" previously meant
    /// bias toward unseen variants (not implemented).
    /// (stronger unseen-variant bias), "idle" (mostly waiting, rare
    /// jabs). Different personas find different bugs.
    #[serde(default)]
    pub persona: Option<crate::enums::Persona>,
    #[serde(default)]
    pub key_presses: Vec<KeyPressInput>,
    /// Raw input surface for chaos/curious bots (Z3). When present,
    /// chaos bot emits RawActions instead of UserIntents.
    #[serde(default)]
    pub raw_surface: Option<RawSurface>,
    /// planner: declarative goal tree (aplib-inspired). The scenario
    /// author writes WHAT to achieve (TestApi predicates), not WHEN
    /// to press.
    #[serde(default)]
    pub goals: Option<crate::planner::GoalNode>,
    /// custom: name of a registered typed bot policy
    /// (PlaytestAppExt::add_bot_policy). Unknown names reject at load.
    #[serde(default)]
    pub policy: Option<String>,
    /// J0: replay prefix for the bot's random choices (recorded
    /// values; each draw consumes one). Absent = pure PRNG from seed.
    #[serde(default)]
    pub choices: Option<Vec<crate::choice::Choice>>,
    /// J0: continuation once replayed choices run out (prng/zeros/
    /// reseed). Default prng.
    #[serde(default)]
    pub continuation: crate::choice::Continuation,
}

/// I4: liveness configuration.
#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct LivenessConfig {
    /// Liveness mode (default: real-time).
    #[serde(default)]
    pub mode: LivenessMode,
    /// Timeout for liveness signal (seconds).
    #[serde(default = "default_liveness_timeout")]
    pub timeout_s: f32,
    /// Optional watched resources/components by type path (future Z6).
    #[serde(default)]
    pub watch: Vec<String>,
}

fn default_liveness_timeout() -> f32 {
    2.0
}

impl Default for LivenessConfig {
    fn default() -> Self {
        Self {
            mode: LivenessMode::default(),
            timeout_s: default_liveness_timeout(),
            watch: Vec::new(),
        }
    }
}

/// I4: liveness mode.
#[derive(Deserialize, Serialize, Clone, Debug, Default)]
#[serde(rename_all = "snake_case")]
pub enum LivenessMode {
    /// Real-time: violation if no liveness signal for timeout_s.
    #[default]
    RealTime,
    /// Turn-based: idle is fine, but after an intent some liveness must occur.
    AfterIntent,
    /// Disable the liveness oracle (loud: recorded in report warnings).
    Off,
}

fn default_seed() -> u64 {
    42
}
fn default_rate() -> u32 {
    10
}
fn default_deadzone() -> f32 {
    10.0
}

/// Raw input surface declaration (Z3). Defines what raw inputs are available
/// for chaos/curious bots to sample from.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, Default)]
pub struct RawSurface {
    #[serde(default)]
    pub keys: Vec<String>,
    #[serde(default)]
    pub mouse_buttons: bool,
    #[serde(default)]
    pub mouse_motion: bool,
    #[serde(default)]
    pub wheel: bool,
    #[serde(default)]
    pub gamepad: bool,
    #[serde(default)]
    pub clickables: bool, // ClickEntity targets discovered by Z6/Z5
}

/// A typed replay intent. Whatever intent sequence a scenario author
/// (or a recorded human play session translated through the adapter)
/// provides is replayed verbatim.
#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct ReplayInput {
    pub frame: u64,
    pub intent: ReplayIntent,
}

/// A raw input action for the tier-0 actuator (Z3). Peer of ReplayIntent:
/// keys, mouse, cursor, gamepad, wheel, and ClickEntity gestures.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(tag = "raw", rename_all = "snake_case")]
pub enum RawAction {
    /// Key press (reuse parse_key_code for validation).
    Key { key: String, hold_frames: u32 },
    /// Mouse button press.
    MouseButton { button: MouseBtn, hold_frames: u32 },
    /// Mouse motion (delta in logical pixels).
    MouseMove { delta: (f32, f32) },
    /// Absolute cursor position (logical pixels, viewport coordinates).
    Cursor { pos: (f32, f32) },
    /// Click gesture: move to pos, press, hold, release.
    Click {
        pos: (f32, f32),
        button: MouseBtn,
        hold_frames: u32,
    },
    /// Click an entity by Name or StableId (resolved each frame).
    ClickEntity { target: EntityRef },
    /// Mouse wheel scroll.
    Wheel { dy: f32 },
    /// Gamepad button.
    GamepadButton { button: String, hold_frames: u32 },
    /// Gamepad axis.
    GamepadAxis {
        axis: String,
        value: f32,
        hold_frames: u32,
    },
    /// Wait N frames (no-op, for pacing).
    Wait { frames: u32 },
}

/// Reference to a gameplay entity (by Name or StableId). Used by ClickEntity.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(untagged)]
pub enum EntityRef {
    ByName(String),
    ByStableId(u64),
}

/// Mouse button for raw input (wire-compatible with PointerButton).
#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum MouseBtn {
    Left,
    Right,
    Middle,
}

/// synthetic_pointer: click a named entity at a given frame. The
/// pointer position is derived from the entity's world transform via
/// the camera, so the click lands on whatever the picking backend
/// actually hits — which is the point.
/// synthetic_keyboard: press+release a key at a given frame, through
/// the real keyboard input chain.
#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct KeyPressInput {
    pub frame: u64,
    /// Key name: "space", "enter", "escape", "tab", "backspace",
    /// "up"/"down"/"left"/"right", single letters a-z, digits 0-9.
    pub key: String,
    /// Hold the key for N frames (default 1). Charge attacks, dash
    /// charging, and long-press menus require this.
    #[serde(default = "default_hold_frames")]
    pub hold_frames: u64,
}

fn default_hold_frames() -> u64 {
    1
}

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct PointerClickInput {
    pub frame: u64,
    /// Entity Name to click (resolved against Gameplay entities).
    pub target: String,
    /// Primary button by default; "secondary"/"middle" supported.
    #[serde(default)]
    pub button: crate::enums::PointerButton,
    /// ACTIONABILITY GATE (Playwright-style, opt-in): after the click
    /// gesture completes, verify the picking backend's hover map shows
    /// the pointer over SOME entity — i.e. the click was receivable by
    /// the game's picking wiring. A click through a masked/occluded
    /// target or with a dead picking backend reports
    /// `pointer_not_actionable` instead of silently hitting nothing.
    #[serde(default)]
    pub require_actionable: bool,
    /// Hold the button for N frames before releasing (default 1).
    /// Long-press interactions, drag-to-hold menus.
    #[serde(default = "default_hold_frames")]
    pub hold_frames: u64,
}

pub fn action_log_to_timed_actions(log: &[crate::contract::ActionEntry]) -> Vec<TimedAction> {
    log.iter()
        .map(|e| TimedAction {
            frame: e.frame,
            source: match &e.source {
                crate::contract::ActionSource::BotScenario(s) => format!("scenario:{}", s),
                crate::contract::ActionSource::Agent => "agent".into(),
                crate::contract::ActionSource::Manual => "manual".into(),
            },
            action: e.action.clone(),
            details: e.details.clone(),
        })
        .collect()
}

#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "intent", rename_all = "snake_case", deny_unknown_fields)]
pub enum ReplayIntent {
    Move {
        dir: (f32, f32),
    },
    Choice {
        index: usize,
    },
    Axis {
        name: String,
        value: f32,
    },
    /// Select a tile/entity BY NAME or StableId — resolved against live
    /// `Name` + `Gameplay` entities at fire time. Unlike raw-entity
    /// Selects, this survives reset cycles (fresh entity ids) — the
    /// replay scenario refers to stable scene identities. StableId
    /// targets work for unnamed Gameplay entities (I1).
    Select {
        target: SelectTarget,
    },
    Wait,
}

/// Target of a replay Select: Name (plain string, back-compat) or
/// StableId (unnamed Gameplay entities; I1).
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(untagged)]
pub enum SelectTarget {
    /// Plain string name (existing JSON scenarios keep working).
    Name(String),
    /// StableId object form: {"stable_id": n}.
    Stable { stable_id: u64 },
}

/// Timed action for replay from ActionLog. Frame-aligned with the original
/// run for deterministic reproduction. Used by the replay bot and ddmin
/// minimizer to reconstruct crash-inducing sequences.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct TimedAction {
    pub frame: u64,
    pub source: String,
    pub action: String,
    pub details: Option<String>,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct Invariant {
    pub name: String,
    pub rule: crate::enums::InvariantRule,
    // nodes_in_bounds
    pub min_x: Option<f32>,
    pub max_x: Option<f32>,
    pub min_y: Option<f32>,
    pub max_y: Option<f32>,
    pub min_z: Option<f32>,
    pub max_z: Option<f32>,
    /// Entity Names. Empty = all Gameplay entities.
    #[serde(default)]
    pub targets: Vec<String>,
    // frame_time_p99_below / fps_floor / custom thresholds
    pub value: Option<serde_json::Value>,
    // custom: "TestApi.score" (numeric) or "TestApi.game_phase" (string)
    pub path: Option<String>,
    pub check: Option<crate::enums::CheckOp>,
    pub after_s: Option<f32>,
    pub before_s: Option<f32>,
    pub max_delta_per_sec: Option<f64>,
    /// Differential invariants: no_decrease (value must never drop) or
    /// no_increase (value must never rise). Requires numeric path.
    pub differential: Option<crate::enums::DifferentialOp>,
    /// Eventually mode: the rule must hold AT LEAST ONCE within this
    /// deadline (seconds). Unlike always-mode invariants (per-frame),
    /// eventually waits for the first satisfaction and reports failure
    /// only if the deadline expires unmet. Playwright/gdUnit pattern.
    pub eventually_s: Option<f32>,
    /// Query-shaped target: component filters instead of a TestApi path.
    /// When present, the rule applies to the COUNT of entities matching
    /// the filter (e.g., "count_above", "count_equals"). This is the
    /// ECS-native analogue of Playwright's `toHaveCount`.
    #[serde(default)]
    pub query: Option<QueryTarget>,
    /// Expert-rule oracle: WHEN `path` satisfies `check` vs
    /// `value`, REQUIRE `requires_path` to satisfy `requires_check` vs
    /// `requires_value`. Checks game POLICY compliance ("low HP ⇒ heal"),
    /// complementing state invariants.
    pub requires_path: Option<String>,
    pub requires_check: Option<crate::enums::CheckOp>,
    pub requires_value: Option<serde_json::Value>,
}

#[derive(Deserialize, Serialize, Clone, Debug, Default)]
#[serde(deny_unknown_fields)]
pub struct QueryTarget {
    /// Component types that MUST be present.
    #[serde(default)]
    pub with: Vec<String>,
    /// Component types that MUST NOT be present.
    #[serde(default)]
    pub without: Vec<String>,
}

#[derive(Deserialize, Serialize, Clone, Debug, Default)]
#[serde(deny_unknown_fields)]
pub struct Setup {
    #[serde(default)]
    pub resets: Vec<ResetCall>,
    /// Cheats invoked at scheduled frames, mid-run (AUDITED — every
    /// invocation lands in the action log and bumps cheat_count).
    #[serde(default)]
    pub cheats: Vec<CheatCall>,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct CheatCall {
    pub frame: u64,
    pub kind: String,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct ResetCall {
    pub kind: String,
}

// ---------------------------------------------------------------------------
// Load-time validation (reject, never run)
// ---------------------------------------------------------------------------

/// Errors rejecting a scenario before/at run start. Structured so
/// callers can match on the category (e.g. missing contract pieces)
/// instead of parsing strings.
#[derive(Debug, Clone, PartialEq)]
pub enum ScenarioError {
    /// The scenario JSON is structurally invalid (bad field combos).
    Rejected(String),
    /// The game does not implement the contract (missing TestApi).
    ContractMissing(String),
    /// The scenario references a reset kind the game never registered.
    UnknownReset { kind: String, known: Vec<String> },
    /// The scenario references a cheat kind the game never registered.
    UnknownCheat { kind: String, known: Vec<String> },
    /// A query-target component name failed to resolve.
    InvalidComponent { name: String, reason: String },
}

impl std::fmt::Display for ScenarioError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ScenarioError::Rejected(msg) => write!(f, "scenario rejected: {}", msg),
            ScenarioError::ContractMissing(msg) => write!(f, "contract missing: {}", msg),
            ScenarioError::UnknownReset { kind, known } => write!(
                f,
                "unknown reset kind '{}' — known kinds: {}",
                kind,
                if known.is_empty() {
                    "(none registered)".to_string()
                } else {
                    known.join(", ")
                }
            ),
            ScenarioError::UnknownCheat { kind, known } => write!(
                f,
                "unknown cheat kind '{}' — known kinds: {}",
                kind,
                if known.is_empty() {
                    "(none registered)".to_string()
                } else {
                    known.join(", ")
                }
            ),
            ScenarioError::InvalidComponent { name, reason } => {
                write!(f, "invalid component '{}': {}", name, reason)
            }
        }
    }
}

impl std::error::Error for ScenarioError {}

pub fn validate_scenario(scenario: &Scenario) -> Result<(), ScenarioError> {
    // Validate tps and duration_s
    if scenario.tps < 1 {
        return Err(ScenarioError::Rejected("tps must be >= 1".into()));
    }
    if !scenario.duration_s.is_finite() || scenario.duration_s <= 0.0 {
        return Err(ScenarioError::Rejected(
            "duration_s must be finite and > 0".into(),
        ));
    }
    // Bot-specific requirements (C6).
    use crate::enums::BotType;
    match scenario.bot.bot_type {
        BotType::Planner => {
            if scenario.bot.goals.is_none() {
                return Err(ScenarioError::Rejected(
                    "planner bot requires a non-empty goals tree".into(),
                ));
            }
        }
        BotType::Pursuit => {
            if scenario.bot.agent_target.is_none() && scenario.bot.target.is_none() {
                return Err(ScenarioError::Rejected(
                    "pursuit bot requires agent_target or target".into(),
                ));
            }
        }
        BotType::SyntheticPointer => {
            if scenario.bot.pointer_clicks.is_empty() {
                return Err(ScenarioError::Rejected(
                    "synthetic_pointer bot requires a non-empty pointer_clicks list".into(),
                ));
            }
        }
        BotType::SyntheticKeyboard => {
            if scenario.bot.key_presses.is_empty() {
                return Err(ScenarioError::Rejected(
                    "synthetic_keyboard bot requires a non-empty key_presses list".into(),
                ));
            }
            for kp in &scenario.bot.key_presses {
                if parse_key_code(&kp.key).is_none() {
                    return Err(ScenarioError::Rejected(format!(
                        "key_presses: unknown key name '{}' at frame {}",
                        kp.key, kp.frame
                    )));
                }
            }
        }
        _ => {}
    }

    // Invariant names key the eventual-state and dedup tables —
    // duplicates would silently shadow each other.
    let mut seen = HashSet::new();
    for rule in &scenario.invariants {
        if !seen.insert(rule.name.as_str()) {
            return Err(ScenarioError::Rejected(format!(
                "duplicate invariant name '{}' — names must be unique",
                rule.name
            )));
        }
    }
    for rule in &scenario.invariants {
        if rule.rule == crate::enums::InvariantRule::Custom {
            let check = rule.check.unwrap_or(crate::enums::CheckOp::Le);
            // Text comparison: only Equals/Ne accept string values.
            if let Some(v) = &rule.value {
                if !v.is_number() && check.holds_text("", "").is_none() {
                    return Err(ScenarioError::Rejected(format!(
                        "invariant '{}' uses check='{}' with a non-numeric value ({}) — use check:'equals'/'ne' or a numeric threshold",
                        rule.name, check, v
                    )));
                }
            }
        }
        if let Some(mps) = rule.max_delta_per_sec {
            if mps < 0.0 {
                return Err(ScenarioError::Rejected(format!(
                    "invariant '{}' has negative max_delta_per_sec",
                    rule.name
                )));
            }
        }
        if rule.differential.is_some() && rule.path.is_none() {
            return Err(ScenarioError::Rejected(format!(
                "invariant '{}' uses differential without a TestApi path",
                rule.name
            )));
        }
        // Time-window sanity (C6).
        if let Some(ev) = rule.eventually_s {
            if ev <= 0.0 {
                return Err(ScenarioError::Rejected(format!(
                    "invariant '{}' has eventually_s <= 0",
                    rule.name
                )));
            }
        }
        if let (Some(after), Some(before)) = (rule.after_s, rule.before_s) {
            if after >= before {
                return Err(ScenarioError::Rejected(format!(
                    "invariant '{}' has after_s ({}) >= before_s ({})",
                    rule.name, after, before
                )));
            }
        }
        // Bounds sanity (C6).
        let bound_pairs = [
            (rule.min_x, rule.max_x, "x"),
            (rule.min_y, rule.max_y, "y"),
            (rule.min_z, rule.max_z, "z"),
        ];
        for (min, max, axis) in bound_pairs {
            if let (Some(lo), Some(hi)) = (min, max) {
                if lo > hi {
                    return Err(ScenarioError::Rejected(format!(
                        "invariant '{}' has min_{} ({}) > max_{} ({})",
                        rule.name, axis, lo, axis, hi
                    )));
                }
            }
        }
        // Expert-rule validation: requires_* only valid when WHEN clause exists
        if (rule.requires_path.is_some() || rule.requires_check.is_some()) && rule.path.is_none() {
            return Err(ScenarioError::Rejected(format!(
                "invariant '{}' specifies requires_* without a WHEN path",
                rule.name
            )));
        }
        if rule.requires_check.is_some() && rule.requires_value.is_none() {
            return Err(ScenarioError::Rejected(format!(
                "invariant '{}' specifies requires_check without requires_value",
                rule.name
            )));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
