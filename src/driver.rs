//! Test driver: PlaytestPlugin, run_scenario, and the report.

use bevy::app::App;
use bevy::ecs::entity::Entity;
use bevy::ecs::message::MessageWriter;
use bevy::ecs::query::With;
use bevy::ecs::resource::Resource;
use bevy::ecs::schedule::{LogLevel, ScheduleBuildSettings, SingleThreadedExecutor};
use bevy::ecs::system::{Query, Res, ResMut};
use bevy::ecs::world::World;

use crate::bots::*;
use crate::contract::{ResetHooks, TestApi};
use crate::oracles::*;
use crate::scenario::*;
use crate::state::*;
use bevy::picking::pointer::PointerInput;

//-----------------------------------------------------------------------
// PlaytestPlugin
//-----------------------------------------------------------------------

/// Harness pipeline phases.
///
/// # Frame pipeline
///
/// ```text
/// Schedule  | Set                   | Systems
/// ----------|-----------------------|----------------------------------------
/// First     | PlaytestSet::Tick     | tick_counter, cheat_scheduler (chained)
/// PreUpdate | PlaytestSet::Bots     | chaos, replay, pursuit, planner
/// Update    | PlaytestSet::RawInput | synthetic pointer/keyboard bots
/// Last      | PlaytestSet::Oracles  | frozen_world, finite_transforms,
///           |                       | bounds, custom, intent_audit_log
/// ```
///
/// `state.frame` is incremented in `First`, so bots (PreUpdate), game
/// systems (Update) and the audit log (Last) all see the SAME frame
/// number: an intent logged at frame N replays at frame N and is
/// consumed by the game in the same frame.
///
/// Games that consume `UserIntent` in `Update` run after
/// `PlaytestSet::Bots` automatically (PreUpdate precedes Update).
/// Games reading `UserIntent` in `PreUpdate` must order themselves
/// `.after(PlaytestSet::Bots)`.
///
/// # Actuator frame semantics (Bevy 0.20)
///
/// - Typed intents (`UserIntent`) are written in PreUpdate/Bots and are
///   readable the same frame.
/// - Raw keyboard/gamepad input written in Update reaches `ButtonInput`
///   only at the NEXT frame's PreUpdate (Bevy's input plugin copies the
///   accumulated buffer in PreUpdate). Raw keyboard injection therefore
///   belongs in PreUpdate before InputSystems; the synthetic POINTER
///   gesture stays in Update (bevy_picking reads the previous frame's
///   hover map).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, bevy::ecs::schedule::SystemSet)]
pub enum PlaytestSet {
    /// Frame counter: runs first, so every later system in the frame
    /// sees the same frame number.
    Tick,
    /// Typed-intent bots (chaos/replay/pursuit/planner) writing
    /// `UserIntent`.
    Bots,
    /// Raw-input bots (pointer/keyboard) going through the real input
    /// chain.
    RawInput,
    /// World checks, evaluated after ALL game systems of the frame.
    Oracles,
}

#[derive(Resource)]
pub struct ScenarioResource(pub std::sync::Arc<Scenario>);

impl std::ops::Deref for ScenarioResource {
    type Target = Scenario;
    fn deref(&self) -> &Scenario {
        &self.0
    }
}

pub struct PlaytestPlugin;

impl bevy::app::Plugin for PlaytestPlugin {
    fn build(&self, app: &mut App) {
        use bevy::ecs::schedule::IntoScheduleConfigs;
        // PointerInput registration: headless test apps may lack DefaultPlugins,
        // so ensure the synthetic pointer bot can always write. Idempotent.
        app.add_message::<PointerInput>();
        // Synthetic keyboard bot writes KeyboardInput; register so apps
        // without DefaultPlugins don't fail parameter validation.
        app.add_message::<bevy::input::keyboard::KeyboardInput>();
        // The default contract's UserIntent message + core resources:
        // headless apps built by headless_app() get PlaytestPlugin
        // without the game adding TestConventionsPlugin, so register
        // everything the harness systems read. Idempotent.
        app.add_message::<crate::contract::UserIntent>();
        app.init_resource::<crate::state::Violations>();
        // I1: identity index + observers (idempotent).
        crate::identity::install_identity(app);
        // I3: typed oracle/predicate/policy registry.
        app.init_resource::<crate::typed::TypedRegistry>();
        // RF11: bevy's bevy_picking feature compiles viewport_picking
        // and UiPickingPlugin into UiPlugin. They need the picking
        // pipeline's resources/messages, which apps without
        // DefaultPlugins lack. Install them (idempotent).
        app.init_resource::<bevy::picking::hover::HoverMap>();
        app.init_resource::<bevy::picking::hover::PreviousHoverMap>();
        app.init_resource::<bevy::picking::events::PointerState>();
        app.add_message::<bevy::picking::backend::PointerHits>();
        app.add_message::<bevy::picking::pointer::PointerInput>();
        app.add_message::<bevy::picking::events::PointerCancel>();
        app.add_message::<bevy::picking::events::PointerClick>();
        app.add_message::<bevy::picking::events::PointerPress>();
        app.add_message::<bevy::picking::events::PointerDragDrop>();
        app.add_message::<bevy::picking::events::PointerDragEnd>();
        app.add_message::<bevy::picking::events::PointerDragEnter>();
        app.add_message::<bevy::picking::events::PointerDragLeave>();
        app.add_message::<bevy::picking::events::PointerDragOver>();
        app.add_message::<bevy::picking::events::PointerDragStart>();
        app.add_message::<bevy::picking::events::PointerDrag>();
        app.add_message::<bevy::picking::events::PointerMove>();
        app.add_message::<bevy::picking::events::PointerOut>();
        app.add_message::<bevy::picking::events::PointerOver>();
        app.add_message::<bevy::picking::events::PointerEnter>();
        app.add_message::<bevy::picking::events::PointerLeave>();
        app.init_resource::<bevy::picking::PickingSettings>();
        app.init_resource::<bevy::picking::hover::PointerCaptureMap>();
        app.init_resource::<bevy::picking::pointer::PointerMap>();
        app.init_resource::<bevy::picking::backend::ray::RayMap>();
        // RF11: headless camera target_info — fill Camera.computed.target_info
        // from the primary window so resolve_click_position can project world→viewport.
        app.add_systems(bevy::app::PostStartup, headless_camera_target_info_system);

        // Every harness system is gated on a live scenario so the plugin
        // is safe to leave in a production App (or an App never driven
        // by run_scenario): without ScenarioResource + PlaytestState the
        // sets are skipped entirely.
        // Both ScenarioResource and PlaytestState must exist for harness
        // systems to run.
        let scenario_live = |world: &World| {
            world.get_resource::<ScenarioResource>().is_some()
                && world.get_resource::<PlaytestState>().is_some()
        };
        // Bots and oracles must not run before readiness (a game may
        // spend frames loading assets); only tick/crash capture runs
        // pre-ready. Games that never insert GameReady are instantly
        // ready (frame-0 behavior preserved).
        let game_ready = |world: &World| {
            world
                .get_resource::<crate::oracles::GameReady>()
                .is_none_or(|r| r.0)
        };
        let live_and_ready = move |world: &World| scenario_live(world) && game_ready(world);
        app.configure_sets(bevy::app::First, PlaytestSet::Tick.run_if(scenario_live));
        app.configure_sets(
            bevy::app::PreUpdate,
            PlaytestSet::Bots.run_if(live_and_ready),
        );
        app.configure_sets(
            bevy::app::Update,
            PlaytestSet::RawInput.run_if(live_and_ready),
        );
        app.configure_sets(bevy::app::Last, PlaytestSet::Oracles.run_if(live_and_ready));

        app.add_systems(
            bevy::app::First,
            (
                tick_counter_system,
                // Z6: incremental gameplay inference (no-op unless
                // GameTypes present and no explicit Gameplay marker).
                crate::game_types::apply_gameplay_inference,
                cheat_scheduler_system,
            )
                .chain()
                .in_set(PlaytestSet::Tick),
        );
        app.init_resource::<crate::raw_input::RawActionQueue>();
        app.init_resource::<crate::effects::ActionEffects>();
        app.init_resource::<crate::raw_input::InputHolds>();
        app.init_resource::<crate::raw_input::VirtualGamepad>();
        app.init_resource::<crate::raw_input::HeadlessWindow>();
        // le3.21 (RF25): harness systems read these; run_scenario
        // replaces them per-run, but any app driven by the plugin
        // (e.g. a test calling app.update() directly) must not panic
        // on missing params — a pool-thread panic leaks into sibling
        // runs under parallel autotest.
        app.init_resource::<crate::robustness::RobustnessTracker>();
        app.init_resource::<crate::choice::ChoiceStream>();
        // Raw input actuator writes these messages directly; without
        // winit nothing else registers them in a headless app.
        app.add_message::<bevy::input::keyboard::KeyboardInput>();
        app.add_message::<bevy::input::mouse::MouseButtonInput>();
        app.add_message::<bevy::input::mouse::MouseMotion>();
        app.add_message::<bevy::input::mouse::MouseWheel>();
        app.add_message::<bevy::window::WindowEvent>();
        #[cfg(feature = "gamepad")]
        {
            app.add_message::<bevy::input::gamepad::GamepadConnectionEvent>();
            app.add_message::<bevy::input::gamepad::RawGamepadEvent>();
            app.add_message::<bevy::input::gamepad::GamepadEvent>();
        }

        // Raw input actuator (Z3): keyboard/gamepad injection in PreUpdate,
        // before Bevy's InputSystems so ButtonInput reflects them same-frame.
        app.add_systems(
            bevy::app::PreUpdate,
            crate::raw_input::raw_input_preupdate_system
                .after(PlaytestSet::Bots)
                .before(bevy::input::InputSystems),
        );

        app.add_systems(
            bevy::app::PreUpdate,
            (
                chaos_bot_system.run_if(bot_is(crate::enums::BotType::Chaos)),
                replay_bot_system.run_if(bot_is(crate::enums::BotType::Replay)),
                pursuit_bot_system.run_if(bot_is(crate::enums::BotType::Pursuit)),
                crate::planner::planner_bot_system.run_if(bot_is(crate::enums::BotType::Planner)),
                crate::typed::custom_policy_bot_system
                    .run_if(bot_is(crate::enums::BotType::Custom)),
            )
                .chain()
                .in_set(PlaytestSet::Bots),
        );
        app.add_systems(
            bevy::app::Update,
            (
                synthetic_pointer_bot_system,
                synthetic_pointer_actionability_check_system,
                synthetic_keyboard_bot_system,
                crate::raw_input::raw_input_click_entity_system,
                crate::raw_input::raw_input_update_system,
            )
                .chain()
                .in_set(PlaytestSet::RawInput),
        );
        app.add_systems(
            bevy::app::Last,
            (
                // I4: liveness oracle (replaces frozen_world_oracle_system).
                crate::liveness::liveness_oracle_system,
                // I5: action-effect oracle (per-action effect rates).
                crate::effects::action_effect_oracle_system,
                check_finite_transforms_system,
                check_transform_desync_system,
                check_bounds_gameplay_system,
                check_custom_system,
                // I3: typed oracles after the JSON-DSL checker.
                crate::typed::run_typed_oracles,
                // RF38: rendering checks (no_active_camera /
                // nothing_on_screen) — one-shot, after liveness settles.
                crate::render_check::render_check_oracle_system,
                // swarm-uz8: pass_through overlap tracking.
                crate::pass_through::pass_through_oracle_system,
                intent_audit_log_system,
                crate::determinism::record_state_digest_system,
            )
                .chain()
                .in_set(PlaytestSet::Oracles),
        );
    }
}

