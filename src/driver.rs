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
pub struct ScenarioResource(pub Scenario);

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
        app.init_resource::<Violations>();
        // I1: identity index + observers (idempotent).
        crate::identity::install_identity(app);
        // I3: typed oracle/predicate/policy registry.
        app.init_resource::<crate::typed::TypedRegistry>();

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
        app.init_resource::<crate::raw_input::ActiveKeyHolds>();
        app.init_resource::<crate::raw_input::ActiveMouseHolds>();
        app.init_resource::<crate::raw_input::VirtualGamepad>();
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
            app.add_message::<bevy::input::gamepad::RawGamepadButtonChangedEvent>();
            app.add_message::<bevy::input::gamepad::RawGamepadAxisChangedEvent>();
        }

        // Raw input actuator (Z3): keyboard/gamepad injection in PreUpdate,
        // before Bevy's InputSystems so ButtonInput reflects them same-frame.
        app.add_systems(
            bevy::app::PreUpdate,
            crate::raw_input::raw_input_preupdate_system.after(PlaytestSet::Bots),
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
                check_bounds_gameplay_system,
                check_custom_system,
                // I3: typed oracles after the JSON-DSL checker.
                crate::typed::run_typed_oracles,
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
        }
    }
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
fn force_single_threaded(app: &mut App) {
    let mut schedules = app
        .world_mut()
        .remove_resource::<bevy::ecs::schedule::Schedules>()
        .unwrap();
    for (_label, sched) in schedules.iter_mut() {
        sched.set_executor(SingleThreadedExecutor::new());
    }
    app.world_mut().insert_resource(schedules);
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

/// Run condition: the live scenario arms this invariant rule.
#[allow(dead_code)]
fn has_rule(
    want: crate::enums::InvariantRule,
) -> impl Fn(Option<Res<ScenarioResource>>) -> bool + Clone {
    move |scenario: Option<Res<ScenarioResource>>| {
        scenario.is_some_and(|s| s.0.invariants.iter().any(|i| i.rule == want))
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

pub fn run_scenario(app: &mut App, scenario: &Scenario) -> Result<PlaytestReport, ScenarioError> {
    validate_scenario(scenario)?;

    // Z14: enter deterministic entropy run (spawn a fresh thread if feature enabled).
    #[cfg(feature = "deterministic-entropy")]
    {
        crate::entropy::reset_fallback_counter();
        crate::entropy::enter_run(scenario.bot.seed);
    }

    // Z7: route Bevy errors/logs/panics from this run into a private sink
    // keyed by thread-local RunId, drained into Violations below. Install
    // the panic hook once per process (chains to previous).
    static mut PANIC_HOOK_INSTALLED: bool = false;
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    {
        let _guard = LOCK.lock().unwrap();
        if !unsafe { PANIC_HOOK_INSTALLED } {
            unsafe { PANIC_HOOK_INSTALLED = true };
            let prev = std::panic::take_hook();
            std::panic::set_hook(Box::new(move |info| {
                crate::sinks::swarm_panic_hook_inner(info, &prev);
            }));
        }
    }
    let run_id = crate::sinks::RunId::new();
    crate::sinks::set_current_run(run_id);
    // Install the swarm error handler once per App (App::set_error_handler
    // panics on a second call). A marker resource tracks ownership; the
    // handler reads the thread-local CURRENT_RUN so parallel runs stay
    // separated. For Error-severity BevyErrors, chain to Bevy's panic
    // handler so panics propagate as crashes.
    #[derive(bevy::ecs::resource::Resource)]
    struct SwarmErrorHandlerInstalled;
    if app
        .world()
        .get_resource::<SwarmErrorHandlerInstalled>()
        .is_none()
    {
        app.set_error_handler(|mut err, ctx| {
            let is_panic = matches!(err.severity(), bevy::ecs::error::Severity::Panic);
            // Capture first (severity/context/message), then resume
            // unwinding so panic-severity errors crash the run as before.
            crate::sinks::capture_bevy_error(&err, &ctx);
            if is_panic {
                if let Some(payload) = err.take_payload() {
                    std::panic::resume_unwind(payload);
                }
            }
        });
        app.insert_resource(SwarmErrorHandlerInstalled);
    }

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
        force_single_threaded(app);
    }

    // Ambiguity gate (finalize): force schedule initialization NOW so
    // ambiguity errors surface as a structured rejection instead of a
    // panic inside Schedule::run during the update loop.
    if scenario.deny_ambiguities {
        let mut schedules = app
            .world_mut()
            .remove_resource::<bevy::ecs::schedule::Schedules>()
            .unwrap();
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
    }) || matches!(scenario.bot.bot_type, crate::enums::BotType::Planner);
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
    let compiled = crate::compiled::compile(app.world_mut(), scenario)?;
    app.insert_resource(compiled);

    let tps = scenario.tps as u64;
    if scenario.simulated_time {
        // Deterministic clock: Time advances by exactly 1/tps per update,
        // independent of host speed.
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_secs_f64(1.0 / tps as f64),
        ));
    }
    app.insert_resource(ScenarioResource(scenario.clone()));

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
    // I1: reset identity index before resets (ids are relative to run start).
    if let Some(mut idx) = app
        .world_mut()
        .get_resource_mut::<crate::identity::IdentityIndex>()
    {
        idx.reset();
    }
    let hooks = app
        .world_mut()
        .remove_resource::<ResetHooks>()
        .unwrap_or_default();
    for k in &kinds {
        if let Some(hook) = hooks.0.get(k) {
            hook(app.world_mut());
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

    let total_ticks = (scenario.duration_s * tps as f32) as u64;
    // Pre-run snapshot for code-aware (system) coverage — see
    // `system_coverage`. Must happen after finish/cleanup so schedules
    // are initialized and enumerable.
    let systems_before = snapshot_systems(app.world_mut());

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
                    "readiness_timeout",
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
        for frame_idx in 0..total_ticks {
            let t0 = std::time::Instant::now();
            app.update();
            let ms = t0.elapsed().as_secs_f64() * 1000.0;
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
                    if let Some(mut violations) = app.world_mut().get_resource_mut::<Violations>() {
                        for name in &scenario_frame_rules {
                            violations.report(name, "", note.clone(), frame_idx);
                        }
                    }
                }
            }
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
        s.pre_ready_frames = pre_ready_frames.get();
        s
    };

    // Planner end-of-run check: if the goal stack is not EMPTY, the top
    // unfinished primitive never achieved its condition — a goal that
    // was pursued but not reached (progress-stall bug signature).
    let mut snap = violations.snapshot();

    // Z7: drain the error/log/panic sink into the snapshot as
    // bevy_error / log_error / panic violations. Dedupe by
    // (rule, context) via Violations semantics.
    crate::sinks::clear_current_run();
    for captured in crate::sinks::drain_run(run_id) {
        match captured {
            crate::sinks::Captured::BevyError {
                severity,
                context,
                message,
                frame,
            } => {
                let rule = if severity >= crate::sinks::Severity::Error {
                    "bevy_error"
                } else {
                    "bevy_warning"
                };
                // Only surface Warning-severity and above.
                if severity >= crate::sinks::Severity::Warning {
                    let entry = crate::state::ViolationEntry {
                        rule: rule.to_string(),
                        target: context.clone(),
                        first_frame: frame,
                        last_frame: frame,
                        count: 1,
                        detail: message.clone(),
                        last_detail: message.clone(),
                        fingerprint: None,
                        fingerprint_scheme: 1,
                    };
                    snap.push(entry);
                }
            }
            crate::sinks::Captured::Log {
                level,
                target,
                message,
                frame,
            } => {
                if level == bevy::log::Level::ERROR {
                    // Allow-list: the second-App LogPlugin message is expected
                    // in multi-App processes and not a game defect.
                    if !message.contains("Could not set global logger")
                        && !message.contains("already set")
                    {
                        let entry = crate::state::ViolationEntry {
                            rule: "log_error".to_string(),
                            target: target.clone(),
                            first_frame: frame,
                            last_frame: frame,
                            count: 1,
                            detail: message.clone(),
                            last_detail: message.clone(),
                            fingerprint: None,
                            fingerprint_scheme: 1,
                        };
                        snap.push(entry);
                    }
                }
            }
            crate::sinks::Captured::Panic {
                message, location, ..
            } => {
                let entry = crate::state::ViolationEntry {
                    rule: "panic".to_string(),
                    target: "world".to_string(),
                    first_frame: 0,
                    last_frame: 0,
                    count: 1,
                    detail: match location {
                        Some(loc) => format!("{message} at {loc}"),
                        None => message.clone(),
                    },
                    last_detail: message.clone(),
                    fingerprint: None,
                    fingerprint_scheme: 1,
                };
                snap.push(entry);
            }
        }
    }
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
    // Intent-audit entries that cannot round-trip to a ReplayIntent:
    // non-zero means the run is not fully minimizable/replayable.
    let unreplayable_actions = crate::scenario::action_log_to_timed_actions(&action_log)
        .iter()
        .filter(|ta| crate::minimize::action_to_replay_intent(ta).is_none())
        .count();
    let mut warnings = Vec::new();
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

    // Auto-minimization on crash is NOT done inline — the harness cannot
    // rebuild the game's App from inside run_scenario. Instead, the
    // caller uses `minimize_crash`, which takes an app builder and the
    // report's action log, applies ddmin, and returns the minimal
    // reproducer plus a ready-to-save regression scenario.

    // Z14: exit deterministic entropy run.
    #[cfg(feature = "deterministic-entropy")]
    crate::entropy::exit_run();

    Ok(PlaytestReport {
        schema_version: 1,
        status,
        violations: snap,
        metrics: final_metrics,
        coverage: state.coverage,
        frame_count: state.frame,
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
        system_coverage: system_coverage(&systems_before, app.world_mut()),
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
    })
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
                    "synthetic_keyboard_config",
                    &kp.key,
                    "unknown key name (see KeyPressInput docs for supported names)".to_string(),
                    frame,
                );
            }
        }
    }
}
