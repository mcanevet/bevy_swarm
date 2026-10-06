//! Headless in-process playtest harness for Bevy (port of Godot test_player.gd).
//!
//! Lives in the `bevy_swarm` crate — games consume it as a path
//! dependency instead of copying 1900 lines into src/. NO game-specific
//! glue in this file.
//!
//! Verified against Bevy 0.20.0-rc.2 APIs via cargo check + cargo test.
//! NOTE: in 0.20.0-rc.2, buffered events are `Message`s
//! (`app.add_message`, `MessageWriter`/`MessageReader` in
//! `bevy::ecs::message`) — `Event` is now the observer system.

use crate::contract::{
    IntentSurface, ResetHooks, SurfaceVariant, TestApi, TestApiResolve, UserIntent,
};
use bevy::app::App;
use bevy::prelude::Camera;
use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::ecs::entity::Entity;
use bevy::ecs::message::MessageWriter;
use bevy::ecs::query::{Changed, With};
use bevy::ecs::resource::Resource;
use bevy::ecs::system::{Query, Res, ResMut};
use bevy::ecs::world::World;
use bevy::picking::pointer::{PointerAction, PointerButton, PointerId, PointerInput};
use bevy::prelude::{GlobalTransform, Name, Transform};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

// Re-export for consumers that only depend on the harness module.
pub use crate::contract::Gameplay;
use crate::contract::TestFieldValue;
use crate::contract::HARNESS_TYPE_PATHS;

// ---------------------------------------------------------------------------
// Scenario schema (mirrors test_player.gd JSON)
// ---------------------------------------------------------------------------

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    pub bot: BotConfig,
    #[serde(default = "default_duration")]
    pub duration_s: f32,
    #[serde(default)]
    pub invariants: Vec<Invariant>,
    #[serde(default)]
    pub setup: Setup,
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
    /// the FULL keyboard chain (KeyboardInput → ButtonInput<KeyCode>
    /// → game adapter) that intent-injection bots bypass. A dead
    /// adapter or wrong key mapping fails the scenario's invariants.
    /// Chaos persona: "aggressive" (2x rate, never Wait), "curious"
    /// (stronger unseen-variant bias), "idle" (mostly waiting, rare
    /// jabs). Different personas find different bugs (MIMIC).
    #[serde(default)]
    pub persona: Option<crate::enums::Persona>,
    #[serde(default)]
    pub key_presses: Vec<KeyPressInput>,
    /// planner: declarative goal tree (aplib-inspired). The scenario
    /// author writes WHAT to achieve (TestApi predicates), not WHEN
    /// to press. See `planner.rs` and `references/planner.md`.
    #[serde(default)]
    pub goals: Option<crate::planner::GoalNode>,
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

/// A typed replay intent. Whatever intent sequence a scenario author
/// (or a recorded human play session translated through the adapter)
/// provides is replayed verbatim.
#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct ReplayInput {
    pub frame: u64,
    pub intent: ReplayIntent,
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
    #[serde(default = "default_pointer_button")]
    pub button: String,
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

fn default_pointer_button() -> String {
    "primary".to_string()
}

// ---------------------------------------------------------------------------
// Ddmin — Delta Debugging minimizer (Zeller's ddmin over action sequences).
// Rebellion-style: record the full action stream, binary-search removals,
// keep a minimal reproducer. The result is a permanent regression scenario.
// ---------------------------------------------------------------------------

/// Minimize an action sequence that reproduces a failure. `reproduce`
/// receives candidate subsequences; must be deterministic and side-effect
/// isolated (caller is responsible for resetting world state between calls).
/// Returns the smallest found prefix-preserving subsequence that still fails.
pub fn ddmin_minimize<T: Clone>(actions: &[T], reproduce: impl Fn(&[T]) -> bool) -> Vec<T> {
    let mut cur: Vec<T> = actions.to_vec();
    if cur.is_empty() || !reproduce(&cur) {
        return cur; // not reproducible (or empty) — return as-is
    }
    let mut n = 2;
    while cur.len() > 1 {
        let chunk = (cur.len() as f64 / n as f64).ceil() as usize;
        let mut removed = false;
        let mut i = 0;
        while i < cur.len() {
            let end = (i + chunk).min(cur.len());
            let candidate: Vec<T> = cur
                .iter()
                .enumerate()
                .filter(|(j, _)| *j < i || *j >= end)
                .map(|(_, a)| a.clone())
                .collect();
            if reproduce(&candidate) {
                cur = candidate;
                removed = true;
                // restart chunking from the shorter sequence
                break;
            }
            i += chunk;
        }
        if removed {
            n = (n - 1).max(2); // removal worked — try finer chunks
        } else {
            if n >= cur.len() {
                break; // 1-progress: try simpler minimality (pairwise removal below)
            }
            n = (n * 2).min(cur.len());
        }
    }
    // Final 1-minimality pass: try removing each single remaining action.
    loop {
        let mut single_removed = false;
        for i in 0..cur.len() {
            let candidate: Vec<T> = cur
                .iter()
                .enumerate()
                .filter(|(j, _)| *j != i)
                .map(|(_, a)| a.clone())
                .collect();
            if reproduce(&candidate) {
                cur = candidate;
                single_removed = true;
                break;
            }
        }
        if !single_removed {
            break;
        }
    }
    cur
}

/// Convert a report's action log to replayable timed actions.
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
    /// Select a tile/entity BY NAME — resolved against live `Name` +
    /// `Gameplay` entities at fire time. Unlike raw-entity Selects, this
    /// survives reset cycles (fresh entity ids) — the replay scenario
    /// refers to stable scene names.
    Select {
        target: String,
    },
    Wait,
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
    /// Expert-rule oracle (TITAN): WHEN `path` satisfies `check` vs
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

#[derive(Debug, Clone, PartialEq)]
pub enum ScenarioError {
    Rejected(String),
}

impl std::fmt::Display for ScenarioError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ScenarioError::Rejected(msg) => write!(f, "scenario rejected: {}", msg),
        }
    }
}

impl std::error::Error for ScenarioError {}

const KNOWN_BOT_TYPES: [crate::enums::BotType; 6] = [
    crate::enums::BotType::Chaos,
    crate::enums::BotType::Pursuit,
    crate::enums::BotType::Replay,
    crate::enums::BotType::SyntheticPointer,
    crate::enums::BotType::SyntheticKeyboard,
    crate::enums::BotType::Planner,
];