//-----------------------------------------------------------------------
// Test driver — pump the loop, catch panics, produce the report
//-----------------------------------------------------------------------

#[derive(Debug, Clone, serde::Serialize)]
pub struct PlaytestReport {
    /// Schema version: stays 1 until v0.2 release freezes it.
    pub schema_version: u32,
    pub status: crate::enums::PlaytestStatus,
    pub violations: Vec<ViolationEntry>,
    pub metrics: Metrics,
    pub coverage: Coverage,
    pub frame_count: u64,
    /// FX12 (A1): configured ticks-per-second of the run.
    pub tps: u64,
    /// FX12 (A1): simulated duration in seconds = frame_count / tps.
    pub simulated_duration_s: f64,
    /// cj8.14 (E2): wall-clock duration of the run loop, seconds.
    /// Used by JUnit output for the `<testcase time>` attribute when
    /// present (falls back to simulated time).
    pub wall_time_s: f64,
    /// Frames spent waiting for GameReady before the scenario began.
    pub pre_ready_frames: u64,
    pub error: Option<String>,
    /// AUDIT (phase 3): every injected input and cheat invocation,
    /// labeled by source. Full replay material for the run.
    pub action_log: Vec<crate::contract::ActionEntry>,
    /// AUDIT: game build identity stamped into the report.
    pub game_version: Option<String>,
    /// AUDIT: cheats detected in this run (subset of action_log, both
    /// counted and listed so reports can flag cheated runs cheaply).
    pub cheat_count: usize,
    /// Intent-audit entries that cannot be converted back into
    /// ReplayIntents (e.g. Select on an unnamed entity). Non-zero means
    /// the run is NOT fully minimizable/replayable; name your Gameplay
    /// entities.
    pub unreplayable_actions: usize,
    /// Report-level warnings (non-fatal issues worth surfacing:
    /// unnamed Select targets, harness deprecations, ...). Reused by
    /// later beads (C2/C3).
    pub warnings: Vec<String>,
    /// Code-aware coverage (CA²): which scheduled systems executed
    /// during the run, vs every registered system. Unexecuted systems
    /// are coverage gaps to target with new scenarios.
    pub system_coverage: SystemCoverage,
    /// Per-frame state digests (A4), present only when a StateTrace
    /// resource was inserted by the caller (normal runs skip it).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_trace: Option<Vec<crate::determinism::FrameDigest>>,
    /// I5: per-action effect rates (key -> (effective, total)).
    /// Feeds dead_verb and the Z1 vacuity guard.
    pub action_effect_rate: std::collections::HashMap<String, crate::effects::EffectRate>,
    /// J0: every bot random choice this run (frame, reduced value,
    /// bound). Replay via bot.choices. Populated for chaos/curious
    /// (and any bot drawing from the stream).
    pub choices: Vec<crate::choice::Choice>,
    /// J1: signed robustness margins (STL-style) per invariant and
    /// overall (normalized). Positive = satisfied by that margin.
    pub robustness: crate::robustness::RobustnessSummary,
    /// Z4: input surface discovered by differential probing, when
    /// the caller ran discovery for this game.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub discovered_surface: Option<crate::discover::DiscoveredSurface>,
    /// Z1: tier + what actually ran (oracle families, actuators).
    pub scope: crate::contract::CheckedScope,
    /// Z19/Z20: which harness mode produced this report.
    /// Conventions §7 vocabulary: "in_process", "external_smoke", "external_driven".
    pub harness_mode: String,
}

impl PlaytestReport {
    /// An empty Crash-status report — used when a runner-level
    /// structured error (scenario rejection) must not tear down a
    /// whole matrix run.
    pub fn crashed_empty() -> Self {
        Self {
            schema_version: 1,
            status: crate::enums::PlaytestStatus::Crash,
            violations: vec![],
            metrics: Default::default(),
            coverage: Default::default(),
            frame_count: 0,
            wall_time_s: 0.0,
            tps: 0,
            simulated_duration_s: 0.0,
            pre_ready_frames: 0,
            error: None,
            action_log: vec![],
            game_version: None,
            cheat_count: 0,
            unreplayable_actions: 0,
            warnings: vec![],
            system_coverage: Default::default(),
            state_trace: None,
            action_effect_rate: Default::default(),
            choices: vec![],
            robustness: Default::default(),
            discovered_surface: None,
            scope: crate::contract::CheckedScope::default(),
            harness_mode: "in_process".to_string(),
        }
    }
}

impl PlaytestReport {
    /// FX4.3: a scenario rejected before running gets a Crash-status
    /// report WITH a scenario_error violation (not an empty one), so
    /// aggregation counts it as a failure instead of 0 violations.
    pub fn scenario_error(msg: &str) -> Self {
        let mut rep = Self::crashed_empty();
        rep.error = Some(msg.to_string());
        let scheme = crate::fingerprint::default_normalizer();
        let fp = scheme.fingerprint("crash", crate::rules::Rule::ScenarioError.name(), &[], msg);
        rep.violations = vec![crate::state::ViolationEntry {
            rule: crate::rules::Rule::ScenarioError.to_string(),
            target: "scenario".to_string(),
            first_frame: 0,
            last_frame: 0,
            count: 1,
            detail: msg.to_string(),
            last_detail: msg.to_string(),
            fingerprint: Some(fp),
            fingerprint_scheme: 1,
        }];
        rep
    }
}

/// Normalize a sink message for dedup: numbers vary between frames
/// ("entity 5 failed" vs "entity 6 failed" is one signature).
/// ml5 (L3): KNOWN fidelity cost — digit-collapsing also merges
/// "entity 5 failed" with "entity 50 failed" into one grouped entry.
/// Accepted for dedup purposes (the RAW messages remain in the
/// report); grouping-by-signature is worth more than exact counting.
fn normalize_sink_message(msg: &str) -> String {
    // le3.11 (RF7): collapse consecutive digits to a single '#' so
    // "x6" and "x18" (and "hp 9" vs "hp 10") normalize identically —
    // per-digit replacement left width-dependent groups and unstable
    // fingerprints across run durations.
    let mut out = String::with_capacity(msg.len());
    let mut prev_digit = false;
    for c in msg.chars() {
        if c.is_ascii_digit() {
            if !prev_digit {
                out.push('#');
            }
            prev_digit = true;
        } else {
            out.push(c);
            prev_digit = false;
        }
    }
    out
}

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
/// Force deterministic, single-threaded execution on every schedule
/// currently registered. Call BEFORE the readiness gate and before
/// snapshot_systems (which initializes schedules). Schedules created
/// lazily afterward keep the default executor (document this caveat).
/// dde: bounded grace period for detached bevy_tasks work at run end.
///
/// Task pools are process-global and never shut down between seeds;
/// games routinely detach background work (asset decode, pathfinding).
/// A detached task that panics AFTER we cleared the run ids would be
/// lost entirely (or attributed to the next run via GLOBAL_RUN).
/// Detached futures can't be joined, and the pools expose no
/// pending-count API, so we sleep a short bounded window — but ONLY
/// when the process actually initialized the global pools (pure
/// unit-test runs without TaskPoolPlugin pay nothing).
///
/// NOTE: this sleep affects real-time oracle timing; keep it minimal
/// to avoid flaky tests (parallel_matches_sequential). 50ms is enough
/// for most asset decodes / small async tasks; stragglers beyond that
/// are rare and will be attributed to the next run (documented).
const POOL_DRAIN_BUDGET_MS: u64 = 50;

