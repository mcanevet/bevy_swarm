//! Test driver: PlaytestPlugin, run_scenario, and the report.

use bevy::app::App;
use bevy::ecs::entity::Entity;
use bevy::ecs::message::MessageWriter;
use bevy::ecs::query::With;
use bevy::ecs::resource::Resource;
use bevy::ecs::system::{Query, Res, ResMut};
use bevy::ecs::world::World;
use std::collections::{HashMap, HashSet};

use crate::contract::{ResetHooks, TestApi};
use crate::scenario::*;
use crate::state::*;
use crate::bots::*;
use crate::oracles::check_custom_system;
use crate::oracles::*;
use bevy::picking::pointer::PointerInput;

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
    // report metrics (frame_ms_p99 is a true nearest-rank p99 over the
    // last 600 frames, computed by FrameTimingStats).
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

pub(crate) fn cheat_scheduler_system(world: &mut World) {
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
pub(crate) fn synthetic_keyboard_bot_system(
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