pub fn validate_scenario(scenario: &Scenario) -> Result<(), ScenarioError> {
    // Bot type validity is enforced by the BotType enum deserialization
    // itself — unknown strings fail at parse time.
    if !KNOWN_BOT_TYPES.contains(&scenario.bot.bot_type) {
        return Err(ScenarioError::Rejected(format!(
            "unknown bot type '{}' (expected one of: {})",
            scenario.bot.bot_type, scenario.bot.bot_type
        )));
    }
    for rule in &scenario.invariants {
        if rule.rule == crate::enums::InvariantRule::Custom {
            let check = rule.check.unwrap_or(crate::enums::CheckOp::Below);
            if check != crate::enums::CheckOp::Equals {
                if let Some(v) = &rule.value {
                    if !v.is_number() {
                        return Err(ScenarioError::Rejected(format!(
                            "invariant '{}' uses check='{}' with a non-numeric value ({}) — use check:'equals' or a numeric threshold",
                            rule.name, check, v
                        )));
                    }
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
// Violations — dedup by (rule, target), ordered by first_frame
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct ViolationEntry {
    pub rule: String,
    pub target: String,
    pub first_frame: u64,
    pub last_frame: u64,
    pub count: u64,
    pub detail: String,
}

#[derive(Resource, Clone, Debug, Default)]
pub struct Violations {
    pub entries: HashMap<(String, String), ViolationEntry>,
    /// Latest bot intent in flight (set by bot systems each fire, read
    /// when reporting) — a violation carries the input that produced it.
    context: String,
}

impl Violations {
    pub fn set_context(&mut self, ctx: impl Into<String>) {
        self.context = ctx.into();
    }

    pub fn report(&mut self, rule: &str, target: &str, detail: String, frame: u64) {
        let key = (rule.to_string(), target.to_string());
        let entry = self.entries.entry(key).or_insert(ViolationEntry {
            rule: rule.to_string(),
            target: target.to_string(),
            first_frame: frame,
            last_frame: frame,
            count: 0,
            detail: detail.clone(),
        });
        entry.count += 1;
        entry.last_frame = frame;
        entry.detail = format!("[intent: {}] {}", self.context, detail);
    }

    /// Snapshot sorted by first_frame — callers store reports across
    /// scenarios; never hand out the live map.
    pub fn snapshot(&self) -> Vec<ViolationEntry> {
        let mut v: Vec<ViolationEntry> = self.entries.values().cloned().collect();
        v.sort_by_key(|e| e.first_frame);
        v
    }
}

// ---------------------------------------------------------------------------
// Runtime state resources
// ---------------------------------------------------------------------------

#[derive(Resource)]
pub struct PlaytestState {
    pub frame: u64,
    pub tps: u64,
    pub rng: u64, // xorshift; deterministic from scenario seed
    pub metrics: Metrics,
    pub delta_windows: HashMap<String, Vec<(u64, f64)>>, // rule -> samples
    warned_paths: HashSet<String>,
    pub coverage: Coverage,
    /// Execution-time oracle state (Welford per-frame stats).
    pub frame_timing: FrameTimingStats,
    /// Differential invariant history: path → last observed numeric value.
    /// Used by no_decrease/no_increase checks.
    api_history: HashMap<String, f64>,
    /// Eventually-mode state: rule → (deadline_s, satisfied_at_frame).
    /// Tracks whether an eventually-rule has been satisfied; deadline
    /// expiration without satisfaction triggers a violation.
    eventually_state: HashMap<String, (f32, Option<u64>)>,
    /// synthetic_pointer bot: queued press/release halves of click
    /// gestures awaiting their scheduled frames.
    pending_gestures: std::collections::VecDeque<PendingGesture>,
    /// synthetic_pointer actionability gates: gestures whose click must
    /// be verified against the hover map one frame after release.
    pending_actionability_checks: std::collections::VecDeque<PendingGesture>,
    /// synthetic_keyboard bot: key names whose release is due next frame.
    pending_key_releases: Vec<(String, u64)>,
    /// Frozen-world oracle: consecutive frames with zero Gameplay-entity
    /// Transform mutations (see `frozen_world_oracle_system`). Reports a
    /// soft-lock suspicion after 2s of silence in real-time games.
    pub frozen_frames: u64,
    /// Readiness gate: frames spent waiting for `GameReady` before
    /// scenario duration began accruing (UE IsReady analog).
    pub pre_ready_frames: u64,
    /// Planner bot persistent goal stack (empty = inactive/complete).
    pub planner: crate::planner::PlannerStack,
}

#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct Metrics {
    pub input_count: u64,
    pub frame_count: u64,
    pub frame_ms_p99: f64,
    pub worst_frame_ms: f64,
    pub crash_detected: bool,
}

/// Coverage metrics: counts of intent variants exercised, TestApi paths
/// read, and reset kinds invoked. Exported in PlaytestReport for
/// incremental-test selection (cf. SMART's 94% branch coverage).
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct Coverage {
    /// Map of intent variant name → count emitted by bots
    pub intents_emitted: HashMap<String, u64>,
    /// Set of TestApi paths resolved by custom invariants
    pub test_api_paths_read: HashSet<String>,
    /// Set of reset hook kinds invoked during setup
    pub resets_invoked: HashSet<String>,
    /// For chaos bot: which SurfaceVariant indices were sampled
    pub chaos_surface_indices: HashSet<u64>,
}

/// Code-aware coverage (CA² pattern): which SCHEDULED SYSTEMS actually
/// executed during a scenario run. Bevy's schedules are inspectable —
/// each system's `last_run` tick advances when it runs, so comparing
/// pre/post-run snapshots gives exact execution sets without
/// instrumentation. Systems that never ran are coverage gaps the chaos
/// bot (or the game author) should target next.
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct SystemCoverage {
    /// Fully-qualified "schedule::system_name" for every system that
    /// EXECUTED during the run.
    pub executed: Vec<String>,
    /// Every system REGISTERED in the app's schedules (executed or not).
    pub registered: Vec<String>,
}

impl SystemCoverage {
    /// Registered-but-never-executed systems — the coverage gaps.
    pub fn unexecuted(&self) -> Vec<String> {
        let exec: std::collections::HashSet<&str> =
            self.executed.iter().map(|s| s.as_str()).collect();
        self.registered
            .iter()
            .filter(|s| !exec.contains(&s.as_str()))
            .cloned()
            .collect()
    }

    /// Fraction of registered systems that executed, 0.0..1.0.
    pub fn fraction(&self) -> f64 {
        if self.registered.is_empty() {
            return 1.0;
        }
        self.executed.len() as f64 / self.registered.len() as f64
    }
}

/// Snapshot every scheduled system's identity + last_run tick, keyed by
/// "schedule::system". Requires the schedules to be initialized
/// (app.finish() + app.cleanup() must have run) — the harness driver
/// ensures this before scenarios start.
pub fn snapshot_systems(world: &mut World) -> HashMap<String, u32> {
    let mut out = HashMap::new();
    // Take Schedules out of the world so each schedule can borrow the
    // world for lazy initialization (schedules init on first run).
    let Some(mut schedules) = world
        .remove_resource::<bevy::ecs::schedule::Schedules>()
        .map(|mut s| std::mem::take(&mut s))
    else {
        return out;
    };
    // Store each system's AGE: how far its last_run tick lags the world
    // tick, as a wrapping distance. Raw ticks wrap and get clamped by
    // check_change_ticks, so absolute comparisons lie; a shrinking age
    // ("ran more recently") is wrap-safe.
    let world_tick = world.change_tick().get();
    for (label, sched) in schedules.iter_mut() {
        let key = format!("{:?}", label);
        let _ = sched.initialize(world);
        if let Ok(systems) = sched.systems() {
            for (_id, system) in systems {
                let name = format!("{}::{}", key, system.name());
                let age = world_tick.wrapping_sub(system.get_last_run().get());
                out.insert(name, age);
            }
        }
    }
    world.insert_resource(schedules);
    out
}

/// Compute code-aware system coverage for a completed run: executed =
/// systems whose last_run tick ADVANCED past the pre-run snapshot.
/// A `before` snapshot of None-entry means the system was added
/// mid-run (still counts as executed).
pub fn system_coverage(before: &HashMap<String, u32>, world: &mut World) -> SystemCoverage {
    let after = snapshot_systems(world);
    let mut cov = SystemCoverage {
        registered: after.keys().cloned().collect(),
        ..Default::default()
    };
    for (name, age) in &after {
        match before.get(name) {
            // Same-or-older age = did not run during the scenario.
            Some(prev) if age >= prev => {}
            _ => cov.executed.push(name.clone()),
        }
    }
    cov.registered.sort();
    cov.executed.sort();
    cov
}

impl PlaytestState {
    /// Construct a fresh playtest state for a run at `tps` ticks per
    /// second with the given RNG seed. Public entry point for callers
    /// driving the harness manually (e.g. calibration runs).
    pub fn new(tps: u64, seed: u64) -> Self {
        PlaytestState {
            frame: 0,
            tps,
            rng: if seed == 0 { 42 } else { seed },
            metrics: Metrics::default(),
            delta_windows: HashMap::default(),
            warned_paths: HashSet::default(),
            coverage: Coverage::default(),
            frame_timing: FrameTimingStats::default(),
            api_history: HashMap::default(),
            eventually_state: HashMap::default(),
            frozen_frames: 0,
            pre_ready_frames: 0,
            planner: crate::planner::PlannerStack::default(),
            pending_gestures: std::collections::VecDeque::new(),
            pending_actionability_checks: std::collections::VecDeque::new(),
            pending_key_releases: Vec::new(),
        }
    }

    pub fn next_rand(&mut self) -> u64 {
        // xorshift64* — deterministic chaos bot from scenario seed
        let mut x = self.rng;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.rng = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }

    pub fn elapsed_s(&self) -> f32 {
        self.frame as f32 / self.tps.max(1) as f32
    }
}

// ---------------------------------------------------------------------------
// Bots — all emit UserIntent (contract-aware, genre-general)
// ---------------------------------------------------------------------------

/// chaos: samples a random intent variant FROM THE GAME'S DECLARED
/// `IntentSurface` each fire interval. The harness holds zero genre
/// knowledge — a dialogue game gets random Choices (within its declared
/// range), a strategy game gets random Selects/Chooses, a racing game gets
/// declared axes. An empty/missing surface is a contract violation
/// (loud), not a silent no-op — chaos refuses to issue input it can't
/// know the game consumes. `Select` targets are drawn from live
/// `Gameplay` entities (a placeholder entity would be ignored-or-panic
/// noise, not a test). `SurfaceVariant::Axis(name)` counts as variant
/// `"axis:<name>"` for coverage tracking.
fn chaos_bot_system(
    mut state: ResMut<PlaytestState>,
    mut intents: MessageWriter<UserIntent>,
    mut violations: ResMut<Violations>,
    surface: Option<Res<IntentSurface>>,
    q: Query<Entity, With<Gameplay>>,
    scenario: Res<ScenarioResource>,
) {
    if scenario.0.bot.bot_type != crate::enums::BotType::Chaos {
        return;
    }
    // Persona bias (MIMIC): aggressive = 2x rate + never Wait; curious =
    // strong preference for unseen variants; idle = mostly Wait with rare
    // jabs. Different personas find different bugs.
    let persona = scenario.0.bot.persona.unwrap_or(crate::enums::Persona::Curious);
    let rate = scenario.0.bot.input_rate_hz.max(1);
    let effective_rate = if persona == crate::enums::Persona::Aggressive {
        rate.saturating_mul(2)
    } else {
        rate
    };
    let fire_every = (state.tps / effective_rate as u64).max(1);
    if !state.frame.is_multiple_of(fire_every) {
        return;
    }
    // Idle persona: skip 7 of 8 fire slots entirely (occasional jabs).
    if persona == crate::enums::Persona::Idle && !state.next_rand().is_multiple_of(8) {
        return;
    }
    let Some(surface) = surface else {
        violations.report(
            "chaos_bot_config",
            "",
            "game does not implement testable-conventions: IntentSurface resource missing — chaos cannot know which intents the game consumes"
                .to_string(),
            state.frame,
        );
        return;
    };
    if surface.0.is_empty() {
        violations.report(
            "chaos_bot_config",
            "",
            "IntentSurface is empty — the game declared no intent variants. Declare the game's input surface (IntentSurface::new(...))"
                .to_string(),
            state.frame,
        );
        return;
    }
    let mut roll = state.next_rand() % surface.0.len() as u64;
    // Aggressive persona: never Wait (loop — a single reroll can land
    // on Wait again when the surface is small).
    if persona == crate::enums::Persona::Aggressive {
        while matches!(&surface.0[roll as usize], SurfaceVariant::Wait) {
            roll = state.next_rand() % surface.0.len() as u64;
        }
    }
    let variant_name: String;
    let intent = match &surface.0[roll as usize] {
        SurfaceVariant::Move => {
            variant_name = "move".into();
            UserIntent::Move {
                dir: bevy::math::Vec2::new(
                    (state.next_rand() % 100) as f32 / 50.0 - 1.0,
                    (state.next_rand() % 100) as f32 / 50.0 - 1.0,
                ),
            }
        }
        SurfaceVariant::Choice(max_index) => {
            variant_name = format!("choice:max={}", max_index);
            UserIntent::Choice {
                index: (state.next_rand() % (*max_index as u64 + 1)) as usize,
            }
        }
        SurfaceVariant::Axis(name) => {
            variant_name = format!("axis:{}", name);
            UserIntent::Axis {
                name: name.clone(),
                value: (state.next_rand() % 100) as f32 / 50.0 - 1.0,
            }
        }
        SurfaceVariant::Select => {
            variant_name = "select".into();
            // Draw from live Gameplay entities; zero entities = surface
            // declaration mismatch (declared Select but nothing selectable)
            let candidates: Vec<Entity> = q.iter().collect();
            if candidates.is_empty() {
                violations.report(
                    "chaos_bot_config",
                    "",
                    "SurfaceVariant::Select declared but no Gameplay entities exist to select from"
                        .to_string(),
                    state.frame,
                );
                return;
            }
            let idx = (state.next_rand() % candidates.len() as u64) as usize;
            UserIntent::Select {
                target: candidates[idx],
            }
        }
        SurfaceVariant::Wait => {
            variant_name = "wait".into();
            UserIntent::Wait
        }
    };
    // Coverage: track which variant was sampled
    state
        .coverage
        .intents_emitted
        .entry(variant_name.clone())
        .and_modify(|c| *c += 1)
        .or_insert(1);
    state.coverage.chaos_surface_indices.insert(roll);
    // Context for violations
    violations.set_context(format!("chaos:{},idx={}", variant_name, roll));
    intents.write(intent);
    state.metrics.input_count += 1;
}

/// replay: frame-indexed typed intents — genre-free. Tracks coverage
/// of each intent type replayed.
fn replay_bot_system(
    mut state: ResMut<PlaytestState>,
    mut intents: MessageWriter<UserIntent>,
    mut violations: ResMut<Violations>,
    q_named: Query<(bevy::ecs::entity::Entity, &Name), With<Gameplay>>,
    scenario: Res<ScenarioResource>,
) {
    if scenario.0.bot.bot_type != crate::enums::BotType::Replay {
        return;
    }
    for entry in &scenario.0.bot.inputs {
        if entry.frame == state.frame {
            let variant_name: String;
            let intent = match &entry.intent {
                ReplayIntent::Move { dir } => {
                    variant_name = format!("move:x={:.2},y={:.2}", dir.0, dir.1);
                    UserIntent::Move {
                        dir: bevy::math::Vec2::new(dir.0, dir.1),
                    }
                }
                ReplayIntent::Choice { index } => {
                    variant_name = format!("choice:idx={}", index);
                    UserIntent::Choice { index: *index }
                }
                ReplayIntent::Axis { name, value } => {
                    variant_name = format!("axis:{}={:.2}", name, value);
                    UserIntent::Axis {
                        name: name.clone(),
                        value: *value,
                    }
                }
                ReplayIntent::Wait => {
                    variant_name = "wait".into();
                    UserIntent::Wait
                }
                ReplayIntent::Select { target } => {
                    variant_name = format!("select:name={}", target);
                    // Resolve by Name against live Gameplay entities —
                    // stable across resets, unlike raw entity ids.
                    match q_named.iter().find(|(_, n)| n.as_str() == target) {
                        Some((entity, _)) => UserIntent::Select { target: entity },
                        None => {
                            violations.set_context(format!(
                                "replay:frame={},select-target-miss={}",
                                entry.frame, target
                            ));
                            continue;
                        }
                    }
                }
            };
            state
                .coverage
                .intents_emitted
                .entry(variant_name)
                .and_modify(|c| *c += 1)
                .or_insert(1);
            violations.set_context(format!(
                "replay:frame={},intent={:?}",
                entry.frame, entry.intent
            ));
            intents.write(intent);
        }
    }
}

/// One in-flight click gesture for the synthetic_pointer bot: press
/// and release frames scheduled relative to the initiating move.
#[derive(Clone)]
struct PendingGesture {
    press_at: u64,
    release_at: u64,
    location: bevy::picking::pointer::Location,
    button: PointerButton,
    target_name: String,
    require_actionable: bool,
}

/// synthetic_pointer: emits RAW `PointerInput` events (Move → Press →
/// Release) at a named entity's projected viewport position. Unlike
/// intent-injection bots (chaos/replay), this drives the game through
/// its real input chain: pointer input → picking backend raycast →
/// `PointerClick` message → the game's own input adapter → UserIntent.
/// A game whose picking wiring is broken (e.g. missing
/// MeshPickingPlugin) produces clicks that hit nothing — caught by the
/// scenario's invariants, not silently skipped.
fn synthetic_pointer_bot_system(
    mut state: ResMut<PlaytestState>,
    mut violations: ResMut<Violations>,
    mut pointer_inputs: MessageWriter<PointerInput>,
    q_named: Query<(&Name, &GlobalTransform), With<Gameplay>>,
    q_camera: Query<(&Camera, &GlobalTransform)>,
    primary_window: Query<Entity, With<bevy::window::PrimaryWindow>>,
    scenario: Res<ScenarioResource>,
) {
    if scenario.0.bot.bot_type != crate::enums::BotType::SyntheticPointer {
        return;
    }
    // Deliver pending gesture halves scheduled for this frame.
    let frame = state.frame;
    let mut still_pending = std::collections::VecDeque::new();
    while let Some(mut g) = state.pending_gestures.pop_front() {
        if g.press_at == frame {
            pointer_inputs.write(PointerInput::new(
                PointerId::Mouse,
                g.location.clone(),
                PointerAction::Press(g.button),
            ));
        }
        if g.release_at == frame {
            pointer_inputs.write(PointerInput::new(
                PointerId::Mouse,
                g.location.clone(),
                PointerAction::Release(g.button),
            ));
            // ACTIONABILITY GATE (opt-in): after the gesture the
            // picking backend must show the pointer over SOME entity —
            // proof the click went through live picking wiring, not
            // into the void. Checked one frame AFTER release (hover
            // maps lag input by a frame).
            if g.require_actionable {
                state.pending_actionability_checks.push_back(g.clone());
            }
            state
                .coverage
                .intents_emitted
                .entry(format!("pointer_click:{}", g.target_name))
                .and_modify(|c| *c += 1)
                .or_insert(1);
            violations.set_context(format!(
                "synthetic_pointer:frame={},target={}",
                frame, g.target_name
            ));
            continue; // gesture complete
        }
        // Not yet fully delivered — keep waiting. A gesture that has
        // already pressed but not released stays queued.
        if g.press_at > frame {
            g.press_at = frame; // avoid double-press on future loops
        }
        still_pending.push_back(g);
    }
    state.pending_gestures = still_pending;
    for click in &scenario.0.bot.pointer_clicks {
        if click.frame != state.frame {
            continue;
        }
        let button = match click.button.as_str() {
            "primary" => PointerButton::Primary,
            "secondary" => PointerButton::Secondary,
            "middle" => PointerButton::Middle,
            other => {
                violations.report(
                    "synthetic_pointer_config",
                    &click.target,
                    format!("unknown pointer button '{}'", other),
                    state.frame,
                );
                continue;
            }
        };
        let Some((_, transform)) = q_named.iter().find(|(n, _)| n.as_str() == click.target) else {
            violations.report(
                "synthetic_pointer_config",
                &click.target,
                "click target not found among Gameplay entities".to_string(),
                state.frame,
            );
            continue;
        };
        let Ok(window_entity) = primary_window.single() else {
            violations.report(
                "synthetic_pointer_config",
                &click.target,
                "no primary window entity — cannot synthesize pointer events".to_string(),
                state.frame,
            );
            continue;
        };
        // Project the entity's world position to viewport coords using
        // the first camera that can (2D boards typically have exactly one).
        let mut viewport_pos = None;
        for (camera, cam_transform) in q_camera.iter() {
            if let Ok(p) = camera.world_to_viewport(cam_transform, transform.translation()) {
                viewport_pos = Some(p);
                break;
            }
        }
        let Some(position) = viewport_pos else {
            violations.report(
                "synthetic_pointer_config",
                &click.target,
                "no camera could project the target to viewport coordinates".to_string(),
                state.frame,
            );
            continue;
        };
        let Some(target) =
            bevy::camera::RenderTarget::Window(bevy::window::WindowRef::Entity(window_entity))
                .normalize(Some(window_entity))
        else {
            violations.report(
                "synthetic_pointer_config",
                &click.target,
                "primary window render-target normalization failed".to_string(),
                state.frame,
            );
            continue;
        };
        let location = bevy::picking::pointer::Location { target, position };
        // A click is a THREE-FRAME gesture: Move, then Press, then Release.
        // Rationale: bevy_picking's release/click dispatch reads the
        // PREVIOUS frame's hover map (previous_hover_map), so a press
        // and release in the same frame as the move finds no prior
        // hover and silently drops the click. Spreading the gesture
        // across frames mirrors real mouse timing.
        pointer_inputs.write(PointerInput::new(
            PointerId::Mouse,
            location.clone(),
            PointerAction::Move {
                delta: bevy::math::Vec2::ZERO,
            },
        ));
        // Pending press/release delivered on subsequent frames via
        // SPBGesture state.
        let f = state.frame;
        state.pending_gestures.push_back(PendingGesture {
            press_at: f + 1,
            release_at: f + 1 + click.hold_frames.max(1),
            location,
            button,
            target_name: click.target.clone(),
            require_actionable: click.require_actionable,
        });
    }
}

/// pursuit: resolves agent/target by Name (Gameplay + Transform required),
/// emits Move intents toward the target with a deadzone. Fails LOUDLY —
/// a bot issuing NO input is itself a violation, not a silent fallback.
/// Spatial games only; that's inherent to pursuing.
fn pursuit_bot_system(
    state: ResMut<PlaytestState>,
    mut violations: ResMut<Violations>,
    mut intents: MessageWriter<UserIntent>,
    q: Query<(&Name, &Transform), With<Gameplay>>,
    scenario: Res<ScenarioResource>,
) {
    if scenario.0.bot.bot_type != crate::enums::BotType::Pursuit {
        return;
    }
    let bot = &scenario.0.bot;
    let (Some(agent_name), Some(target_name)) = (&bot.agent_target, &bot.target) else {
        violations.report(
            "pursuit_bot_config",
            "",
            "pursuit bot requires agent_target and target".to_string(),
            state.frame,
        );
        return;
    };
    let agent = q.iter().find(|(n, _)| n.as_str() == agent_name);
    let target = q.iter().find(|(n, _)| n.as_str() == target_name);
    let (Some((_, agent_t)), Some((_, target_t))) = (agent, target) else {
        violations.report(
            "pursuit_bot_config",
            agent_name,
            format!(
                "agent '{}' or target '{}' not found — bot issued NO input. Fix agent_target/target; do not treat this run as a valid pursuit exercise.",
                agent_name, target_name
            ),
            state.frame,
        );
        return;
    };

    let dx = target_t.translation.x - agent_t.translation.x;
    let dy = target_t.translation.y - agent_t.translation.y;
    if dx.abs() > bot.deadzone || dy.abs() > bot.deadzone {
        violations.set_context(format!(
            "pursuit:agent={} target={} dist=({:.1},{:.1})",
            agent_name, target_name, dx, dy
        ));
        intents.write(UserIntent::Move {
            dir: bevy::math::Vec2::new(dx, dy).normalize_or_zero(),
        });
    }
}

fn tick_counter_system(mut state: ResMut<PlaytestState>) {
    state.frame += 1;
    state.metrics.frame_count += 1;
}

// ---------------------------------------------------------------------------
// Invariants
// ---------------------------------------------------------------------------

fn check_finite_transforms_system(
    q: Query<(Entity, &Transform), Changed<Transform>>,
    mut violations: ResMut<Violations>,
    state: Res<PlaytestState>,
) {
    for (entity, transform) in q.iter() {
        let t = transform.translation;
        if !t.x.is_finite() || !t.y.is_finite() || !t.z.is_finite() {
            violations.report(
                "nodes_finite",
                &format!("entity:{}", entity),
                format!("non-finite translation ({}, {}, {})", t.x, t.y, t.z),
                state.frame,
            );
        }
    }
}

/// Shared bounds predicate: x/y always; z only when a z bound is declared
/// (the Godot original checked z for 3D nodes — don't silently drop it).
fn out_of_bounds(t: bevy::math::Vec3, inv: &Invariant) -> bool {
    let (min_x, max_x, min_y, max_y) = (
        inv.min_x.unwrap_or(-10000.0),
        inv.max_x.unwrap_or(10000.0),
        inv.min_y.unwrap_or(-10000.0),
        inv.max_y.unwrap_or(10000.0),
    );
    let xy_ok = t.x >= min_x && t.x <= max_x && t.y >= min_y && t.y <= max_y;
    let z_ok = if inv.min_z.is_some() || inv.max_z.is_some() {
        let lo = inv.min_z.unwrap_or(f32::MIN);
        let hi = inv.max_z.unwrap_or(f32::MAX);
        t.z >= lo && t.z <= hi
    } else {
        true
    };
    !(xy_ok && z_ok)
}

/// Bounds checking — two-tier: existence/aliveness of explicit targets is
/// checked EVERY tick (a despawned target is a violation the moment it
/// goes missing — `no_null_refs` semantics), but the numeric bounds
/// comparison only runs on `Changed<Transform>` (O(changes), not
/// O(all entities × ticks)). ECS change detection is what makes the
/// per-tick poll affordable.
#[allow(clippy::type_complexity)]
fn check_bounds_gameplay_system(
    q_changed: Query<(&Name, &Transform), (With<Gameplay>, Changed<Transform>)>,
    q_all: Query<(&Name, &Transform), With<Gameplay>>,
    mut violations: ResMut<Violations>,
    state: Res<PlaytestState>,
    scenario: Res<ScenarioResource>,
) {
    for inv in &scenario.0.invariants {
        if inv.rule != crate::enums::InvariantRule::NodesInBounds {
            continue;
        }
        if let Some(after) = inv.after_s {
            if state.elapsed_s() < after {
                continue;
            }
        }
        if let Some(before) = inv.before_s {
            if state.elapsed_s() >= before {
                continue;
            }
        }
        if inv.targets.is_empty() {
            // Default: gameplay entities only — structural/UI at origin
            // are legitimately positioned and would flood reports.
            // Only re-check entities whose Transform changed this tick.
            for (name, transform) in q_changed.iter() {
                if out_of_bounds(transform.translation, inv) {
                    violations.report(
                        &inv.name,
                        name.as_ref(),
                        format!(
                            "out of bounds ({}, {}, {})",
                            transform.translation.x,
                            transform.translation.y,
                            transform.translation.z
                        ),
                        state.frame,
                    );
                }
            }
        } else {
            // Explicit targets: existence is part of the contract —
            // a vanished target checks ZERO entities and would pass green.
            // Existence + bounds both checked here; existence every tick,
            // bounds only on change.
            for target in &inv.targets {
                let matched_changed: Vec<(&Name, &Transform)> = q_changed
                    .iter()
                    .filter(|(n, _)| n.as_str() == target)
                    .collect();
                let matched_all: Vec<(&Name, &Transform)> =
                    q_all.iter().filter(|(n, _)| n.as_str() == target).collect();
                if matched_all.is_empty() {
                    violations.report(
                        &inv.name,
                        target,
                        "target not found — expected entity missing from the scene (missing gameplay object?)"
                            .to_string(),
                        state.frame,
                    );
                }
                for (name, transform) in matched_changed {
                    if out_of_bounds(transform.translation, inv) {
                        violations.report(
                            &inv.name,
                            name.as_ref(),
                            format!(
                                "out of bounds ({}, {}, {})",
                                transform.translation.x,
                                transform.translation.y,
                                transform.translation.z
                            ),
                            state.frame,
                        );
                    }
                }
            }
        }
    }
}

fn check_frame_times_system(
    diagnostics: Option<Res<DiagnosticsStore>>,
    mut violations: ResMut<Violations>,
    mut state: ResMut<PlaytestState>,
    scenario: Res<ScenarioResource>,
) {
    let Some(diag) = diagnostics else { return };
    let elapsed_s = state.elapsed_s();
    for inv in &scenario.0.invariants {
        // Honor after_s for timing rules: cold-start frames (shader
        // compile, asset load, first-tick cache misses) routinely
        // spike 30-100ms and pollute cumulative averages. Scenarios
        // that care can skip the warmup window.
        if let Some(after) = inv.after_s {
            if elapsed_s < after {
                continue;
            }
        }
        match inv.rule {
            crate::enums::InvariantRule::FrameTimeP99Below => {
                // Headless: no renderer/shader-compile stalls — warm-up
                // discard skipped. Diagnostic::max() doesn't exist in
                // 0.20-rc; we use latest value as a conservative proxy
                // (NOT a true rolling p99 — see MAPPING-NOTES).
                let threshold = inv.value.as_ref().and_then(|v| v.as_f64()).unwrap_or(33.3);
                if let Some(ft) = diag.get(&FrameTimeDiagnosticsPlugin::FRAME_TIME) {
                    if let Some(val) = ft.value() {
                        if val > threshold {
                            state.metrics.frame_ms_p99 = val;
                            state.metrics.worst_frame_ms = state.metrics.worst_frame_ms.max(val);
                            violations.report(
                                &inv.name,
                                "",
                                format!(
                                    "frame time value = {:.2}ms (threshold: {:.2}ms)",
                                    val, threshold
                                ),
                                state.frame,
                            );
                        }
                    }
                }
            }
            crate::enums::InvariantRule::FpsFloor => {
                let min_fps = inv.value.as_ref().and_then(|v| v.as_f64()).unwrap_or(30.0);
                let threshold_ms = 1000.0 / min_fps;
                if let Some(ft) = diag.get(&FrameTimeDiagnosticsPlugin::FRAME_TIME) {
                    if let Some(avg) = ft.average() {
                        if avg > threshold_ms {
                            violations.report(
                                &inv.name,
                                "",
                                format!("avg frame time = {:.2}ms (min fps: {:.0})", avg, min_fps),
                                state.frame,
                            );
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Custom invariants — resolve TestApi paths (the contract read surface)
// ---------------------------------------------------------------------------

/// Resolve a "TestApi.<field>" path by delegating to the GAME's
/// `TestApi::resolve` — the harness holds no field-name knowledge. When a
/// game extends `TestApi`, it extends its own `resolve` implementation
/// (see testable-conventions template). Numeric fields feed below/above +
/// windowed max_delta_per_sec; text fields feed equals.
/// World-level reflection percept resolution — generic state discovery
/// without game-side boilerplate (the percept layer).
///
/// Supported path forms:
///   "Resource:<TypePath>.<field>"     — Resource field (single-instance)
///   "Component:<TypePath>#count"      — count of entities with component
///   "Component:<TypePath>[<idx>].<f>" — Nth entity's component field
///   "Component:<TypePath>.<field>"    — first entity's component field
///
/// Enums resolve to variant name as Text. Numeric leaf types → Numeric.
pub fn resolve_world_percept(world: &World, path: &str) -> Option<TestFieldValue> {
    use bevy::ecs::reflect::AppTypeRegistry;
    use bevy::reflect::GetPath;

    let registry = world.resource::<AppTypeRegistry>().0.read();

    // Helper: look up a registered type by short path and resolve its world
    // ComponentId (may not exist if never inserted as component/resource).
    let comp_id_for = |type_path: &str| -> Option<bevy::ecs::component::ComponentId> {
        let reg = registry.get_with_short_type_path(type_path)?;
        world.components().get_valid_id(reg.type_id())
    };

    // Resource: "Resource:<TypePath>.<field>" — resources live on singleton
    // entities; find it via resource_entities, then ReflectComponent.
    if let Some(rest) = path.strip_prefix("Resource:") {
        let dot_idx = rest.find('.')?;
        let type_path = &rest[..dot_idx];
        let field = &rest[dot_idx + 1..];
        let comp_id = comp_id_for(type_path)?;
        let reflect_component = registry
            .get_with_short_type_path(type_path)?
            .data::<bevy::ecs::reflect::ReflectComponent>()?;
        // Find the singleton entity holding this resource component.
        let resource_entity = world
            .resource_entities()
            .iter()
            .find(|(id, _)| *id == comp_id)
            .map(|(_, e)| e)?;
        let entity = world.get_entity(resource_entity).ok()?;
        let reflect = reflect_component.reflect(entity)?;
        let val = reflect.reflect_path(field).ok()?;
        return Some(value_from_reflect(val));
    }

    if let Some(rest) = path.strip_prefix("Component:") {
        // Count: "Component:<TypePath>#count"
        if let Some(type_path) = rest.strip_suffix("#count") {
            let comp_id = comp_id_for(type_path)?;
            let mut count = 0usize;
            for entity in world.iter_entities() {
                if entity.contains_id(comp_id) {
                    count += 1;
                }
            }
            return Some(TestFieldValue::Numeric(count as f64));
        }
        // Field: "Component:<TypePath>.<field>" or "Component:<TypePath>[<i>].<field>"
        let (type_path, idx, field) = if let Some(bracket_end) = rest.find('[') {
            let idx_start = bracket_end + 1;
            let idx_end = rest[idx_start..].find(']')? + idx_start;
            let idx: usize = rest[idx_start..idx_end].parse().ok()?;
            let field = rest.get(idx_end + 1..)?.strip_prefix('.')?;
            (&rest[..bracket_end], idx, field)
        } else {
            let dot_idx = rest.find('.')?;
            (&rest[..dot_idx], 0usize, &rest[dot_idx + 1..])
        };

        let comp_id = comp_id_for(type_path)?;
        let reflect_component = registry
            .get_with_short_type_path(type_path)?
            .data::<bevy::ecs::reflect::ReflectComponent>()?;
        let mut seen = 0usize;
        for entity in world.iter_entities() {
            if entity.contains_id(comp_id) {
                if seen == idx {
                    let reflect = reflect_component.reflect(entity)?;
                    let val = reflect.reflect_path(field).ok()?;
                    return Some(value_from_reflect(val));
                }
                seen += 1;
            }
        }
        return None;
    }

    None
}

/// Convert a reflected leaf value into a TestFieldValue. Numbers become
/// Numeric; enums become Text (variant name); strings become Text;
/// anything else falls back to the debug representation.
fn value_from_reflect(val: &dyn bevy::reflect::PartialReflect) -> TestFieldValue {
    use crate::contract::TestFieldValue;
    use bevy::reflect::ReflectKind;

    // Try numeric conversions
    if let Some(v) = val.try_downcast_ref::<i64>() {
        return TestFieldValue::Numeric(*v as f64);
    }
    if let Some(v) = val.try_downcast_ref::<i32>() {
        return TestFieldValue::Numeric(*v as f64);
    }
    if let Some(v) = val.try_downcast_ref::<u64>() {
        return TestFieldValue::Numeric(*v as f64);
    }
    if let Some(v) = val.try_downcast_ref::<u32>() {
        return TestFieldValue::Numeric(*v as f64);
    }
    if let Some(v) = val.try_downcast_ref::<f64>() {
        return TestFieldValue::Numeric(*v);
    }
    if let Some(v) = val.try_downcast_ref::<f32>() {
        return TestFieldValue::Numeric(*v as f64);
    }
    if let Some(v) = val.try_downcast_ref::<bool>() {
        return TestFieldValue::Numeric(if *v { 1.0 } else { 0.0 });
    }
    // Enum: stringify variant name
    if val.reflect_kind() == ReflectKind::Enum {
        if let bevy::reflect::ReflectRef::Enum(enum_val) = val.reflect_ref() {
            return TestFieldValue::Text(enum_val.variant_name().to_string());
        }
    }
    // String
    if let Some(s) = val.try_downcast_ref::<String>() {
        return TestFieldValue::Text(s.clone());
    }
    // Fallback: debug repr
    TestFieldValue::Text(format!("{val:?}"))
}

/// Count entities matching a QueryTarget (component filters). This is the
/// ECS-native analogue of Playwright's `toHaveCount` — query-shaped
/// assertions on entity archetypes. Uses reflection to resolve component
/// short type paths to ComponentIds, then iterates entities checking
/// presence/absence.
fn count_query_target(world: &World, target: &QueryTarget) -> Option<usize> {
    use bevy::ecs::reflect::AppTypeRegistry;

    let registry = world.resource::<AppTypeRegistry>().0.read();

    let resolve_ids = |names: &[String]| -> Option<Vec<bevy::ecs::component::ComponentId>> {
        names
            .iter()
            .map(|t| {
                let reg = registry.get_with_short_type_path(t)?;
                world.components().get_valid_id(reg.type_id())
            })
            .collect()
    };

    // Unresolvable component names → None (caller reports the typo).
    let with_ids = resolve_ids(&target.with)?;
    let without_ids = resolve_ids(&target.without)?;

    let mut count = 0;
    for entity in world.iter_entities() {
        if with_ids.iter().all(|id| entity.contains_id(*id))
            && without_ids.iter().all(|id| !entity.contains_id(*id))
        {
            count += 1;
        }
    }
    Some(count)
}

fn resolve_test_api(api: &TestApi, path: &str) -> (Option<f64>, Option<String>) {
    match api.resolve(path) {
        Some(TestFieldValue::Numeric(n)) => (Some(n), None),
        Some(TestFieldValue::Text(s)) => (None, Some(s)),
        None => (None, None),
    }
}

/// Exclusive system: snapshot scenario/TestApi state immutably first, then
/// mutate violations/delta-windows (two-phase to satisfy the borrow checker).
fn check_custom_system(world: &mut World) {
    let scenario = world.resource::<ScenarioResource>().0.clone();
    let frame = world.resource::<PlaytestState>().frame;
    let elapsed_s = world.resource::<PlaytestState>().elapsed_s();
    let tps = world.resource::<PlaytestState>().tps;
    let api = world.resource::<TestApi>().clone();

    for inv in &scenario.invariants {
        if inv.rule != crate::enums::InvariantRule::Custom {
            continue;
        }
        if let Some(after) = inv.after_s {
            if elapsed_s < after {
                continue;
            }
        }
        if let Some(before) = inv.before_s {
            if elapsed_s >= before {
                continue;
            }
        }

        // Query-target invariants: component-filtered entity counts.
        // ECS-native analogue of Playwright's `toHaveCount`.
        if let Some(query) = &inv.query {
            let Some(count) = count_query_target(world, query) else {
                world.resource_mut::<Violations>().report(
                    &inv.name,
                    &format!("query({:?})", query),
                    "unresolved component type in query (typo?)".into(),
                    frame,
                );
                continue;
            };
            let check = inv.check.unwrap_or(crate::enums::CheckOp::Above);
            let threshold = inv.value.as_ref().and_then(|v| v.as_f64());

            let holds_now = match (check, threshold) {
                (crate::enums::CheckOp::Equals, Some(thr)) => (count as f64 - thr).abs() <= f64::EPSILON,
                (crate::enums::CheckOp::Below, Some(thr)) => count as f64 <= thr,
                (crate::enums::CheckOp::Above, Some(thr)) => count as f64 >= thr,
                _ => false,
            };

            match inv.eventually_s {
                Some(deadline) => {
                    let mut state_mut = world.resource_mut::<PlaytestState>();
                    let entry = state_mut
                        .eventually_state
                        .entry(inv.name.clone())
                        .or_insert((deadline, None));
                    if holds_now {
                        if entry.1.is_none() {
                            entry.1 = Some(frame);
                        }
                } else if elapsed_s >= deadline && entry.1.is_none() {
                    world.resource_mut::<Violations>().report(
                        &inv.name,
                        &format!("query({:?})", query),
                            format!(
                                "query count {} never {} {} within {:.1}s (eventually deadline expired)",
                                count, check, threshold.unwrap_or(0.0), deadline
                            ),
                            frame,
                        );
                    }
                }
                None => {
                    if !holds_now {
                        world.resource_mut::<Violations>().report(
                            &inv.name,
                            &format!("query({:?})", query),
                            format!(
                                "query count {} {} {} (threshold: {})",
                                count,
                                if check == crate::enums::CheckOp::Equals {
                                    "!="
                                } else if check == crate::enums::CheckOp::Below {
                                    ">"
                                } else {
                                    "<"
                                },
                                threshold.unwrap_or(0.0),
                                threshold.unwrap_or(0.0)
                            ),
                            frame,
                        );
                    }
                }
            }
            continue;
        }

        let Some(path) = &inv.path else { continue };
        let (numeric, string_val) = match resolve_test_api(&api, path) {
            (Some(n), s) => (Some(n), s),
            (None, Some(s)) => (None, Some(s)),
            // TestApi miss → try world reflection percept (generic layer).
            (None, None) => match resolve_world_percept(world, path) {
                Some(TestFieldValue::Numeric(n)) => (Some(n), None),
                Some(TestFieldValue::Text(s)) => (None, Some(s)),
                None => (None, None),
            },
        };
        world
            .resource_mut::<PlaytestState>()
            .coverage
            .test_api_paths_read
            .insert(path.clone());
        if numeric.is_none() && string_val.is_none() {
            // Warn-once for unresolved paths (per-cycle would recreate
            // log spam). Unknown TestApi field = contract breach.
            if !world
                .resource::<PlaytestState>()
                .warned_paths
                .contains(path)
            {
                world
                    .resource_mut::<PlaytestState>()
                    .warned_paths
                    .insert(path.clone());
                world.resource_mut::<Violations>().report(
                    &inv.name,
                    path,
                    "TestApi field not found — extend TestApi or fix the invariant path"
                        .to_string(),
                    frame,
                );
            }
            continue;
        }

        // Equals on enum-ish (string) fields.
        if let (Some(sv), Some(serde_json::Value::String(expect))) = (&string_val, &inv.value) {
            let eq = sv == expect;
            // Eventually-mode: pass as soon as satisfied; report only at
            // deadline expiry if never satisfied.
            if let Some(deadline) = inv.eventually_s {
                let mut state_mut = world.resource_mut::<PlaytestState>();
                let entry = state_mut
                    .eventually_state
                    .entry(inv.name.clone())
                    .or_insert((deadline, None));
                if eq {
                    entry.1 = Some(entry.1.unwrap_or(frame)); // first satisfaction
                } else if elapsed_s >= deadline && entry.1.is_none() {
                    world.resource_mut::<Violations>().report(
                        &inv.name,
                        path,
                        format!(
                            "{} never equaled {} within {:.1}s (eventually deadline expired)",
                            path, expect, deadline
                        ),
                        frame,
                    );
                }
            } else if !eq {
                world.resource_mut::<Violations>().report(
                    &inv.name,
                    path,
                    format!("{} = {} (expected: {})", path, sv, expect),
                    frame,
                );
            }
            continue;
        }

        let Some(current) = numeric else { continue };
        let check = inv.check.unwrap_or(crate::enums::CheckOp::Below);
        let threshold = inv.value.as_ref().and_then(|v| v.as_f64());
        if let Some(thr) = threshold {
            // Shared predicate evaluation for both modes.
            let holds_now = if check == crate::enums::CheckOp::Equals {
                (current - thr).abs() <= f64::EPSILON
            } else if check == crate::enums::CheckOp::Below {
                current <= thr
            } else {
                current >= thr // "above"
            };
            match inv.eventually_s {
                // Eventually-mode: satisfied on first hold; report only at
                // deadline expiry if never held. Semantics mirror
                // Playwright's expect().toBeVisible(timeout) / gdUnit's
                // await_func().wait_until(ms).
                Some(deadline) => {
                    let mut state_mut = world.resource_mut::<PlaytestState>();
                    let entry = state_mut
                        .eventually_state
                        .entry(inv.name.clone())
                        .or_insert((deadline, None));
                    if holds_now {
                        if entry.1.is_none() {
                            entry.1 = Some(frame);
                        }
                    } else if elapsed_s >= deadline && entry.1.is_none() {
                        world.resource_mut::<Violations>().report(
                            &inv.name,
                            path,
                            format!(
                                "{} = {} never {} {} within {:.1}s (eventually deadline expired)",
                                path, current, check, thr, deadline
                            ),
                            frame,
                        );
                    }
                }
                None => {
                    // Always-mode: strict per-frame check.
                    if !holds_now {
                        world.resource_mut::<Violations>().report(
                            &inv.name,
                            path,
                            format!("{} = {} (threshold: {})", path, current, thr),
                            frame,
                        );
                    }
                }
            }
        }

        // Windowed rate-of-change: rate over ~1s of samples, not per-tick.
        // A discrete +1 event is a window rate of ~1/sec — legal under a
        // ceiling >= 1. A re-firing handler climbs the whole window.
        if let Some(max_dps) = inv.max_delta_per_sec {
            let mut state_mut = world.resource_mut::<PlaytestState>();
            let samples = state_mut.delta_windows.entry(inv.name.clone()).or_default();
            samples.push((frame, current));
            let window_ticks = tps.max(1);
            while samples.len() > 1 && frame - samples[0].0 > window_ticks {
                samples.remove(0);
            }
            let oldest = samples[0];
            let dt_ticks = frame - oldest.0;
            // Evaluate only once the window holds a full physics second —
            // shorter spans measure rates with warm-up skew.
            if dt_ticks >= window_ticks {
                let rate = (current - oldest.1).abs() / (dt_ticks as f64 / tps as f64);
                if rate > max_dps {
                    world.resource_mut::<Violations>().report(
                        &inv.name,
                        path,
                        format!(
                            "delta rate {:.1}/sec exceeds max_delta_per_sec {} (value {} -> {} over {} ticks)",
                            rate, max_dps, oldest.1, current, dt_ticks
                        ),
                        frame,
                    );
                }
            }
        }

        // Differential invariants: no_decrease (value must never drop) or
        // no_increase (value must never rise). Requires at least one prior
        // sample to compare against.
        if let Some(diff) = &inv.differential {
            let mut state_mut = world.resource_mut::<PlaytestState>();
            let prev = state_mut.api_history.insert(path.clone(), current);
            if let Some(prev_val) = prev {
                let violated = if diff == &crate::enums::DifferentialOp::NoDecrease {
                    current < prev_val
                } else {
                    current > prev_val // no_increase
                };
                if violated {
                    world.resource_mut::<Violations>().report(
                        &inv.name,
                        path,
                        format!(
                            "{}: {} changed from {} to {} (violates {})",
                            diff, path, prev_val, current, diff
                        ),
                        frame,
                    );
                }
            }
        }

        // Expert-rule oracle (TITAN): WHEN `path` satisfies check vs value,
        // REQUIRE `requires_path` to satisfy requires_check vs
        // requires_value. Checks game POLICY compliance ("low HP ⇒ must
        // heal"), complementing state invariants.
        if let (Some(req_path), Some(req_check), Some(req_value)) =
            (&inv.requires_path, &inv.requires_check, &inv.requires_value)
        {
            // WHEN guard: only fire when the condition holds. A missing
            // `check` means an unconditional policy rule.
            let when_holds = if let (Some(check), Some(when_val)) = (&inv.check, &inv.value) {
                if let (Some(cur), Some(thr)) = (numeric, when_val.as_f64()) {
                    match check {
                        crate::enums::CheckOp::Below => cur < thr,
                        crate::enums::CheckOp::Above => cur > thr,
                        crate::enums::CheckOp::Equals => (cur - thr).abs() <= f64::EPSILON,
                    }
                } else {
                    // Text WHEN: string equality against the field value.
                    string_val
                        .as_deref()
                        .and_then(|sv| when_val.as_str().map(|wv| sv == wv))
                        .unwrap_or(false)
                }
            } else {
                true
            };
            if when_holds {
                let (req_num, req_str) = resolve_test_api(&api, req_path);
                world
                    .resource_mut::<PlaytestState>()
                    .coverage
                    .test_api_paths_read
                    .insert(req_path.clone());
                let violated = match (req_num, req_value.as_f64()) {
                    (Some(v), Some(thr)) => match req_check {
                        crate::enums::CheckOp::Below => v > thr,
                        crate::enums::CheckOp::Above => v < thr,
                        crate::enums::CheckOp::Equals => (v - thr).abs() > f64::EPSILON,
                    },
                    _ => match (&req_str, req_value.as_str()) {
                        (Some(s), Some(exp)) => s != exp,
                        _ => false,
                    },
                };
                if violated {
                    let actual = req_num
                        .map(|n| n.to_string())
                        .or_else(|| req_str.clone())
                        .unwrap_or_else(|| "<unresolved>".into());
                    let when_desc = numeric
                        .map(|n| format!("{} = {}", path, n))
                        .or_else(|| string_val.clone().map(|s| format!("{} = {}", path, s)))
                        .unwrap_or_else(|| path.clone());
                    world.resource_mut::<Violations>().report(
                        &inv.name,
                        req_path,
                        format!(
                            "expert rule violated: when {}, {} = {} (required: {} {})",
                            when_desc, req_path, actual, req_check, req_value
                        ),
                        frame,
                    );
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Intent audit observer — optional, ECS-native (0.20 observer `Event`s)
// ---------------------------------------------------------------------------

/// Observer event: fired whenever a bot (or anything) writes a
/// `UserIntent`. Games/harness extensions can listen to audit intent
/// traffic without polling. The audit logger (below) counts every intent
/// by variant for coverage — ECS observers replace the per-bot manual
/// counting with a single centralized listener.
/// Centralized intent audit log: reads ALL UserIntent messages (fan-out —
/// buffered messages can have many readers) and records them in the action
/// log with full structured payloads (`intent:<variant>` + JSON details).
/// Single canonical AUDIT-LOG path: bots count coverage themselves (rich
/// variant names), but the audit trail — what makes crashes found by ANY
/// writer minimizable via ddmin — lives here and nowhere else.
fn intent_audit_log_system(
    state: ResMut<PlaytestState>,
    mut reader: bevy::ecs::message::MessageReader<UserIntent>,
    mut action_log: Option<ResMut<crate::contract::ActionLog>>,
) {
    for intent in reader.read() {
        let variant = match &intent {
            UserIntent::Move { .. } => "move",
            UserIntent::Choice { .. } => "choice",
            UserIntent::Axis { .. } => "axis",
            UserIntent::Select { .. } => "select",
            UserIntent::Wait => "wait",
        };
        let details = match &intent {
            UserIntent::Move { dir } => Some(format!(r#"{{"dir":[{:.4},{:.4}]}}"#, dir.x, dir.y)),
            UserIntent::Choice { index } => Some(format!(r#"{{"index":{}}}"#, index)),
            UserIntent::Axis { name, value } => {
                Some(format!(r#"{{"name":"{}","value":{:.4}}}"#, name, value))
            }
            UserIntent::Select { target } => Some(format!(r#"{{"target":"{}"}}"#, target.index())),
            UserIntent::Wait => None,
        };
        if let Some(log) = action_log.as_mut() {
            log.entries.push(crate::contract::ActionEntry {
                frame: state.frame,
                elapsed_ms: state.elapsed_s() as f64 * 1000.0,
                source: crate::contract::ActionSource::BotScenario("intent-audit".into()),
                action: format!("intent:{}", variant),
                details,
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Execution-time oracle (TITAN-inspired): per-FRAME timing anomaly
// detection via Welford's online algorithm. Flags frames slower than
// mean + 3σ after warm-up — catches infinite-loop-in-system,
// progressive degradation (resource leak), and stalls the static
// frame_time_p99_below threshold misses. Zero game knowledge: measures
// the update loop itself.
// ---------------------------------------------------------------------------

const FRAME_TIME_ANOMALY_MIN_SAMPLES: u64 = 60; // one second @ 60 TPS
const FRAME_TIME_ANOMALY_SIGMA: f64 = 3.0;
/// Relative-deviation gate: an anomalous frame must ALSO be this factor
/// above the mean. Guards against zero-variance histories (perfectly
/// steady frames would make 3σ hair-trigger) and ties the oracle to
/// human-meaningful slowdowns, not microsecond jitter.
const FRAME_TIME_ANOMALY_RELATIVE: f64 = 1.5;

/// Welford online stats for frame durations (ms), plus a bounded ring
/// buffer of recent samples backing a TRUE rolling p99 (nearest-rank).
#[derive(Clone, Debug)]
pub struct FrameTimingStats {
    mean_ms: f64,
    m2: f64,
    count: u64,
    /// Worst observed frame (ms)
    pub worst_ms: f64,
    /// Number of anomaly verdicts (each logged frame is one tick —
    /// dedup happens via Violations).
    pub anomalies: u64,
    /// Ring buffer of the last `ring_cap()` frame samples (ms),
    /// for rolling-quantile estimates.
    ring: std::collections::VecDeque<f64>,
}

const FRAME_TIMING_RING_CAP: usize = 600; // ten seconds @ 60 TPS

impl Default for FrameTimingStats {
    fn default() -> Self {
        Self {
            mean_ms: 0.0,
            m2: 0.0,
            count: 0,
            worst_ms: 0.0,
            anomalies: 0,
            ring: std::collections::VecDeque::with_capacity(FRAME_TIMING_RING_CAP),
        }
    }
}

impl FrameTimingStats {
    pub fn observe(&mut self, ms: f64) {
        self.count += 1;
        let delta = ms - self.mean_ms;
        self.mean_ms += delta / self.count as f64;
        self.m2 += delta * (ms - self.mean_ms);
        self.worst_ms = self.worst_ms.max(ms);
        if self.ring.len() == FRAME_TIMING_RING_CAP {
            self.ring.pop_front();
        }
        self.ring.push_back(ms);
    }

    /// TRUE rolling p99 (nearest-rank) over the last `ring_cap()`
    /// samples. Requires at least 100 samples; below that, returns
    /// None (a p99 of a handful of samples is noise).
    pub fn p99_ms(&self) -> Option<f64> {
        if self.ring.len() < 100 {
            return None;
        }
        let mut sorted: Vec<f64> = self.ring.iter().copied().collect();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let rank = ((sorted.len() as f64) * 0.99).ceil() as usize;
        sorted.get(rank.saturating_sub(1)).copied()
    }

    pub fn is_anomalous(&self, ms: f64) -> bool {
        if self.count < FRAME_TIME_ANOMALY_MIN_SAMPLES {
            return false;
        }
        let stddev = (self.m2 / self.count as f64).sqrt();
        let above_sigma = ms > self.mean_ms + FRAME_TIME_ANOMALY_SIGMA * stddev;
        let above_relative = ms > self.mean_ms * FRAME_TIME_ANOMALY_RELATIVE;
        above_sigma && above_relative
    }

    pub fn mean_ms(&self) -> f64 {
        self.mean_ms
    }
}

/// Per-frame timing oracle: observe each frame's wall duration and flag
/// statistical outliers. Uses `Time` delta (headless-safe: MinimalPlugins
/// pumps Time; delta reflects real update-loop cost in headless runs).
/// OPT-IN via an invariant with rule "frame_time_anomaly" — frame-time
/// jitter from scheduler/OS noise under test runners is normal; only a
/// scenario author who knows the game's profile should arm this oracle.
fn check_frame_time_anomaly_system(
    time: Option<Res<bevy::time::Time>>,
    mut state: ResMut<PlaytestState>,
    mut violations: ResMut<Violations>,
    scenario: Res<ScenarioResource>,
) {
    let Some(time) = time else { return };
    if !scenario
        .0
        .invariants
        .iter()
        .any(|inv| inv.rule == crate::enums::InvariantRule::FrameTimeAnomaly)
    {
        return;
    }
    let ms = time.delta_secs_f64() * 1000.0;
    if state.frame_timing.is_anomalous(ms) {
        state.frame_timing.anomalies += 1;
        violations.report(
            "frame_time_anomaly",
            "",
            format!(
                "frame took {:.2}ms — {:.1}σ above running mean ({:.2}ms, worst {:.2}ms) — possible stall, loop, or leak",
                ms,
                FRAME_TIME_ANOMALY_SIGMA,
                state.frame_timing.mean_ms(),
                state.frame_timing.worst_ms
            ),
            state.frame,
        );
    }
    state.frame_timing.observe(ms);
}

// ---------------------------------------------------------------------------
// PlaytestPlugin
// ---------------------------------------------------------------------------

#[derive(Resource)]
pub struct ScenarioResource(pub Scenario);

pub struct PlaytestPlugin;

impl bevy::app::Plugin for PlaytestPlugin {
    fn build(&self, app: &mut App) {
        // PointerInput registration: headless test apps may lack DefaultPlugins,
        // so ensure the synthetic pointer bot can always write. Idempotent.
        app.add_message::<PointerInput>();
        app.init_resource::<Violations>().add_systems(
            bevy::app::Update,
            (
                tick_counter_system,
                chaos_bot_system,
                replay_bot_system,
                pursuit_bot_system,
                crate::planner::planner_bot_system,
                frozen_world_oracle_system,
                check_finite_transforms_system,
                check_bounds_gameplay_system,
                check_frame_times_system,
                check_frame_time_anomaly_system,
            ),
        );
        // Exclusive system: reads TestApi with &mut World access.
        app.add_systems(bevy::app::Last, check_custom_system);
        // Canonical audit-log path: reads all UserIntent messages.
        app.add_systems(bevy::app::Last, intent_audit_log_system);
    }
}

// ---------------------------------------------------------------------------
// Test driver — pump the loop, catch panics, produce the report
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Serialize)]
pub struct PlaytestReport {
    pub status: crate::enums::PlaytestStatus,
    pub violations: Vec<ViolationEntry>,
    pub metrics: Metrics,
    pub coverage: Coverage,
    pub frame_count: u64,
    pub error: Option<String>,
    /// AUDIT (phase 3): every injected input and cheat invocation,
    /// labeled by source. Full replay material for the run.
    pub action_log: Vec<crate::contract::ActionEntry>,
    /// AUDIT: game build identity stamped into the report.
    pub game_version: Option<String>,
    /// AUDIT: cheats detected in this run (subset of action_log, both
    /// counted and listed so reports can flag cheated runs cheaply).
    pub cheat_count: usize,
    /// If the run CRASHED and minimization was requested via
    /// [`MinimizeOnCrash`], the ddmin-minimal reproducing action
    /// subsequence — a permanent regression scenario's raw material.
    pub minimized_actions: Vec<TimedAction>,
    /// Code-aware coverage (CA²): which scheduled systems executed
    /// during the run, vs every registered system. Unexecuted systems
    /// are coverage gaps to target with new scenarios.
    pub system_coverage: SystemCoverage,
}

/// Insert this resource before `run_scenario` to enable automatic
/// ddmin minimization when the run crashes. The minimized action
/// sequence lands in `PlaytestReport.minimized_actions`.
#[derive(Resource, Clone, Debug)]
pub struct MinimizeOnCrash;

/// Run a validated scenario against an already-built headless App.
///
/// Contract requirements, enforced loudly (a missing contract piece is a
/// REJECTED scenario, never a vacuous pass):
/// - `ResetHooks` resource must exist; unknown reset kinds are rejected
///   listing the known kinds.
/// - `TestApi` must exist (custom invariants read it; missing TestApi
///   would silently skip every check).
/// - `UserIntent` events must be registered (bots write them).
///
/// Panics are caught: a panic is a violation (no_fatal_errors mapping)
/// and ends the run with status "crash".
pub fn run_scenario(app: &mut App, scenario: &Scenario) -> Result<PlaytestReport, ScenarioError> {
    validate_scenario(scenario)?;

    if app.world().get_resource::<TestApi>().is_none() {
        return Err(ScenarioError::Rejected(
            "game does not implement testable-conventions: TestApi resource missing — custom invariants cannot be evaluated"
                .to_string(),
        ));
    }

    // ResetHooks: required only if the scenario declares resets — a game
    // with no persistent state may legitimately register none.
    let known_kinds: Vec<String> = match app.world().get_resource::<ResetHooks>() {
        Some(hooks) => hooks.0.keys().cloned().collect(),
        None => Vec::new(),
    };
    for reset in &scenario.setup.resets {
        if !known_kinds.contains(&reset.kind) {
            return Err(ScenarioError::Rejected(format!(
                "unknown reset kind '{}' — known kinds: {}",
                reset.kind,
                if known_kinds.is_empty() {
                    "(none registered)".to_string()
                } else {
                    known_kinds.join(", ")
                }
            )));
        }
    }

    // Validate scheduled cheats (reject unknown kinds — never run).
    let known_cheats = match app.world().get_resource::<crate::contract::CheatHooks>() {
        Some(h) => h.known_kinds(),
        None => vec![],
    };
    for cheat in &scenario.setup.cheats {
        if !known_cheats.contains(&cheat.kind.as_str()) {
            return Err(ScenarioError::Rejected(format!(
                "unknown cheat kind '{}' — known kinds: {}",
                cheat.kind,
                if known_cheats.is_empty() {
                    "(none registered)".to_string()
                } else {
                    known_cheats.join(", ")
                }
            )));
        }
    }

    let tps = 60u64; // Default physics ticks/sec. Games with different tick rates
                     // (e.g., turn-based at 10 TPS) must configure their App's
                     // ScheduleRunnerPlugin accordingly; the harness reads the
                     // configured TPS from PlaytestState, not this default.
    app.insert_resource(ScenarioResource(scenario.clone()));

    // Seed propagation: games with randomness read this resource to seed
    // their generators, making chaos runs reproducible.
    app.insert_resource(crate::contract::ScenarioSeed(scenario.bot.seed));

    // AUDIT (phase 3): scheduled cheats fire on their due frames,
    // logged with source BotScenario("cheat-schedule").
    app.add_systems(bevy::app::Update, cheat_scheduler_system);

    /// synthetic_keyboard: emits RAW `KeyboardInput` messages (press on
    /// the due frame, release the next) — the exact chain winit drives:
    /// KeyboardInput → ButtonInput<KeyCode> → the game's input adapter.
    /// An adapter that's unregistered or maps the wrong key produces NO
    /// intents — caught by the scenario's TestApi invariants.
    /// AUDIT (phase 3): fires scenario-scheduled cheats on their due
    /// frames. Every invocation is action-logged (cheat:<kind>) — cheat
    /// use is never silent.
    fn cheat_scheduler_system(world: &mut World) {
        let frame = world.resource::<PlaytestState>().frame;
        let elapsed_ms = world.resource::<PlaytestState>().elapsed_s() as f64 * 1000.0;
        let due: Vec<String> = world
            .resource::<ScenarioResource>()
            .0
            .setup
            .cheats
            .iter()
            .filter(|c| c.frame == frame)
            .map(|c| c.kind.clone())
            .collect();
        for kind in due {
            // AUDIT first — cheat use is never silent.
            world.resource_mut::<crate::contract::ActionLog>().record(
                frame,
                elapsed_ms,
                crate::contract::ActionSource::BotScenario("cheat-schedule".into()),
                format!("cheat:{}", kind),
                None,
            );
            // Execute: hooks aren't Clone — remove resource, extract, run,
            // restore.
            let Some(mut hooks) = world.remove_resource::<crate::contract::CheatHooks>() else {
                continue;
            };
            if let Some(hook) = hooks.0.remove(&kind) {
                hook(world);
            }
            world.insert_resource(hooks);
        }
    }

    fn synthetic_keyboard_bot_system(
        mut state: ResMut<PlaytestState>,
        mut violations: ResMut<Violations>,
        mut keyboard_inputs: MessageWriter<bevy::input::keyboard::KeyboardInput>,
        primary_window: Query<Entity, With<bevy::window::PrimaryWindow>>,
        scenario: Res<ScenarioResource>,
    ) {
        if scenario.0.bot.bot_type != crate::enums::BotType::SyntheticKeyboard {
            return;
        }
        let frame = state.frame;
        // Entity is not Default; PLACEHOLDER satisfies the field when the
        // headless app somehow lacks a window (KeyboardInput window only
        // routes text input, not key state).
        let window = match primary_window.single() {
            Ok(e) => e,
            Err(_) => bevy::ecs::entity::Entity::PLACEHOLDER,
        };
        // Releases due this frame.
        let pending = std::mem::take(&mut state.pending_key_releases);
        for (key, release_at) in pending {
            if frame >= release_at {
                if let Some(code) = parse_key_code(&key) {
                    keyboard_inputs.write(bevy::input::keyboard::KeyboardInput {
                        key_code: code,
                        logical_key: bevy::input::keyboard::Key::Unidentified(
                            bevy::input::keyboard::NativeKey::Unidentified,
                        ),
                        state: bevy::input::ButtonState::Released,
                        text: None,
                        repeat: false,
                        window,
                    });
                    state
                        .coverage
                        .intents_emitted
                        .entry(format!("key:{}", key))
                        .and_modify(|c| *c += 1)
                        .or_insert(1);
                    violations
                        .set_context(format!("synthetic_keyboard:frame={},key={}", frame, key));
                }
            } else {
                state.pending_key_releases.push((key, release_at));
            }
        }
        // Presses due this frame.
        for kp in &scenario.0.bot.key_presses {
            if kp.frame != frame {
                continue;
            }
            match parse_key_code(&kp.key) {
                Some(code) => {
                    keyboard_inputs.write(bevy::input::keyboard::KeyboardInput {
                        key_code: code,
                        logical_key: bevy::input::keyboard::Key::Unidentified(
                            bevy::input::keyboard::NativeKey::Unidentified,
                        ),
                        state: bevy::input::ButtonState::Pressed,
                        text: None,
                        repeat: false,
                        window,
                    });
                    state
                        .pending_key_releases
                        .push((kp.key.clone(), frame + kp.hold_frames));
                }
                None => {
                    violations.report(
                        "synthetic_keyboard_config",
                        &kp.key,
                        "unknown key name (see KeyPressInput docs for supported names)".to_string(),
                        frame,
                    );
                }
            }
        }
    }

    // Register the synthetic_pointer bot only when the scenario asks for
    // it: its ResMut<PlaytestState>/ResMut<Violations> params would add
    // scheduler edges that perturb system ordering for OTHER bot types     // registration keeps the default schedule graph byte-identical.
    if scenario.bot.bot_type == crate::enums::BotType::SyntheticPointer {
        app.add_systems(bevy::app::Update, synthetic_pointer_bot_system);
        // Actionability gate queue processor (only meaningful when the
        // scenario opts in via require_actionable clicks).
        app.add_systems(
            bevy::app::Update,
            synthetic_pointer_actionability_check_system,
        );
    }
    if scenario.bot.bot_type == crate::enums::BotType::SyntheticKeyboard {
        app.add_systems(bevy::app::Update, synthetic_keyboard_bot_system);
    }
    // Apply resets once, before the loop (typed registry, no dispatch).
    // Coverage: record each reset kind invoked.
    let mut coverage = Coverage::default();
    // Take ResetHooks out of the world (owned, no borrow held), invoke
    // each hook with full world access, then restore the registry —
    // calling while holding the resource borrow is a double-borrow error.
    let kinds: Vec<String> = scenario
        .setup
        .resets
        .iter()
        .map(|r| r.kind.clone())
        .collect();
    for k in &kinds {
        coverage.resets_invoked.insert(k.clone());
    }
    let hooks = app
        .world_mut()
        .remove_resource::<ResetHooks>()
        .expect("presence checked above");
    for k in &kinds {
        if let Some(hook) = hooks.0.get(k) {
            hook(app.world_mut());
        }
    }
    app.insert_resource(hooks);

    app.insert_resource(PlaytestState {
        frame: 0,
        tps,
        rng: if scenario.bot.seed == 0 {
            42
        } else {
            scenario.bot.seed
        },
        metrics: Metrics::default(),
        delta_windows: HashMap::default(),
        warned_paths: HashSet::default(),
        coverage,
        frame_timing: FrameTimingStats::default(),
        api_history: HashMap::default(),
        eventually_state: HashMap::default(),
        frozen_frames: 0,
        pre_ready_frames: 0,
        planner: crate::planner::PlannerStack::default(),
        pending_gestures: std::collections::VecDeque::new(),
        pending_actionability_checks: std::collections::VecDeque::new(),
        pending_key_releases: Vec::new(),
    });

    let total_ticks = (scenario.duration_s * tps as f32) as u64;
    // Pre-run snapshot for code-aware (system) coverage — see
    // `system_coverage`. Must happen after finish/cleanup so schedules
    // are initialized and enumerable.
    let systems_before = snapshot_systems(app.world_mut());

    // Readiness gate (UE IsReady analog): wait for GameReady=true before
    // counting scenario duration. Games that never insert GameReady get
    // immediate readiness (frame-0 behavior preserved).
    let mut pre_ready_frames = 0u64;
    while app
        .world_mut()
        .get_resource::<GameReady>()
        .is_some_and(|r| !r.0)
    {
        app.update();
        pre_ready_frames += 1;
        if pre_ready_frames > tps * 30 {
            let mut violations = app
                .world_mut()
                .get_resource_mut::<Violations>()
                .expect("Violations initialized by plugin");
            violations.report(
                "readiness_timeout",
                "world",
                "game did not become ready within 30s (GameReady never set true)".into(),
                0,
            );
            break;
        }
    }

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        for _ in 0..total_ticks {
            app.update();
        }
    }));

    let state = app
        .world_mut()
        .remove_resource::<PlaytestState>()
        .expect("PlaytestState inserted above");
    let violations = app
        .world_mut()
        .get_resource::<Violations>()
        .cloned()
        .unwrap_or_default();

    // Stamp the readiness-gate delay into the state for the report.
    let state = {
        let mut s = state;
        s.pre_ready_frames = pre_ready_frames;
        s
    };

    // Planner end-of-run check: if the goal stack is not EMPTY, the top
    // unfinished primitive never achieved its condition — a goal that
    // was pursued but not reached (progress-stall bug signature).
    let mut snap = violations.snapshot();
    if let Some(v) = crate::planner::planner_unfinished_check(&state.planner, &state) {
        snap.push(v);
    }
    let crash_detected = result.is_err();
    let mut final_metrics = state.metrics;
    // Fold the Welford frame-timing worst and the ROLLING p99 into the
    // report metrics (frame_ms_p99 is now a true nearest-rank p99 over
    // the last 600 frames — see MAPPING-NOTES history: it was previously
    // a latest-value proxy).
    final_metrics.worst_frame_ms = final_metrics
        .worst_frame_ms
        .max(state.frame_timing.worst_ms);
    if let Some(p99) = state.frame_timing.p99_ms() {
        final_metrics.frame_ms_p99 = p99;
    }
    final_metrics.crash_detected = crash_detected;
    let status = if crash_detected {
        crate::enums::PlaytestStatus::Crash
    } else if snap.is_empty() {
        crate::enums::PlaytestStatus::Pass
    } else {
        crate::enums::PlaytestStatus::Fail
    };
    let action_log = app
        .world()
        .get_resource::<crate::contract::ActionLog>()
        .map(|l| l.entries.clone())
        .unwrap_or_default();
    let cheat_count = action_log
        .iter()
        .filter(|e| e.action.starts_with("cheat:"))
        .count();
    let game_version = app
        .world()
        .get_resource::<crate::contract::GameVersion>()
        .map(|v| v.0.clone());

    // Auto-minimization on crash is NOT done inline — the harness cannot
    // rebuild the game's App from inside run_scenario. Instead, the
    // caller uses `minimize_crash`, which takes an app builder and the
    // report's action log, applies ddmin, and returns the minimal
    // reproducer plus a ready-to-save regression scenario.

    Ok(PlaytestReport {
        status,
        violations: snap,
        metrics: final_metrics,
        coverage: state.coverage,
        frame_count: state.frame,
        error: result.err().map(|e| format!("{:?}", e)),
        action_log,
        game_version,
        cheat_count,
        minimized_actions: Vec::new(),
        system_coverage: system_coverage(&systems_before, app.world_mut()),
    })
}

// ---------------------------------------------------------------------------
// Branch testing — fork the World, run variant scenarios, compare reports.
// Bevy-exclusive: cloning a World is cheap; Godot ports pay dearly for
// this. Enables non-destructive state-space exploration .
// ---------------------------------------------------------------------------

/// Outcome of a single variant in a branch matrix: how many ticks ran
/// before the status was decided, and the report (if the run reached
/// completion or crashed; early-stopped branches carry a partial report).
#[derive(Debug, Clone)]
pub struct BranchOutcome {
    /// Human-readable variant name (from the scenario's bot config or caller)
    pub variant_name: String,
    /// Tick count at termination
    pub ticks_run: u64,
    /// Report from this branch
    pub report: PlaytestReport,
}

impl BranchOutcome {
    /// Did this branch complete without violations or crashes?
    pub fn passed(&self) -> bool {
        self.report.status == crate::enums::PlaytestStatus::Pass
    }
}

/// Result of a branch matrix run.
#[derive(Debug, Clone)]
pub struct BranchMatrixReport {
    /// Ordered outcomes — same order as input scenarios.
    pub outcomes: Vec<BranchOutcome>,
}

impl BranchMatrixReport {
    /// Did ALL variants pass?
    pub fn all_passed(&self) -> bool {
        self.outcomes.iter().all(|o| o.passed())
    }

    /// Names of variants that did NOT pass.
    pub fn failed_variants(&self) -> Vec<&str> {
        self.outcomes
            .iter()
            .filter(|o| !o.passed())
            .map(|o| o.variant_name.as_str())
            .collect()
    }
}

/// Run a scenario matrix — each variant builds a fresh App via
/// `app_builder` (which registers the game's plugins) and runs its
/// scenario. Determinism comes from per-variant seed + resets, not
/// World snapshots.
///
/// DESIGN NOTE on World cloning: a deep `World` fork (true snapshot/
/// branch) requires every component/resource to be `Reflect`-registered
/// plus a snapshot crate (bevy_save-style). We deliberately do NOT
/// require that — each branch replays setup instead, which is
/// deterministic given identical resets + seed. This covers the common
/// matrix case with zero reflect burden on the game.
///
/// Example:
///
/// ```ignore
/// let mut variants = Vec::new();
/// for seed in [1u64, 2, 3, 4, 5] {
///     let mut scenario = base_scenario.clone();
///     scenario.bot.seed = seed;
///     variants.push((format!("chaos-seed-{seed}"), scenario));
/// }
/// let matrix = run_branch_matrix(build_headless_app, variants)?;
/// assert!(matrix.all_passed(), "failed: {:?}", matrix.failed_variants());
/// ```
// ---------------------------------------------------------------------------
// Crash minimization (ddmin) — Rebellion-style crash reproducer reduction.
// ---------------------------------------------------------------------------
//
/// Outcome of a crash-minimization session.
#[derive(Debug, Clone)]
pub struct MinimizeOutcome {
    /// Minimal subsequence of actions still reproducing the crash.
    pub minimal_actions: Vec<TimedAction>,
    /// A ready-to-save regression scenario that replays the minimal
    /// action sequence (persist to assets/scenarios/regression_*.json).
    pub regression_scenario: Scenario,
}

/// Parse a logged action string back into a replayable intent. Handles
/// both the legacy per-bot formats ("move:..", "choice:idx=..") and the
/// canonical structured form: action "intent:<variant>" with a JSON
/// payload in `details` (written by the IntentAudited observer for ALL
/// writers — chaos/replay/pursuit/planner/game adapters alike). This is
/// what makes crashes found by ANY bot minimizable via ddmin.
pub fn action_to_replay_intent(entry: &TimedAction) -> Option<ReplayIntent> {
    let a = &entry.action;
    // Canonical structured form: intent:<variant> + JSON details.
    if let Some(variant) = a.strip_prefix("intent:") {
        return match variant {
            "wait" => Some(ReplayIntent::Wait),
            _ => {
                let d = entry.details.as_deref()?;
                let v: serde_json::Value = serde_json::from_str(d).ok()?;
                match variant {
                    "move" => {
                        let arr = v.get("dir")?.as_array()?;
                        let x = arr.first()?.as_f64()? as f32;
                        let y = arr.get(1)?.as_f64()? as f32;
                        Some(ReplayIntent::Move { dir: (x, y) })
                    }
                    "choice" => Some(ReplayIntent::Choice {
                        index: v.get("index")?.as_u64()? as usize,
                    }),
                    "axis" => Some(ReplayIntent::Axis {
                        name: v.get("name")?.as_str()?.to_string(),
                        value: v.get("value")?.as_f64()? as f32,
                    }),
                    "select" => Some(ReplayIntent::Select {
                        target: v.get("target")?.as_str()?.to_string(),
                    }),
                    _ => None,
                }
            }
        };
    }
    // Legacy per-bot string formats (kept for compatibility with old logs).
    if let Some(rest) = a.strip_prefix("choice:idx=") {
        return rest
            .parse::<usize>()
            .ok()
            .map(|index| ReplayIntent::Choice { index });
    }
    if let Some(rest) = a.strip_prefix("select:name=") {
        return Some(ReplayIntent::Select {
            target: rest.to_string(),
        });
    }
    if a == "wait" {
        return Some(ReplayIntent::Wait);
    }
    None
}

/// Minimize a crashing scenario via ddmin over its recorded action log.
/// `app_builder` must build a FRESH App with the game's plugins (same
/// construction as the original run — `run_branch_matrix` pattern).
/// Returns the minimal reproducer and a ready-to-persist regression
/// scenario; `None` if the crash is not reproducible from the log alone
/// (e.g., it depends on wall-clock or unseeded randomness).
pub fn minimize_crash(
    app_builder: impl Fn() -> App,
    scenario: &Scenario,
    action_log: &[crate::contract::ActionEntry],
) -> Option<MinimizeOutcome> {
    let timed = action_log_to_timed_actions(action_log);
    // Only intent-bearing actions are replayable.
    let replayable: Vec<TimedAction> = timed
        .iter()
        .filter(|ta| action_to_replay_intent(ta).is_some())
        .cloned()
        .collect();
    if replayable.is_empty() {
        return None;
    }

    let reproduce = |candidate: &[TimedAction]| -> bool {
        let mut app = app_builder();
        let inputs: Vec<ReplayInput> = candidate
            .iter()
            .filter_map(|ta| {
                action_to_replay_intent(ta).map(|intent| ReplayInput {
                    frame: ta.frame,
                    intent,
                })
            })
            .collect();
        let replay_scenario = Scenario {
            setup: scenario.setup.clone(),
            duration_s: scenario.duration_s,
            bot: BotConfig {
                bot_type: crate::enums::BotType::Replay,
                seed: scenario.bot.seed,
                input_rate_hz: scenario.bot.input_rate_hz,
                agent_target: None,
                target: None,
                deadzone: scenario.bot.deadzone,
                goals: None,
                inputs,
                pointer_clicks: vec![],
                key_presses: vec![],
                persona: None,
            },
            invariants: scenario.invariants.clone(),
        };
        matches!(run_scenario(&mut app, &replay_scenario), Ok(r) if r.status == crate::enums::PlaytestStatus::Crash)
    };

    let minimal = ddmin_minimize(&replayable, reproduce);
    let inputs: Vec<ReplayInput> = minimal
        .iter()
        .filter_map(|ta| {
            action_to_replay_intent(ta).map(|intent| ReplayInput {
                frame: ta.frame,
                intent,
            })
        })
        .collect();
    let regression_scenario = Scenario {
        setup: scenario.setup.clone(),
        duration_s: scenario.duration_s,
        bot: BotConfig {
            bot_type: crate::enums::BotType::Replay,
            seed: scenario.bot.seed,
            input_rate_hz: scenario.bot.input_rate_hz,
            agent_target: None,
            target: None,
            deadzone: scenario.bot.deadzone,
            goals: None,
            inputs,
            pointer_clicks: vec![],
            key_presses: vec![],
            persona: None,
        },
        invariants: scenario.invariants.clone(),
    };
    Some(MinimizeOutcome {
        minimal_actions: minimal,
        regression_scenario,
    })
}

/// Serialize a regression scenario to pretty JSON for persistence.
pub fn regression_scenario_json(scenario: &Scenario) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(scenario)
}

pub fn run_branch_matrix(
    app_builder: impl Fn() -> App,
    variants: Vec<(String, Scenario)>,
) -> Result<BranchMatrixReport, ScenarioError> {
    let mut outcomes = Vec::with_capacity(variants.len());
    for (name, scenario) in variants {
        let mut app = app_builder();
        let report = run_scenario(&mut app, &scenario)?;
        outcomes.push(BranchOutcome {
            variant_name: name,
            ticks_run: report.frame_count,
            report,
        });
    }
    Ok(BranchMatrixReport { outcomes })
}

// ---------------------------------------------------------------------------
// Contract diagnostics — detect contract pieces, warn on gaps
// ---------------------------------------------------------------------------

/// Report of contract-tier detection for a built App. Produced by
/// [`contract_diagnostics`]; consumed by the CLI and human users.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ContractReport {
    /// Reset hooks registered (empty => scenarios can't reset).
    pub reset_hooks: Vec<String>,
    /// Cheat kinds registered.
    pub cheat_kinds: Vec<String>,
    /// Named intents registered.
    pub named_intents: Vec<String>,
    /// Intent surface size (0 => chaos bot samples nothing).
    pub intent_surface_size: usize,
    /// Whether a TestApi-like resolve succeeds at all.
    pub test_api_present: bool,
    /// Recommendations based on gaps.
    pub recommendations: Vec<String>,
    /// Detected gameplay features via field vocabulary.
    pub detected_features: Vec<DetectedFeature>,
}

/// Scan a built App for contract coverage. Pure inspection — no
/// mutation, safe to call on any App.
pub fn contract_diagnostics(world: &World) -> ContractReport {
    let mut recommendations = Vec::new();

    let reset_hooks: Vec<String> = world
        .get_resource::<ResetHooks>()
        .map(|h| h.0.keys().cloned().collect())
        .unwrap_or_default();
    if reset_hooks.is_empty() {
        recommendations.push(
            "no reset hooks — add register ResetHooks explicitly or register ResetHooks".into(),
        );
    }

    let cheat_kinds: Vec<String> = world
        .get_resource::<crate::contract::CheatHooks>()
        .map(|h| h.0.keys().cloned().collect())
        .unwrap_or_default();
    if cheat_kinds.is_empty() {
        recommendations.push(
            "no cheats registered — CheatHooks unlock scenario setup.cheats for state shaping"
                .into(),
        );
    }

    let named_intents: Vec<String> = world
        .get_resource::<crate::contract::NamedIntents>()
        .map(|n| n.0.keys().cloned().collect())
        .unwrap_or_default();
    if named_intents.is_empty() {
        recommendations.push(
            "no named intents — NamedIntents let agents drive gameplay semantically (e.g. end_turn)"
                .into(),
        );
    }

    let intent_surface_size = world
        .get_resource::<IntentSurface>()
        .map(|s| s.0.len())
        .unwrap_or(0);
    if intent_surface_size == 0 {
        recommendations.push(
            "empty IntentSurface — chaos/pursuit bots can't sample meaningful actions".into(),
        );
    }

    let test_api_present = world.get_resource::<TestApi>().is_some();

    // Feature detection: classify gameplay by component/resource field
    // vocabulary (zero game cooperation). Feeds persona selection and
    // invariant suggestions.
    let detected_features = detect_features(world);

    ContractReport {
        reset_hooks,
        cheat_kinds,
        named_intents,
        intent_surface_size,
        test_api_present,
        recommendations,
        detected_features,
    }
}

/// A gameplay feature detected by reflection — zero game cooperation.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub enum DetectedFeature {
    TurnBased,
    Combat,
    Inventory,
    Dialogue,
    RealTime,
}

impl DetectedFeature {
    pub fn label(&self) -> &'static str {
        match self {
            DetectedFeature::TurnBased => "turn_based",
            DetectedFeature::Combat => "combat",
            DetectedFeature::Inventory => "inventory",
            DetectedFeature::Dialogue => "dialogue",
            DetectedFeature::RealTime => "real_time",
        }
    }
}

impl std::fmt::Display for DetectedFeature {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.label())
    }
}

/// Field-vocabulary probes for feature detection: (feature, field names).
const FEATURE_VOCAB: [(&str, &[&str]); 4] = [
    (
        "turn_based",
        &["turn", "turn_number", "current_turn", "faction_turn"],
    ),
    ("combat", &["hp", "health", "damage", "attack", "defense"]),
    (
        "inventory",
        &["item", "capacity", "slot", "inventory", "stack"],
    ),
    ("dialogue", &["dialogue", "speaker", "line", "node_choice"]),
];

/// Walk reflected types, collect all field names present in the app.
fn collect_field_vocab(world: &World) -> std::collections::HashSet<String> {
    use bevy::ecs::reflect::AppTypeRegistry;
    let mut fields = std::collections::HashSet::new();
    let Some(registry) = world.get_resource::<AppTypeRegistry>() else {
        return fields;
    };
    let registry = registry.0.clone();
    let reg = registry.read();
    for ty in reg.iter() {
        let path = ty.type_info().type_path();
        // Skip harness-owned types (they'd pollute detection).
        if HARNESS_TYPE_PATHS.iter().any(|h| path.starts_with(h)) {
            continue;
        }
        if let bevy::reflect::TypeInfo::Struct(s) = ty.type_info() {
            for name in s.field_names() {
                fields.insert(name.to_string());
            }
        }
    }
    fields
}

/// Entity-motion heuristic for real_time detection: if Transform components
/// exist AND entity positions change across two consecutive reads... but we
/// only have one world here. Simpler proxy: presence of a `Time` resource
/// with advancing virtual clock plus moving transforms is undecidable at
/// rest — fall back to Transform+Velocity-style field vocab.
fn detect_features(world: &World) -> Vec<DetectedFeature> {
    let mut found = Vec::new();
    let vocab = collect_field_vocab(world);
    for (feature, names) in FEATURE_VOCAB {
        if names.iter().any(|n| vocab.contains(*n)) {
            match feature {
                "turn_based" => found.push(DetectedFeature::TurnBased),
                "combat" => found.push(DetectedFeature::Combat),
                "inventory" => found.push(DetectedFeature::Inventory),
                "dialogue" => found.push(DetectedFeature::Dialogue),
                _ => unreachable!(),
            }
        }
    }
    found
}

/// Persona-flavored chaos bot configuration — different exploration
/// biases find different bugs (MIMIC finding).
#[derive(Deserialize, Clone, Debug, Default, PartialEq)]
pub struct PersonaConfig {
    /// "aggressive" (high rate, many selects/choices), "curious" (bias
    /// toward unseen variants), "idle" (mostly Wait, occasional jabs).
    #[serde(default)]
    pub persona: Option<String>,
}

/// Extract a persona hint from a scenario bot config. Overrides the
/// chaos bot's default uniform-ish sampling.
pub fn persona_of(bot: &BotConfig) -> crate::enums::Persona {
    bot.persona.unwrap_or(crate::enums::Persona::Curious)
}

/// Probes the world's reflected resources and auto-publishes observable
/// paths for them. Contract shrinks to "derive Reflect + register_type"
/// (which games do anyway for saves). Games with their own TestApi keep
/// their hand-written resolution, but gain automatic observability of
/// every reflected resource field.
pub fn auto_observed_paths(world: &World) -> Vec<String> {
    use bevy::ecs::reflect::AppTypeRegistry;
    let mut paths = Vec::new();
    let Some(registry) = world.get_resource::<AppTypeRegistry>() else {
        return paths;
    };
    let registry = registry.0.clone();
    let reg = registry.read();
    'outer: for registration in reg.iter() {
        let type_path = registration.type_info().type_path();
        if HARNESS_TYPE_PATHS.iter().any(|h| type_path.starts_with(h)) {
            continue;
        }
        let Some(rc) = registration.data::<bevy::ecs::reflect::ReflectComponent>() else {
            continue;
        };
        let Some(component_id) = world.components().get_valid_id(registration.type_id()) else {
            continue;
        };
        for (cid, entity) in world.resource_entities().iter() {
            if cid != component_id {
                continue;
            }
            let Ok(entity_ref) = world.get_entity(entity) else {
                continue;
            };
            if rc.reflect(entity_ref).is_none() {
                continue;
            }
            // Enumerate struct fields from static TypeInfo — no live
            // value probing needed for path listing.
            if let bevy::reflect::TypeInfo::Struct(s) = registration.type_info() {
                let short_name = short_type_name(type_path);
                for name in s.field_names() {
                    paths.push(format!("{}.{}", short_name, name));
                }
            }
            continue 'outer;
        }
    }
    paths.sort();
    paths.dedup();
    paths
}

fn short_type_name(type_path: &str) -> &str {
    type_path.rsplit("::").next().unwrap_or(type_path)
}

// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Auto-discovery calibration — derive invariants from calibration runs
// ---------------------------------------------------------------------------

/// Calibration snapshot: baseline measurements from a short chaos run.
#[derive(Clone, Debug, Default)]
pub struct CalibrationSnapshot {
    /// Archetype counts: (component_signatures_as_sorted_pipe_delimited, min_count, max_count)
    pub archetype_envelopes: Vec<(String, usize, usize)>,
    /// Resources observed: (short_type_name, last_numeric_value)
    pub resource_baselines: Vec<(String, f64)>,
}

/// Run a calibration phase: short chaos-driven run that probes the world
/// for measurable patterns. Returns a snapshot suitable for generating
/// candidate invariants.
///
/// USAGE: Call this once per game binary, save the snapshot, then
/// programmatically generate invariants (e.g., "archetype X never drops
/// below N"). Zero game-code changes required.
pub fn calibrate_world(
    app: &mut App,
    duration_s: f32,
) -> Result<CalibrationSnapshot, ScenarioError> {
    // Inject a minimal chaos scenario so harness systems can run.
    let scenario = serde_json::from_str::<Scenario>(r#"{"bot":{"type":"chaos","seed":42}}"#)
        .map_err(|e| ScenarioError::Rejected(format!("internal: bad calibration scenario: {e}")))?;
    app.insert_resource(ScenarioResource(scenario));
    app.insert_resource(Violations::default());

    let tps = app.world().resource::<PlaytestState>().tps;
    let total_ticks = (duration_s * tps as f32) as u64;

    // Track per-signature count extremes across MANY samples — a
    // two-point (start/end) estimate misses spawn/despawn transients
    // (e.g. Startup-spawned Tiles sampled as 0 before tick 1).
    let mut archetype_min: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    let mut archetype_max: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    let mut resource_values = std::collections::HashMap::<String, Vec<f64>>::new();

    let sample_every = (tps / 10).max(1); // ~10 samples/sec

    // Run calibration, sampling archetype counts periodically.
    for tick in 0..total_ticks {
        app.update();

        if tick % sample_every == 0 || tick == total_ticks - 1 {
            for archetype in app.world().archetypes().iter() {
                let mut sig_parts: Vec<String> = archetype
                    .components()
                    .iter()
                    .filter_map(|id| {
                        app.world()
                            .components()
                            .get_info(*id)
                            .map(|info| info.name().to_string())
                    })
                    .collect();
                sig_parts.sort();
                let sig = sig_parts.join("|");
                let len = archetype.entities().len();
                if len == 0 {
                    continue;
                }
                let cur_min = archetype_min.entry(sig.clone()).or_insert(len);
                if len < *cur_min {
                    *cur_min = len;
                }
                let cur_max = archetype_max.entry(sig).or_insert(len);
                if len > *cur_max {
                    *cur_max = len;
                }
            }
        }

        // Sample resource baselines
        if let Some(api) = app.world().get_resource::<TestApi>() {
            if api.score != 0 {
                resource_values
                    .entry("TestApi".into())
                    .or_default()
                    .push(api.score as f64);
            }
        }
    }

    // Compute envelopes
    let mut archetype_envelopes = Vec::new();
    let mut all_sigs: Vec<String> = archetype_min.keys().cloned().collect();
    all_sigs.sort();
    for sig in all_sigs {
        let min = archetype_min[&sig];
        let max = archetype_max[&sig];
        if max > 0 {
            archetype_envelopes.push((sig, min, max));
        }
    }

    let mut resource_baselines = Vec::new();
    for (name, values) in resource_values {
        let avg = values.iter().sum::<f64>() / values.len() as f64;
        resource_baselines.push((name, avg));
    }
    resource_baselines.sort_by(|a, b| a.0.cmp(&b.0));

    Ok(CalibrationSnapshot {
        archetype_envelopes,
        resource_baselines,
    })
}

/// Generate candidate invariants from a calibration snapshot.
/// Returns JSON-formatted invariants ready to drop into a scenario.
pub fn generate_invariants_from_calibration(snapshot: &CalibrationSnapshot) -> String {
    use serde_json::json;

    let mut invariants = Vec::new();

    // Archetype envelope invariants. Skip resource archetypes
    // (IsResource markers — covered by resource baselines instead) and
    // use SHORT component names to match query-resolution semantics.
    for (sig, min, _max) in &snapshot.archetype_envelopes {
        if sig.contains("IsResource") {
            continue;
        }
        if *min > 0 {
            // Query with GAME-OWNED components only — engine internals
            // (Transform, Observer, ...) are not the game author's
            // contract and make brittle invariants.
            let comps: Vec<String> = sig
                .split('|')
                .filter(|c| {
                    !c.starts_with("bevy_") && !c.contains("harvestcycle::state::GameState")
                })
                .map(|s| short_type_name(s).to_string())
                .collect();
            if comps.is_empty() {
                continue;
            }
            invariants.push(json!({
                "name": format!("archetype_min_{}", sanitize_name(sig)),
                "rule": "custom",
                "query": {
                    "with": comps,
                    "without": []
                },
                "check": "above",
                "value": (*min - 1) as f64,
                "eventually_s": null
            }));
        }
    }

    // Resource baseline hints
    for (name, val) in &snapshot.resource_baselines {
        if *val >= 0.0 {
            invariants.push(json!({
                "name": format!("{}_non_negative", sanitize_name(name)),
                "rule": "custom",
                "path": format!("{}.score", name),
                "check": "above",
                "value": -0.5,
                "eventually_s": null
            }));
        }
    }

    serde_json::to_string_pretty(&invariants).unwrap()
}

fn sanitize_name(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect()
}

// ---------------------------------------------------------------------------
// Frozen-world stuck oracle (ICARUS/aplib liveness check)
// ---------------------------------------------------------------------------

/// Detects a "frozen world" — no Gameplay entity's Transform changed for
/// 2+ seconds while a real-time scenario runs. Turn-based games are
/// exempt: if any entity carries the `TurnBased` marker component,
/// waiting between turns is by design, not a soft-lock.
/// Zero contract: pure ECS change detection, O(changes) per tick.
fn frozen_world_oracle_system(
    q_changed: Query<(), (With<crate::contract::Gameplay>, Changed<Transform>)>,
    q_turn_based: Query<(), With<crate::contract::TurnBased>>,
    mut state: ResMut<PlaytestState>,
    mut violations: ResMut<Violations>,
) {
    // Turn-based exemption: legitimate long pauses between turns.
    if q_turn_based.iter().next().is_some() {
        return;
    }
    if q_changed.iter().next().is_some() {
        state.frozen_frames = 0;
        return;
    }
    state.frozen_frames += 1;
    let threshold = state.tps.saturating_mul(2);
    if state.frozen_frames == threshold {
        violations.report(
            "frozen_world",
            "world",
            format!(
                "no Gameplay entity changed for 2.0s ({} frames) — world may be stuck (soft-lock)",
                state.frozen_frames
            ),
            state.frame,
        );
    }
}

// ---------------------------------------------------------------------------
// Readiness gate (UE PrepareTest/IsReady analog)
// ---------------------------------------------------------------------------

/// Games with async setup (asset streaming, level generation, server
/// connect) insert this resource and flip it true when their world is
/// ready for the scenario to begin. Until ready, scenario duration does
/// not accrue — avoiding flaky frame-0 assumptions. Games that don't
/// insert it get immediate readiness (frame-0 behavior preserved).
#[derive(Resource, Default)]
pub struct GameReady(pub bool);

// ---------------------------------------------------------------------------
// Actionability gate for synthetic_pointer (Playwright-style, opt-in)
// ---------------------------------------------------------------------------

/// One frame after a pointer click completes, consult the picking backend's
/// `HoverMap` to verify the pointer was over SOME entity — proof the picking
/// wiring is alive and the click wasn't masked/occluded into oblivion.
fn synthetic_pointer_actionability_check_system(
    mut state: ResMut<PlaytestState>,
    mut violations: ResMut<Violations>,
    hover_map: Option<Res<bevy::picking::hover::HoverMap>>,
) {
    let frame = state.frame;
    let mut still_pending = std::collections::VecDeque::new();
    while let Some(g) = state.pending_actionability_checks.pop_front() {
        // This system runs one frame AFTER release_at (release_at == frame-1).
        if g.release_at + 1 != frame {
            still_pending.push_back(g);
            continue;
        }
        if let Some(hover) = hover_map.as_ref() {
            // HoverMap keys are PointerId; values are maps from window to hovered entities.
            let mouse_hovered = hover
                .get(&PointerId::Mouse)
                .and_then(|wm| wm.values().next());
            if mouse_hovered.is_none() {
                violations.report(
                    "pointer_not_actionable",
                    &g.target_name,
                    format!(
                        "click on '{}' completed but hover map shows no entity under cursor — picking backend may be dead or target occluded",
                        g.target_name
                    ),
                    frame,
                );
            }
        } else {
            // No HoverMap resource → picking backend not wired.
            violations.report(
                "pointer_not_actionable",
                &g.target_name,
                "no HoverMap resource — picking backend not active".to_string(),
                frame,
            );
        }
    }
    state.pending_actionability_checks = still_pending;
}