fn drain_pending_pool_work() {
    use bevy::tasks::{AsyncComputeTaskPool, ComputeTaskPool, IoTaskPool};
    if IoTaskPool::try_get().is_none()
        && ComputeTaskPool::try_get().is_none()
        && AsyncComputeTaskPool::try_get().is_none()
    {
        return;
    }
    std::thread::sleep(std::time::Duration::from_millis(POOL_DRAIN_BUDGET_MS));
}

fn force_single_threaded(app: &mut App) -> Result<(), ScenarioError> {
    let Some(mut schedules) = app
        .world_mut()
        .remove_resource::<bevy::ecs::schedule::Schedules>()
    else {
        return Err(ScenarioError::Rejected(
            "force_single_threaded: Schedules not found in app".into(),
        ));
    };
    for (_label, sched) in schedules.iter_mut() {
        sched.set_executor(SingleThreadedExecutor::new());
    }
    app.world_mut().insert_resource(schedules);
    Ok(())
}

/// Split a rendered ambiguity report into individual "-- A and B\n
/// conflict on: X" entries, dropping pairs whose conflict set is ONLY
/// the exclusive World access. Returns "" when nothing remains.
fn filter_world_only_conflicts(rendered: &str) -> String {
    let mut kept = Vec::new();
    let mut lines = rendered.lines();
    while let Some(line) = lines.next() {
        if line.trim_start().starts_with("-- ") {
            let conflict_line = lines.next().unwrap_or("");
            let is_world_only = conflict_line.contains("conflict on:")
                && conflict_line
                    .trim_start()
                    .trim_start_matches("conflict on:")
                    .trim()
                    == "bevy_ecs::world::World";
            if !is_world_only {
                kept.push(format!("{}\n{}", line, conflict_line));
            }
        }
    }
    kept.join("\n")
}

/// Run condition: the live scenario uses this bot type.
fn bot_is(want: crate::enums::BotType) -> impl Fn(Option<Res<ScenarioResource>>) -> bool + Clone {
    move |scenario: Option<Res<ScenarioResource>>| {
        scenario.is_some_and(|s| s.0.bot.bot_type == want)
    }
}

/// Finish plugin building exactly like App::run would: App::update()
/// does NOT call finish()/cleanup(), so plugins doing work in
/// Plugin::finish never complete in test Apps. Idempotent: skips when
/// the caller already finished.
pub(crate) fn finish_plugins(app: &mut App) {
    use bevy::app::PluginsState;
    while app.plugins_state() == PluginsState::Adding {
        bevy::tasks::tick_global_task_pools_on_main_thread();
    }
    if app.plugins_state() == PluginsState::Ready {
        app.finish();
        app.cleanup();
    }
}

// cj8.14 (E2): wall-clock elapsed of the most recent run loop on
// this thread (drained into PlaytestReport.wall_time_s).
thread_local! {
    static WALL_TIMER: std::cell::RefCell<Option<f64>> = const { std::cell::RefCell::new(None) };
}

/// Run one scenario against an already-built `App`.
///
/// The app must contain `PlaytestPlugin` (add it to your test app, or
/// use [`crate::headless::headless_app`], which wires it for you).
/// The returned report carries status, violations (rule names from the
/// [`crate::rules`] registry), coverage, and the frame-indexed action
/// log. Reports are also written to
/// `target/bevy_playtest/runs/<run-id>/report.json`.
///
/// ## Example
/// ```no_run
/// use bevy::prelude::*;
/// use bevy_playtest::driver::{run_scenario, PlaytestPlugin};
/// use bevy_playtest::scenario::Scenario;
///
/// # fn game_app() -> App { unimplemented!() }
/// let scenario: Scenario = serde_json::from_str(
///     r#"{"bot":{"type":"chaos","seed":7},"duration_s":0.5,
///        "invariants":[{"name":"hp","rule":"custom","path":"TestApi.hp",
///                       "check":"above","value":10}]}"#
/// ).unwrap();
/// let mut app = game_app();
/// app.add_plugins(PlaytestPlugin);
/// let report = run_scenario(&mut app, &scenario).unwrap();
/// eprintln!("status: {:?}", report.status);
/// for v in &report.violations {
///     eprintln!("{}: {} (frame {})", v.rule, v.detail, v.first_frame);
/// }
/// ```
/// Z1 rule 3: minimum audited actions before the vacuity guard
/// (harness_could_not_drive_game) may fire.
pub const VACUITY_MIN_ACTIONS: u64 = 20;

pub fn run_scenario(app: &mut App, scenario: &Scenario) -> Result<PlaytestReport, ScenarioError> {
    validate_scenario(scenario)?;

    // Z14: enter deterministic entropy run. RF10: SKIP the re-enter
    // when InProcess already entered on this thread — re-entering
    // resets the stream to its start, making plugin-build draws and
    // the first run-time draws identical (UUID collisions).
    #[cfg(feature = "deterministic-entropy")]
    {
        crate::entropy::reset_fallback_counter();
        if !crate::entropy::in_deterministic_run() {
            crate::entropy::enter_run(scenario.bot.seed);
        }
    }
    // FX3: Drop guard ensures exit_run runs even on early returns/panics.
    #[cfg(feature = "deterministic-entropy")]
    struct EntropyGuard;
    #[cfg(feature = "deterministic-entropy")]
    impl Drop for EntropyGuard {
        fn drop(&mut self) {
            crate::entropy::exit_run();
        }
    }
    #[cfg(feature = "deterministic-entropy")]
    let _entropy_guard = EntropyGuard;

    // Z7: route Bevy errors/logs/panics from this run into a private sink
    // keyed by thread-local RunId, drained into Violations below. Install
    // the panic hook exactly once per process (chains to previous). A
    // lock-free compare_exchange makes concurrent first-use from
    // parallel workers safe: only the winner installs the hook.
    // ml5 (M5): the previous hook is captured AT INSTALL TIME — if
    // another framework swaps hooks after run_scenario, our capture
    // loses theirs silently. Accepted (low practical risk).
    static INSTALLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if INSTALLED
        .compare_exchange(
            false,
            true,
            std::sync::atomic::Ordering::AcqRel,
            std::sync::atomic::Ordering::Acquire,
        )
        .is_ok()
    {
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            crate::sinks::swarm_panic_hook_inner(info, &prev);
        }));
        INSTALL_COUNT.fetch_add(1, std::sync::atomic::Ordering::Release);
    }
    let run_id = crate::sinks::RunId::new();
    // Log capture: ensure the global subscriber (with our capture
    // layer) exists before the run emits any events.
    crate::sinks::ensure_global_subscriber();
    crate::sinks::set_current_run(run_id);
    // dde: also set the process-global run so bevy_tasks pool threads
    // (IoTaskPool, AsyncComputeTaskPool — no thread-local run id)
    // attribute panics/logs/errors to this run.
    crate::sinks::set_global_run(run_id);
    // le3.35 (RF45): registration guard created IMMEDIATELY — the
    // fallible steps below (finish_plugins, single_threaded,
    // ambiguity, contract resets, compile()) all `?`/return Err
    // BEFORE HandlerGuard exists, leaving the dead run id in
    // CURRENT_RUN/GLOBAL_RUNS and a HANDLER_CHAIN entry (contradicting
    // the comment above). Drop clears everything on ANY exit path;
    // the later HandlerGuard repeats the clears (idempotent) plus the
    // handler restore.
    struct RunRegGuard(crate::sinks::RunId);
    impl Drop for RunRegGuard {
        fn drop(&mut self) {
            crate::sinks::clear_current_run();
            crate::sinks::clear_global_run(self.0);
            crate::sinks::clear_previous_handler(self.0);
        }
    }
    let _run_reg_guard = RunRegGuard(run_id);
    // le3.11 (RF7): the swarm error handler and the run-id are BOTH
    // guarded by a single drop guard — a scenario rejected after the
    // handler is installed no longer leaves it half-installed (the
    // next run on the same App captured nothing). Drop restores the
    // previous handler and clears CURRENT_RUN on every path.

    // Finish plugin building exactly like App::run (Plugin::finish
    // hooks: render app links, late registrations).
    finish_plugins(app);

    // Ambiguity gate: configure all schedules to error on conflicts.
    if scenario.deny_ambiguities {
        app.configure_schedules(ScheduleBuildSettings {
            ambiguity_detection: LogLevel::Error,
            ..Default::default()
        });
    }

    // Force single-threaded execution on all schedules.
    if scenario.single_threaded {
        force_single_threaded(app)?;
    }

    // Ambiguity gate (finalize): force schedule initialization NOW so
    // ambiguity errors surface as a structured rejection instead of a
    // panic inside Schedule::run during the update loop.
    if scenario.deny_ambiguities {
        // qa8.16: hand-built minimal Apps may lack Schedules —
        // structured rejection instead of a panic.
        let Some(mut schedules) = app
            .world_mut()
            .remove_resource::<bevy::ecs::schedule::Schedules>()
        else {
            return Err(ScenarioError::Rejected(
                "deny_ambiguities: Schedules not found in app".into(),
            ));
        };
        let mut errs = Vec::new();
        for (_label, sched) in schedules.iter_mut() {
            if let Err(e) = sched.initialize(app.world_mut()) {
                let rendered = e.to_string(sched.graph(), app.world_mut());
                // Exclusive systems (&mut World) conflict with EVERYTHING by
                // construction — they serialize at execution time, so their
                // "ambiguity" carries no ordering hazard. Engine bookkeeping
                // systems tick_global_task_pools / message_update_system /
                // despawn_unused_registered_systems / update_frame_count are
                // exclusive too. Filter conflicts whose ONLY conflict is
                // World access; data-access ambiguities still fail the gate.
                let data_conflicts = filter_world_only_conflicts(&rendered);
                if !data_conflicts.is_empty() {
                    errs.push(data_conflicts);
                }
            }
        }
        app.world_mut().insert_resource(schedules);
        if !errs.is_empty() {
            return Err(ScenarioError::Rejected(format!(
                "schedule ambiguities: {}",
                errs.join("; ")
            )));
        }
    }

    // D1: a resolver is OPTIONAL (zero-contract percepts work alone).
    // Only a scenario that references TestApi.* paths while no resolver
    // is registered fails, at load time, with guidance.
    let needs_test_api = scenario.invariants.iter().any(|inv| {
        inv.path
            .as_deref()
            .or(inv.requires_path.as_deref())
            .is_some_and(|p| p.starts_with("TestApi."))
    }) || (matches!(scenario.bot.bot_type, crate::enums::BotType::Planner)
        // FX12 (I3): a planner run only needs a TestApi resolver if
        // the goal tree contains TestApi Primitives — predicate-only
        // trees (typed Bevy systems) work without any resolver.
        && scenario
            .bot
            .goals
            .as_ref()
            .is_some_and(crate::planner::goal_tree_needs_resolver));
    if needs_test_api
        && app
            .world()
            .get_resource::<crate::contract::TestApiResolver>()
            .is_none()
        && app.world().get_resource::<TestApi>().is_none()
    {
        return Err(ScenarioError::ContractMissing(
            "scenario references TestApi.* paths (or uses the planner bot) but no resolver is registered — \
             call register_test_api::<YourApi>() or add TestConventionsPlugin (or use Resource:/Component: percept paths)"
                .to_string(),
        ));
    }

    // FX12 (I3): validate typed Predicate names at LOAD time against
    // the TypedRegistry (a typo'd name otherwise fails silently as
    // "never achieved").
    if let Some(goals) = scenario.bot.goals.as_ref() {
        if let Some(registry) = app.world().get_resource::<crate::typed::TypedRegistry>() {
            let names = registry.predicate_names();
            crate::planner::validate_predicate_names(goals, &|n| names.contains(&n.to_string()))
                .map_err(ScenarioError::Rejected)?;
        }
    }

    // ResetHooks: required only if the scenario declares resets — a game
    // with no persistent state may legitimately register none.
    let known_kinds: Vec<String> = match app.world().get_resource::<ResetHooks>() {
        Some(hooks) => hooks.0.keys().cloned().collect(),
        None => Vec::new(),
    };
    for reset in &scenario.setup.resets {
        if !known_kinds.contains(&reset.kind) {
            return Err(ScenarioError::UnknownReset {
                kind: reset.kind.clone(),
                known: known_kinds.clone(),
            });
        }
    }

    // Validate scheduled cheats (reject unknown kinds — never run).
    let known_cheats = match app.world().get_resource::<crate::contract::CheatHooks>() {
        Some(h) => h.known_kinds(),
        None => vec![],
    };
    for cheat in &scenario.setup.cheats {
        if !known_cheats.contains(&cheat.kind.as_str()) {
            return Err(ScenarioError::UnknownCheat {
                kind: cheat.kind.clone(),
                known: known_cheats.iter().map(|s| s.to_string()).collect(),
            });
        }
    }

    // I3: typed-oracle name validation (unknown names reject at load).
    crate::typed::validate_oracle_names(app.world(), scenario)?;
    // I3: custom bot-policy name validation.
    crate::typed::validate_policy_name(app.world(), scenario)?;

    // I2: compile invariants once — unknown component/query types
    // reject at LOAD time, before any frame runs.
    // Install the swarm error handler by REPLACING the
    // FallbackErrorHandler resource directly (never via
    // App::set_error_handler, which panics if the game already called
    // it and would be skipped if the game inserted the resource
    // itself). The previous handler is remembered per-run so the
    // wrapper can record the error and then chain to the game's
    // handler. Parallel runs keep separate chains via thread-local
    // CURRENT_RUN routing in sinks.
    use bevy::ecs::error::ErrorContext;
    if let Some(prev_resource) = app
        .world()
        .get_resource::<bevy::ecs::error::FallbackErrorHandler>()
    {
        crate::sinks::set_previous_handler(run_id, prev_resource.0);
    } else {
        // No game handler: chain to Bevy's severity-matched default so
        // panic-severity errors still unwind and warnings still log.
        crate::sinks::set_previous_handler(run_id, bevy::ecs::error::match_severity);
    }

    // le3.21 (RF25): COMPILE FIRST. compile() performs load-time
    // validation and may reject (component typos, bad paths). A
    // rejected run must not leave the swarm error handler
    // half-installed on the App — previously the handler went in
    // before compile(), so a rejected run kept it installed and
    // leaked a HANDLER_CHAIN entry keyed by the dead run id.
    let compiled = crate::compiled::compile(app.world_mut(), scenario)?;

    // le3.11 (RF7): DROP-GUARDED handler install. `HandlerGuard`
    // restores the previous FallbackErrorHandler and clears
    // CURRENT_RUN on drop — including every early-return Err path
    // (UnknownReset, UnknownCheat, ContractMissing, ambiguity
    // rejections). The old SwarmHandlerRestore-resource scheme left
    // the wrapper installed after a rejected run, so the NEXT run on
    // the same App captured zero errors.
    #[derive(bevy::ecs::resource::Resource)]
    struct SwarmHandlerRestore {
        prev: Option<bevy::ecs::error::FallbackErrorHandler>,
    }
    let prev_resource = app
        .world_mut()
        .remove_resource::<bevy::ecs::error::FallbackErrorHandler>();
    let already_wrapped = app.world().get_resource::<SwarmHandlerRestore>().is_some();
    if !already_wrapped {
        fn swarm_error_handler(mut err: bevy::ecs::error::BevyError, ctx: ErrorContext) {
            let is_panic = matches!(err.severity(), bevy::ecs::error::Severity::Panic);
            // Capture first (severity/context/message), then chain.
            crate::sinks::capture_bevy_error(&err, &ctx);
            if is_panic {
                if let Some(payload) = err.take_payload() {
                    std::panic::resume_unwind(payload);
                }
            }
            // Chain to the game's handler last: it consumes err.
            if let Some(prev_fn) = crate::sinks::chained_previous_public() {
                prev_fn(err, ctx);
            }
        }
        app.insert_resource(bevy::ecs::error::FallbackErrorHandler(swarm_error_handler));
        app.insert_resource(SwarmHandlerRestore {
            prev: prev_resource,
        });
    }
    struct HandlerGuard(crate::sinks::RunId);
    impl Drop for HandlerGuard {
        fn drop(&mut self) {
            // Runs on success AND on every early Err return AFTER
            // the install point. load-time validation (compile) now
            // happens BEFORE the install, so a rejected run leaves
            // nothing behind. Clears both per-thread and global run
            // registration so the handler resolves to nothing after
            // the run.
            crate::sinks::clear_current_run();
            crate::sinks::clear_global_run(self.0);
            // le3.21 (RF25): also drop the run's HANDLER_CHAIN entry
            // — otherwise rejected/finished runs leak chain slots and
            // the dead run id can shadow the app's own handler.
            crate::sinks::clear_previous_handler(self.0);
        }
    }
    let _handler_guard = HandlerGuard(run_id);

    // RF8: populate declared_keys from scenario.bot.raw_surface (tier-0
    // explicit declarations). Default-surface keys stay unmarked.
    if let Some(raw) = &scenario.bot.raw_surface {
        let mut eff = app
            .world_mut()
            .resource_mut::<crate::effects::ActionEffects>();
        for key in &raw.keys {
            eff.declared_keys.insert(key.clone());
        }
    }
    app.insert_resource(compiled);

    let tps = scenario.tps as u64;
    if scenario.simulated_time {
        // Deterministic clock: Time advances by exactly 1/tps per update,
        // independent of host speed.
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_secs_f64(1.0 / tps as f64),
        ));
        // swarm-dt0: prime Time<Real> BEFORE the first update. Vanilla
        // Bevy starts Time<Real> with no last_update, so the first
        // app.update() computes delta = startup->now — frame 1 gets a
        // zero/near-zero delta while frames 2..N advance the full
        // manual delta. Priming makes frame 1 uniform with the rest
        // (port of bevy_headless_test's prime_time).
        {
            let now = std::time::Instant::now();
            let mut world = app.world_mut();
            // Apps without a time plugin have no Time<Real>; insert it
            // so priming works everywhere (update_with_instant then
            // behaves like the first ManualDuration tick).
            if let Some(mut t) = world.get_resource_mut::<bevy::time::Time<bevy::time::Real>>() {
                t.update_with_instant(now);
            } else {
                let mut t = bevy::time::Time::<bevy::time::Real>::default();
                t.update_with_instant(now);
                world.insert_resource(t);
            }
        }
    }
    app.insert_resource(ScenarioResource(std::sync::Arc::new(scenario.clone())));

    // Seed propagation: games with randomness read this resource to seed
    // their generators, making chaos runs reproducible.
    app.insert_resource(crate::contract::ScenarioSeed(scenario.bot.seed));

    // Re-run support: fresh Violations and ActionLog for each call to
    // run_scenario on the same App (so repeated runs don't accumulate
    // stale violations/logs). The ActionLog may be game-owned; we slice
    // it later to include only entries since the start index.
    app.insert_resource(crate::state::Violations::default());
    app.insert_resource(crate::contract::ActionLog::default());

    // NOTE: no add_systems here anymore. All harness systems (bots,
    // oracles, cheat scheduler) are registered once by PlaytestPlugin in
    // explicit PlaytestSet phases; each bot's system early-returns when
    // the active bot type is not its own. This also makes calling
    // run_scenario twice on the same App safe (no duplicate systems).
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
    // I1: reset identity index then RE-INDEX existing Gameplay entities
    // (otherwise entities from build/prev run vanish and next spawn
    // duplicates StableId(0)). Sort by Entity bits for determinism.
    if app
        .world()
        .contains_resource::<crate::identity::IdentityIndex>()
    {
        // resource_scope lets us borrow world immutably for the query
        // and mutably for the index simultaneously.
        app.world_mut().resource_scope(
            |world, mut idx: bevy::ecs::change_detection::Mut<crate::identity::IdentityIndex>| {
                let mut q = world.query_filtered::<(
                    bevy::ecs::entity::Entity,
                    &crate::identity::StableId,
                    Option<&bevy::prelude::Name>,
                ), With<crate::contract::Gameplay>>();
                let mut entities: Vec<_> = q
                    .iter(world)
                    .map(|(e, id, n)| (e, *id, n.cloned()))
                    .collect();
                entities.sort_by_key(|(e, _, _)| *e);
                idx.reset();
                for (entity, id, name) in entities {
                    idx.reinsert(id, entity, name.as_ref());
                }
            },
        );
    }
    let hooks = app
        .world_mut()
        .remove_resource::<ResetHooks>()
        .unwrap_or_default();
    for k in &kinds {
        if let Some(hook) = hooks.0.get(k) {
            // FX6: reset hooks run inside a panic boundary — a panicking
            // hook is reported (reset_hook_panicked) instead of escaping
            // run_scenario entirely.
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                hook(app.world_mut());
            }))
            .unwrap_or_else(|_| {
                if let Some(mut violations) = app.world_mut().get_resource_mut::<Violations>() {
                    violations.report(
                        crate::rules::Rule::ResetHookPanicked,
                        k,
                        format!("reset hook '{}' panicked", k),
                        0,
                    );
                }
            });
        }
    }
    app.insert_resource(hooks);

    let mut state = PlaytestState::new(tps, scenario.bot.seed);
    state.coverage = coverage;
    app.insert_resource(state);
    // J0: the bot's choice stream — replay prefix, continuation, and
    // recording of every draw for persistence/mutation/shrinking.
    let choice_stream = crate::choice::ChoiceStream::from_seed(scenario.bot.seed).with_replay(
        scenario.bot.choices.clone().unwrap_or_default(),
        scenario.bot.continuation,
    );
    app.insert_resource(choice_stream);
    // J1: robustness accumulation (thresholds, bounds, differentials...).
    app.init_resource::<crate::robustness::RobustnessTracker>();

    // 716.35: CEIL to match validate_scenario — truncation ran ZERO
    // frames for duration_s=0.016 @ tps=60 (0.96 ticks), silently
    // vacuous-passing validated scenarios. Semantics decided and
    // documented: a fractional tick count rounds UP — the partial
    // frame RUNS (ceil), consistent with validation's cap math; a
    // scenario is never silently zero-length after passing
    // validation.
    let total_ticks = (scenario.duration_s * tps as f32).ceil() as u64;
    // Pre-run snapshot for code-aware (system) coverage — see
    // `system_coverage`. Must happen after finish/cleanup so schedules
    // are initialized and enumerable.
    let systems_before = crate::state::snapshot_systems_with_tick(app.world_mut());

    // Names of FrameTimeAnomaly invariants armed by this scenario; the
    // wall-clock oracle only records/reports when at least one is armed.
    let scenario_frame_rules: Vec<String> = scenario
        .invariants
        .iter()
        .filter(|inv| matches!(inv.rule, crate::enums::InvariantRule::FrameTimeAnomaly))
        .map(|inv| inv.name.clone())
        .collect();

    let pre_ready_frames = std::cell::Cell::new(0u64);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        // Readiness gate (UE IsReady analog), INSIDE the panic boundary:
        // a panic while loading must yield status crash. Games that never
        // insert GameReady get immediate readiness. When the gate opens,
        // frame counters are RESET so after_s, replay frames and
        // duration_s count from readiness.
        while app
            .world_mut()
            .get_resource::<GameReady>()
            .is_some_and(|r| !r.0)
        {
            app.update();
            pre_ready_frames.set(pre_ready_frames.get() + 1);
            if pre_ready_frames.get() > tps * 30 {
                let mut violations = app
                    .world_mut()
                    .get_resource_mut::<Violations>()
                    .expect("Violations initialized by plugin");
                violations.report(
                    crate::rules::Rule::ReadinessTimeout,
                    "world",
                    "game did not become ready within 30s (GameReady never set true)".into(),
                    0,
                );
                break;
            }
        }
        // Gate opened: reset per-scenario counters so the scenario's
        // frame numbering starts at readiness.
        if pre_ready_frames.get() > 0 {
            let mut state = app
                .world_mut()
                .get_resource_mut::<PlaytestState>()
                .expect("PlaytestState inserted above");
            state.frame = 0;
            state.frozen_frames = 0;
            state.eventually_state.clear();
        }
        let junit_wall_start = std::time::Instant::now();
        WALL_TIMER.with(|t| *t.borrow_mut() = Some(junit_wall_start.elapsed().as_secs_f64()));
        for frame_idx in 0..total_ticks {
            let t0 = std::time::Instant::now();
            // FX5 Z7: expose the current frame to the error/log sinks
            // (system-level capture runs during app.update()).
            crate::sinks::set_current_frame(frame_idx);
            app.update();
            let ms = t0.elapsed().as_secs_f64() * 1000.0;
            WALL_TIMER.with(|t| *t.borrow_mut() = Some(junit_wall_start.elapsed().as_secs_f64()));
            // Wall-clock frame timing (simulated time makes Time.delta()
            // constant; oracles need the actual wall duration). Opt-in via
            // a FrameTimeAnomaly invariant, reported under inv.name.
            if !scenario_frame_rules.is_empty() {
                let mut anomaly_note: Option<String> = None;
                {
                    let world = app.world_mut();
                    if let Some(mut ps) = world.get_resource_mut::<PlaytestState>() {
                        if ps.frame_timing.is_anomalous(ms) {
                            ps.frame_timing.anomalies += 1;
                            anomaly_note = Some(format!(
                                "frame took {:.2}ms — 3.0σ above running mean ({:.2}ms, worst {:.2}ms)",
                                ms,
                                ps.frame_timing.mean_ms(),
                                ps.frame_timing.worst_ms
                            ));
                        }
                        ps.frame_timing.observe(ms);
                    }
                }
                if let Some(note) = anomaly_note {
                    // FX12 (A1): use state.frame (simulated time counter)
                    // instead of frame_idx (wall-clock loop index).
                    let frame = app
                        .world()
                        .get_resource::<PlaytestState>()
                        .map(|s| s.frame)
                        .unwrap_or(0);
                    if let Some(mut violations) = app.world_mut().get_resource_mut::<Violations>() {
                        for name in &scenario_frame_rules {
                            // Report under the dedicated informational rule
                            // (excluded from fingerprints/digests); the
                            // user's invariant name rides in the target.
                            violations.report(
                                crate::rules::Rule::FrameTimeAnomaly,
                                name,
                                note.clone(),
                                frame,
                            );
                        }
                    }
                }
            }
        }
    }));

    // RF38: finalize the accumulated rendering-check verdicts
    // (no_active_camera / nothing_on_screen) while the world is
    // still alive.
    crate::render_check::finalize_render_checks(app.world_mut());

    // swarm-uz8: finalize the pass_through verdicts.
    crate::pass_through::finalize_pass_through(app.world_mut());

    // Unwind-safety net: an exclusive system that panicked mid-flight
    // (planner, cheat scheduler) may have removed a resource it was
    // about to re-insert. Restore missing harness-critical resources
    // so report extraction below and subsequent frames keep working;
    // violations recorded before the panic are lost (reported via
    // cheat_hook_panicked / reset_hook_panicked rules when detectable).
    if app.world().get_resource::<Violations>().is_none() {
        app.world_mut().insert_resource(Violations::default());
    }
    if app
        .world()
        .get_resource::<crate::contract::CheatHooks>()
        .is_none()
    {
        app.world_mut()
            .insert_resource(crate::contract::CheatHooks::default());
    }
    if app
        .world()
        .get_resource::<crate::typed::TypedRegistry>()
        .is_none()
    {
        app.world_mut()
            .insert_resource(crate::typed::TypedRegistry::default());
    }

    // le3.34 (RF44): reconcile eventually rules whose deadline
    // extends past the run duration (otherwise overall<0 with no
    // violation — J1 broken). Must run BEFORE PlaytestState is
    // removed below.
    crate::oracles::finalize_eventually_rules(app.world_mut());
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
        s.pre_ready_frames = pre_ready_frames.get();
        s
    };

    // Planner end-of-run check: if the goal stack is not EMPTY, the top
    // unfinished primitive never achieved its condition — a goal that
    // was pursued but not reached (progress-stall bug signature).
    let mut snap = violations.snapshot();

    // FX3: emit informational rule for unattributed entropy draws.
    #[cfg(feature = "deterministic-entropy")]
    {
        let count = crate::entropy::fallback_draw_count();
        if count > 0 {
            snap.push(crate::state::ViolationEntry {
                rule: crate::rules::Rule::EntropyUnattributedDraws.to_string(),
                target: "".to_string(),
                first_frame: 0,
                last_frame: state.frame,
                count: count as u64,
                detail: format!("{} unattributed entropy draw(s) outside enter_run", count),
                last_detail: format!("{} unattributed entropy draw(s) outside enter_run", count),
                fingerprint: None,
                fingerprint_scheme: 1,
            });
        }
    }

    // Z7: drain the error/log/panic sink into the snapshot as
    // bevy_error / log_error / panic violations. Dedupe by
    // (rule, context) via Violations semantics.
    // dde: the run is over, but bevy_tasks pool work the game
    // detached may STILL be executing on process-global pool
    // threads. Give straggler panics a brief grace period to land in
    // the sink BEFORE we clear the global run id and drain —
    // otherwise a panic landing after the drain is lost (or worse,
    // attributed to the NEXT run via GLOBAL_RUN).
    drain_pending_pool_work();
    crate::sinks::clear_current_run();
    crate::sinks::clear_global_run(run_id);
    // Restore the game's original FallbackErrorHandler so a subsequent
    // run on the same App (with a possibly different scenario) doesn't
    // inherit the swarm wrapper.
    if let Some(restore) = app
        .world_mut()
        .remove_resource::<SwarmHandlerRestore>()
        .map(|r| r.prev)
    {
        app.insert_resource(restore.unwrap_or_default());
    }
    crate::sinks::clear_previous_handler(run_id);
    // Z7: group sink events by (rule, target, normalized message) —
    // an error firing every frame must yield ONE entry with a count
    // and first/last frame span, not one entry per occurrence.
    type SinkGroups = std::collections::BTreeMap<(String, String, String), (String, u64, u64, u64)>;
    let mut sink_groups: SinkGroups = std::collections::BTreeMap::new();
    for captured in crate::sinks::drain_run(run_id) {
        let (rule, target, message, frame) = match captured {
            crate::sinks::Captured::BevyError {
                severity,
                context,
                message,
                frame,
            } => {
                // le3.11 (RF7): Suppressed (Ignore-severity) errors
                // surface in the report as an INFORMATIONAL rule
                // (bevy_suppressed) — never gating, but visible
                // end-to-end instead of dropped at the drain.
                if severity < crate::sinks::Severity::Warning {
                    if severity == crate::sinks::Severity::Suppressed {
                        sink_groups
                            .entry((
                                crate::rules::Rule::BevySuppressed.name().to_string(),
                                context.clone(),
                                normalize_sink_message(&message),
                            ))
                            .or_insert((message.clone(), frame, frame, 0));
                        let e = sink_groups
                            .get_mut(&(
                                crate::rules::Rule::BevySuppressed.name().to_string(),
                                context.clone(),
                                normalize_sink_message(&message),
                            ))
                            .unwrap();
                        e.1 = e.1.min(frame);
                        e.2 = e.2.max(frame);
                        e.3 += 1;
                    }
                    continue;
                }
                let rule = if severity >= crate::sinks::Severity::Error {
                    crate::rules::Rule::BevyError.name()
                } else {
                    crate::rules::Rule::BevyWarning.name()
                };
                (rule.to_string(), context, message, frame)
            }
            crate::sinks::Captured::Log {
                level,
                target,
                message,
                frame,
            } => {
                if !matches!(level, bevy::log::Level::ERROR | bevy::log::Level::WARN) {
                    continue;
                }
                // Allow-list: expected harness noise in multi-App processes.
                if message.contains("Could not set global logger")
                    || message.contains("already set")
                {
                    continue;
                }
                let rule = if level == bevy::log::Level::ERROR {
                    crate::rules::Rule::LogError.name()
                } else {
                    crate::rules::Rule::LogWarn.name()
                };
                (rule.to_string(), target, message, frame)
            }
            crate::sinks::Captured::Panic {
                message,
                location,
                backtrace: _,
                system,
            } => {
                let panic_frame = message
                    .rfind("@frame ")
                    .and_then(|i| {
                        message[i + 7..]
                            .split(|c: char| !c.is_ascii_digit())
                            .next()
                            .and_then(|d| d.parse::<u64>().ok())
                    })
                    // Fall back to the frame the driver had advanced to
                    // when the panic fired (the sink's thread-local
                    // CURRENT_FRAME) instead of a hardcoded 0.
                    .unwrap_or_else(crate::sinks::current_frame);
                let target = system.unwrap_or_else(|| "world".to_string());
                let detail = match location {
                    Some(loc) => format!("{message} at {loc}"),
                    None => message.clone(),
                };
                (
                    crate::rules::Rule::Panic.name().to_string(),
                    target,
                    detail,
                    panic_frame,
                )
            }
        };
        let key = (
            rule.clone(),
            target.clone(),
            normalize_sink_message(&message),
        );
        let entry = sink_groups.entry(key).or_insert((message, frame, frame, 0));
        entry.1 = entry.1.min(frame);
        entry.2 = entry.2.max(frame);
        entry.3 += 1;
    }
    for ((rule, target, _norm), (message, first, last, count)) in sink_groups {
        snap.push(crate::state::ViolationEntry {
            rule,
            target,
            first_frame: first,
            last_frame: last,
            count,
            detail: format!("{message} (x{count})"),
            last_detail: message,
            fingerprint: None,
            fingerprint_scheme: 1,
        });
    }
    if let Some(v) = crate::planner::planner_unfinished_check(&state.planner, &state) {
        snap.push(v);
    }
    // FX5: ALL sources (oracles, sinks, planner) are now merged — apply
    // the total order + fingerprints to the combined list so sink and
    // planner entries are fingerprinted like every other violation
    // (they were skipped by sweep/autotest aggregation before).
    snap.sort_by(|a, b| {
        (a.first_frame, &a.rule, &a.target).cmp(&(b.first_frame, &b.rule, &b.target))
    });
    {
        let norm = crate::fingerprint::config_normalizer();
        for e in &mut snap {
            // le3.10 (RF6): informational rules are never fingerprinted
            // (Violations::snapshot already leaves them unfingerprinted;
            // this loop previously stamped frame_time_anomaly anyway).
            if crate::state::Violations::INFORMATIONAL_RULES
                .iter()
                .any(|r| r.name() == e.rule)
            {
                e.fingerprint = None;
                e.fingerprint_scheme = 1;
                continue;
            }
            if e.fingerprint.is_none() {
                let raw_detail = e
                    .detail
                    .strip_prefix("[intent: ")
                    .and_then(|rest| rest.find("] "))
                    .map(|end| &e.detail["[intent: ".len() + end + 2..])
                    .unwrap_or(e.detail.as_str());
                // le3.11 (RF7): strip the "(xN)" count suffix before
                // fingerprinting — the same error firing 10 vs 1000
                // frames must yield the same fingerprint.
                let raw_detail = if raw_detail.ends_with(')')
                    && raw_detail.contains("(x")
                    && raw_detail.rfind("(x").is_some_and(|i| {
                        raw_detail[i..].chars().skip(2).all(|c| c.is_ascii_digit())
                    }) {
                    &raw_detail[..raw_detail.rfind("(x").unwrap()]
                } else {
                    raw_detail
                };
                // Normalize digits in sink messages before
                // fingerprinting — the same error repeated 10 vs 1000
                // frames must yield the same fingerprint (dedupe stability).
                let normalized = normalize_sink_message(raw_detail);
                e.fingerprint = Some(norm.fingerprint(
                    "violation",
                    &e.rule,
                    std::slice::from_ref(&e.target),
                    &normalized,
                ));
                e.fingerprint_scheme = 1;
            }
        }
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
    // Informational frame-budget checks: fps_floor and
    // frame_time_p99_below invariants evaluate POST-RUN against the
    // measured wall-clock metrics. They never gate the run outcome —
    // reported under their dedicated informational rules (excluded
    // from fingerprints/digests, per U1 rule 9).
    if !crash_detected {
        // le3.10 (RF6): fps_floor uses WALL-CLOCK fps, not simulated tps.
        // The old formula always equaled tps (frame/duration_s) and never
        // responded to real frame cost. Use the WALL_TIMER which tracks
        // actual elapsed time of the run.
        let wall = crate::driver::WALL_TIMER
            .with(|t| t.borrow().unwrap_or(0.0))
            .max(f64::EPSILON);
        let measured_fps = state.frame as f64 / wall;
        for inv in &scenario.invariants {
            match inv.rule {
                crate::enums::InvariantRule::FpsFloor => {
                    let floor = inv.value.as_ref().and_then(|v| v.as_f64());
                    if let Some(floor) = floor.filter(|f| *f > 0.0) {
                        if measured_fps < floor {
                            snap.push(crate::state::ViolationEntry {
                                rule: crate::rules::Rule::FpsFloor.to_string(),
                                target: inv.name.clone(),
                                first_frame: 0,
                                last_frame: state.frame,
                                count: 1,
                                detail: format!(
                                    "measured {measured_fps:.1} fps < floor {floor:.1}"
                                ),
                                last_detail: format!(
                                    "measured {measured_fps:.1} fps < floor {floor:.1}"
                                ),
                                fingerprint: None,
                                fingerprint_scheme: 1,
                            });
                        }
                    }
                }
                crate::enums::InvariantRule::FrameTimeP99Below => {
                    let ceiling_ms = inv.value.as_ref().and_then(|v| v.as_f64());
                    if let Some(ceil) = ceiling_ms.filter(|c| *c > 0.0) {
                        if final_metrics.frame_ms_p99 > ceil {
                            snap.push(crate::state::ViolationEntry {
                                rule: crate::rules::Rule::FrameTimeP99Below.to_string(),
                                target: inv.name.clone(),
                                first_frame: 0,
                                last_frame: state.frame,
                                count: 1,
                                detail: format!(
                                    "p99 {:.2}ms > ceiling {:.2}ms",
                                    final_metrics.frame_ms_p99, ceil
                                ),
                                last_detail: format!(
                                    "p99 {:.2}ms > ceiling {:.2}ms",
                                    final_metrics.frame_ms_p99, ceil
                                ),
                                fingerprint: None,
                                fingerprint_scheme: 1,
                            });
                        }
                    }
                }
                _ => {}
            }
        }
    }
    // dbt: strict_warnings escalates the warning-class informational
    // rules (bevy_warning, log_warn) back to gating severity.
    // Z1 rule 3: vacuity guard. Enough actions injected but ZERO effect
    // across all of them => the harness could not drive the game.
    // False-positive guard (spec): a tier-0 game with no observable
    // gameplay state (no Gameplay entities, no GameTypes) cannot
    // ATTRIButE effects at all — recorded as scope.inactive, not a
    // failure.
    let observable_gameplay = app.world().iter_entities().any(|e| {
        e.contains::<crate::contract::Gameplay>()
            || e.contains::<crate::game_types::InferredGameplay>()
    }) || app
        .world()
        .get_resource::<crate::game_types::GameTypes>()
        .is_some();
    if let Some(rate) = crate::effects::overall_effect_rate(app.world()) {
        let total_actions: u64 = app
            .world()
            .get_resource::<crate::effects::ActionEffects>()
            .map(|e| e.rates.values().map(|r| r.total).sum())
            .unwrap_or(0);
        if rate == 0.0 && total_actions >= VACUITY_MIN_ACTIONS && observable_gameplay {
            let top_keys: Vec<String> = app
                .world()
                .get_resource::<crate::effects::ActionEffects>()
                .map(|e| {
                    let mut v: Vec<(u64, String)> =
                        e.rates.iter().map(|(k, r)| (r.total, k.clone())).collect();
                    v.sort_by_key(|(total, _)| std::cmp::Reverse(*total));
                    v.into_iter().take(3).map(|(_, k)| k).collect()
                })
                .unwrap_or_default();
            let last_frame = app
                .world()
                .get_resource::<crate::state::PlaytestState>()
                .map(|s| s.frame)
                .unwrap_or(0);
            snap.push(ViolationEntry {
                rule: crate::rules::Rule::HarnessCouldNotDriveGame.to_string(),
                target: "harness".to_string(),
                first_frame: 0,
                last_frame,
                count: 1,
                detail: format!(
                    "{} actions injected (top: {}) but no effect on the game. Suggested \
                     causes: a menu needs a click the monkey can't find; input is gated \
                     behind a state; broken headless build. Next step: run surface \
                     discovery to see what inputs your game actually reacts to: \
                     `bevy_playtest::discover::discover_surface` (or `cargo bevy-playtest discover`) \
                     on your app builder, then target the discovered verbs in the \
                     scenario's bot config.",
                    total_actions,
                    top_keys.join(", ")
                ),
                last_detail: format!("{total_actions} actions injected with zero effect"),
                fingerprint: Some(crate::fingerprint::default_normalizer().fingerprint(
                    "vacuity",
                    crate::rules::Rule::HarnessCouldNotDriveGame.name(),
                    &[],
                    &format!("{total_actions} actions with zero effect"),
                )),
                fingerprint_scheme: 1,
            });
        }
    }

    let escalated: &[crate::rules::Rule] = if scenario.strict_warnings {
        &[crate::rules::Rule::BevyWarning, crate::rules::Rule::LogWarn]
    } else {
        &[]
    };
    let is_gating = |rule: &str| {
        !crate::state::Violations::INFORMATIONAL_RULES
            .iter()
            .any(|r| r.name() == rule)
            || escalated.iter().any(|r| r.name() == rule)
    };
    // le3.10 (RF6): informational rules NEVER gate the run status.
    // A run whose only violations are informational (fps_floor,
    // frame_time_*, dead_widget, perf_budget, entropy_unattributed)
    // is Pass.
    let gating = snap.iter().filter(|e| is_gating(&e.rule)).count();
    let status = if crash_detected {
        crate::enums::PlaytestStatus::Crash
    } else if gating == 0 {
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
    // Intent-audit entries that cannot round-trip to a ReplayIntent:
    // non-zero means the run is not fully minimizable/replayable.
    let unreplayable_actions = crate::scenario::action_log_to_timed_actions(&action_log)
        .iter()
        .filter(|ta| crate::minimize::action_to_replay_intent(ta).is_none())
        .count();
    let mut warnings = Vec::new();
    // FX12 (I4): liveness.mode: off is surfaced as a report warning.
    if let Some(state) = app.world().get_resource::<crate::harness::PlaytestState>() {
        if state.warned_liveness_off {
            warnings.push(
                "liveness.mode is off — frozen-world detection disabled for this run".to_string(),
            );
        }
        if state.warned_liveness_history_evicted {
            warnings.push(
                "liveness frame history evicted before an intent frame — stuck_after_intent degraded to the weaker frame heuristic".to_string(),
            );
        }
    }
    // FX12 (H1): one-time [idx] selector warning (thread-local from
    // resolve_world_percept).
    if let Some(w) = crate::oracles::take_index_selector_warning() {
        warnings.push(w);
    }
    if scenario.bot.bot_type == crate::enums::BotType::Replay && scenario.bot.inputs.is_empty() {
        warnings.push(
            "replay bot has an empty inputs list — the run does nothing;              record intents first (audit-log -> replay, B1)"
                .to_string(),
        );
    }
    if unreplayable_actions > 0 {
        warnings.push(format!(
            "{} audit-log intents cannot be converted to replay intents (Select on non-Gameplay entity?) — mark entities Gameplay for full replayability",
            unreplayable_actions
        ));
    }
    let game_version = app
        .world()
        .get_resource::<crate::contract::GameVersion>()
        .map(|v| v.0.clone());

    // Z1: checked scope — tier + oracle families + actuators.
    let scope = crate::diagnostics::compute_run_scope(app.world(), scenario);

    // Auto-minimization on crash is NOT done inline — the harness cannot
    // rebuild the game's App from inside run_scenario. Instead, the
    // caller uses `minimize_crash`, which takes an app builder and the
    // action log, applies ddmin, and returns the minimal reproducer
    // plus a ready-to-save regression scenario.

    Ok(PlaytestReport {
        schema_version: 1,
        status,
        violations: snap,
        metrics: final_metrics,
        coverage: state.coverage,
        frame_count: state.frame,
        tps: state.tps,
        simulated_duration_s: if state.tps == 0 {
            0.0
        } else {
            state.frame as f64 / state.tps as f64
        },
        wall_time_s: WALL_TIMER.with(|t| t.borrow_mut().take()).unwrap_or(0.0),
        pre_ready_frames: state.pre_ready_frames,
        error: result.err().map(|payload| {
            payload
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| format!("{:?}", payload))
        }),
        action_log,
        game_version,
        cheat_count,
        unreplayable_actions,
        warnings,
        system_coverage: crate::state::system_coverage_from_snapshots(
            &systems_before,
            app.world_mut(),
        ),
        state_trace: app
            .world_mut()
            .remove_resource::<crate::determinism::StateTrace>()
            .map(|t| t.digests),
        action_effect_rate: crate::effects::effect_rates(app.world()),
        choices: app
            .world_mut()
            .get_resource::<crate::choice::ChoiceStream>()
            .map(|c| c.recorded.clone())
            .unwrap_or_default(),
        robustness: app
            .world_mut()
            .get_resource::<crate::robustness::RobustnessTracker>()
            .map(|t| t.summary())
            .unwrap_or_default(),
        discovered_surface: None,
        scope,
        harness_mode: "in_process".to_string(),
    })
}

/// Z11/swarm-s5z: post-run golden-trajectory comparison. When a
/// scenario sets `compare_golden: Some(dir)`, compare the run's
/// per-frame state digests against the recorded golden entry
/// (`scenario-{name}-{seed}`) and report `golden_diverged` on the
/// first differing frame or a digest count mismatch. Mirrors
/// `check_scenario_determinism` (returns the base report with the
/// violation appended and status recomputed).
pub fn check_scenario_golden(scenario: &Scenario, base: PlaytestReport) -> PlaytestReport {
    let Some(dir) = scenario.compare_golden.as_deref() else {
        return base;
    };
    let mut report = base;
    let mut fail = |detail: String| {
        report.violations.push(crate::state::ViolationEntry {
            rule: crate::rules::Rule::GoldenDiverged.to_string(),
            target: "golden_trajectory".to_string(),
            first_frame: 0,
            last_frame: 0,
            count: 1,
            detail: detail.clone(),
            last_detail: detail,
            fingerprint: None,
            fingerprint_scheme: 1,
        });
    };
    let golden_set = match crate::golden::GoldenSet::load(dir) {
        Ok(set) => set,
        Err(e) => {
            fail(format!("golden set at '{}' could not be loaded: {e}", dir));
            report.status = crate::enums::PlaytestStatus::Fail;
            return report;
        }
    };
    let name = scenario.name.clone().unwrap_or_default();
    let entry_name = format!(
        "scenario-{}-{}",
        crate::golden::sanitize_entry_name(&name),
        scenario.bot.seed
    );
    let golden_path = golden_set.dir.join(format!("{entry_name}.golden.json"));
    let Ok(json) = std::fs::read_to_string(&golden_path) else {
        fail(format!(
            "no golden entry for '{entry_name}' in '{}' (scenario must set `name`)",
            dir
        ));
        report.status = crate::enums::PlaytestStatus::Fail;
        return report;
    };
    let Ok(golden) = serde_json::from_str::<crate::golden::GoldenFile>(&json) else {
        fail(format!("golden entry '{entry_name}' is not valid JSON"));
        report.status = crate::enums::PlaytestStatus::Fail;
        return report;
    };
    let current: Vec<String> = report
        .state_trace
        .iter()
        .flatten()
        .map(|d| format!("{:016x}", d.hash))
        .collect();
    if current.len() != golden.digests.len() {
        fail(format!(
            "digest count mismatch: golden {} frames, current {} frames",
            golden.digests.len(),
            current.len()
        ));
    } else {
        let mut first_diff: Option<u64> = None;
        for (idx, (stored, cur)) in golden.digests.iter().zip(current.iter()).enumerate() {
            if stored != cur {
                first_diff = Some(idx as u64);
                break;
            }
        }
        if let Some(frame) = first_diff {
            fail(format!(
                "state digest diverged from golden at frame {frame}"
            ));
        }
    }
    if report
        .violations
        .iter()
        .any(|v| v.rule == crate::rules::Rule::GoldenDiverged.to_string())
    {
        report.status = crate::enums::PlaytestStatus::Fail;
    }
    report
}

/// A4/swarm-kpl: post-run determinism check. When a scenario sets
/// `check_determinism: Some(N)`, run it N times (N >= 2) on FRESH Apps
/// from `app_builder`, compare per-frame state digests, and report the
/// `nondeterministic` rule on the first divergence. Returns the base
/// report with the extra violation appended (status recomputed).
///
/// Callers with an app factory (InProcess runner, fixture-runner)
/// should call this AFTER `run_scenario` when the flag is set.
pub fn check_scenario_determinism(
    app_builder: impl Fn() -> App + Send + Sync,
    scenario: &Scenario,
    base: PlaytestReport,
) -> PlaytestReport {
    let Some(runs) = scenario.check_determinism else {
        return base;
    };
    let det = match crate::determinism::check_determinism(app_builder, scenario, runs.max(2)) {
        Ok(d) => d,
        Err(_) => return base,
    };
    let mut report = base;
    if let Some(div) = det.first_divergence {
        let detail = if div.differing.is_empty() {
            format!(
                "cross-run state digest diverged at frame {} after {} runs",
                div.frame, runs
            )
        } else {
            let fields: Vec<String> = div
                .differing
                .iter()
                .take(5)
                .map(|(k, a, b)| format!("{}: {:?} != {:?}", k, a, b))
                .collect();
            format!(
                "cross-run state digest diverged at frame {} ({}): {}",
                div.frame,
                runs,
                fields.join(", ")
            )
        };
        report.violations.push(crate::state::ViolationEntry {
            rule: crate::rules::Rule::Nondeterministic.to_string(),
            target: "state_digest".to_string(),
            first_frame: div.frame,
            last_frame: div.frame,
            count: 1,
            detail: detail.clone(),
            last_detail: detail,
            fingerprint: None,
            fingerprint_scheme: 1,
        });
        report.violations.sort_by(|a, b| {
            (a.first_frame, &a.rule, &a.target).cmp(&(b.first_frame, &b.rule, &b.target))
        });
        if report.status == crate::enums::PlaytestStatus::Pass {
            report.status = crate::enums::PlaytestStatus::Fail;
        }
    }
    report
}

//-----------------------------------------------------------------------

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
        // restore. The hook runs inside a panic boundary — a panicking
        // cheat hook is reported (cheat_hook_panicked) instead of
        // unwinding through the scheduler and LOSING the remaining
        // hooks for later frames.
        let Some(mut hooks) = world.remove_resource::<crate::contract::CheatHooks>() else {
            continue;
        };
        if let Some(hook) = hooks.0.remove(&kind) {
            let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                hook(world);
            }))
            .is_err();
            // 716.34: take-run-put — re-insert the hook so a cheat
            // scheduled at frames 60 AND 120 executes BOTH times. The
            // old code logged both but executed once (audit log lied).
            hooks.0.insert(kind.clone(), hook);
            if panicked {
                world.resource_mut::<Violations>().report(
                    crate::rules::Rule::CheatHookPanicked,
                    &kind,
                    format!("cheat hook '{}' panicked", kind),
                    frame,
                );
            }
        }
        world.insert_resource(hooks);
    }
}
/// 6wt: how many times the process-wide panic hook installer has won
/// the compare_exchange (must be exactly 1 across the process life).
/// Asserted in tests/soundness.rs (16-thread race test).
static INSTALL_COUNT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

pub fn panic_hook_install_count() -> usize {
    INSTALL_COUNT.load(std::sync::atomic::Ordering::Acquire)
}

/// RF11: fill `Camera.computed.target_info` from the primary window
/// when bevy_render's camera_system is absent (headless runs with
/// RenderPlugin disabled). Without target_info, pointer picking and
/// `resolve_click_position` fail with NoViewportSize.
pub(crate) fn headless_camera_target_info_system(
    primary_window: Query<&bevy::window::Window, With<bevy::window::PrimaryWindow>>,
    mut cameras: Query<&mut bevy::camera::Camera>,
) {
    use bevy::camera::RenderTargetInfo;
    let Some(window) = primary_window.iter().next() else {
        return;
    };
    let info = RenderTargetInfo {
        physical_size: window.physical_size(),
        scale_factor: window.resolution.scale_factor(),
    };
    for mut camera in &mut cameras {
        // Only fill when bevy_render's camera_system hasn't already
        // (it sets target_info when RenderPlugin is present).
        if camera.computed.target_info.is_none() {
            camera.computed.target_info = Some(info.clone());
        }
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
                violations.set_context(format!("synthetic_keyboard:frame={},key={}", frame, key));
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
                    crate::rules::Rule::SyntheticKeyboardConfig,
                    &kp.key,
                    "unknown key name (see KeyPressInput docs for supported names)".to_string(),
                    frame,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests_716_34 {
    use super::*;
    use crate::contract::{ActionLog, CheatHooks};
    use bevy::prelude::*;
    use std::collections::HashMap;

    #[test]
    fn cheat_scheduled_twice_executes_both_times() {
        // 716.34: a cheat scheduled at frames 1 AND 2 must execute BOTH
        // times and the log must match executions (no lying audit log).
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.init_resource::<ActionLog>();
        app.init_resource::<Violations>();

        let counter = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let c = counter.clone();
        let hook: Box<dyn Fn(&mut World) + Send + Sync> = Box::new(move |_: &mut World| {
            c.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        });
        app.insert_resource(CheatHooks(HashMap::from([("my_cheat".to_string(), hook)])));

        let scenario: Scenario = serde_json::from_str(
            r#"{"bot":{"type":"replay","inputs":[]},"setup":{"cheats":[{"kind":"my_cheat","frame":1},{"kind":"my_cheat","frame":2}]}}"#,
        )
        .unwrap();
        app.insert_resource(ScenarioResource(std::sync::Arc::new(scenario)));
        app.insert_resource(PlaytestState::new(60, 1));

        // Frame 1.
        app.world_mut().resource_mut::<PlaytestState>().frame = 1;
        cheat_scheduler_system(app.world_mut());
        assert_eq!(
            counter.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "first execution"
        );

        // Frame 2 — the hook must STILL be registered (716.34).
        app.world_mut().resource_mut::<PlaytestState>().frame = 2;
        cheat_scheduler_system(app.world_mut());
        assert_eq!(
            counter.load(std::sync::atomic::Ordering::SeqCst),
            2,
            "second execution — hook must survive first use"
        );

        // Audit log matches executions.
        let log = app.world().resource::<ActionLog>();
        let n = log
            .entries
            .iter()
            .filter(|e| e.action.contains("cheat:my_cheat"))
            .count();
        assert_eq!(n, 2, "audit log must record BOTH executions");
    }
}

#[cfg(test)]
mod tests_716_35 {
    /// 716.35: duration_s 0.016 @ tps 60 = 0.96 ticks. Truncation ran
    /// ZERO frames (silent vacuous pass); ceil must run ONE frame,
    /// matching validate_scenario's tick-count math.
    #[test]
    fn fractional_tick_scenario_runs_at_least_one_frame() {
        let ticks = (0.016f32 * 60f32).ceil() as u64;
        assert_eq!(ticks, 1, "0.96 ticks must round up to 1 (ceil)");
        // Truncation (the old bug):
        assert_eq!(
            (0.016f32 * 60f32) as u64,
            0,
            "precondition: truncation yields 0"
        );

        // End-to-end: a validated scenario with fractional tick count
        // must run >= 1 frame and produce a non-vacuous report.
        let scenario: crate::scenario::Scenario = serde_json::from_str(
            r#"{"bot":{"type":"replay","inputs":[]},"duration_s":0.016,"tps":60,"invariants":[]}"#,
        )
        .unwrap();
        let mut app = crate::verify::fixtures::build_app_no_scoring();
        let rep = crate::driver::run_scenario(&mut app, &scenario).unwrap();
        assert!(
            rep.frame_count >= 1,
            "fractional-tick scenario must run at least 1 frame, ran {}",
            rep.frame_count
        );
    }
}
