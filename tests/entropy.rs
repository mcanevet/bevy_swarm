#![allow(unknown_lints)]
#![allow(unexpected_cfgs)]
//! Z14: deterministic entropy via getrandom custom backend.
//!
//! These tests require the build cfg `getrandom_backend="custom"` AND
//! the `deterministic-entropy` feature. Without both, the hook is not
//! defined and getrandom falls back to the OS source; tests skip.

#![cfg(all(feature = "deterministic-entropy", getrandom_backend = "custom"))]

use bevy::prelude::*;
use bevy_swarm::entropy::{enter_run, exit_run};

#[test]
fn getrandom_fill_is_deterministic() {
    // Simulate two runs with the same seed drawing entropy.
    enter_run(42);
    let mut buf1 = [0u8; 32];
    getrandom04::fill(&mut buf1).unwrap();
    let mut buf2 = [0u8; 32];
    getrandom04::fill(&mut buf2).unwrap();
    exit_run();

    // Different position in the stream: continuation, not repetition.
    assert_ne!(buf1, buf2);

    // A fresh run with the same seed reproduces the FIRST draw.
    enter_run(42);
    let mut buf3 = [0u8; 32];
    getrandom04::fill(&mut buf3).unwrap();
    exit_run();
    assert_eq!(buf1, buf3, "same seed must reproduce the same entropy");

    // Different seed diverges.
    enter_run(43);
    let mut buf4 = [0u8; 32];
    getrandom04::fill(&mut buf4).unwrap();
    exit_run();
    assert_ne!(buf1, buf4);
}
#[test]
fn uuid_v4_deterministic() {
    enter_run(7);
    let u1 = uuid::Uuid::new_v4();
    exit_run();

    enter_run(7);
    let u2 = uuid::Uuid::new_v4();
    exit_run();
    assert_eq!(u1, u2, "same seed must reproduce the same uuid");

    enter_run(8);
    let u3 = uuid::Uuid::new_v4();
    exit_run();
    assert_ne!(u1, u3);
}
#[test]
fn run_scenario_provides_entropy_scope() {
    use bevy_swarm::contract::TestApi;
    use bevy_swarm::driver::{run_scenario, PlaytestPlugin};
    use bevy_swarm::scenario::Scenario;

    #[derive(Resource, Default)]
    struct LatestUuid(uuid::Uuid);

    fn spawn_uuid_system(res: Res<LatestUuid>, mut api: ResMut<TestApi>) {
        // Draw randomness INSIDE the run (like a game spawning entities).
        api.custom_numeric
            .insert("Uuid.HighBits".into(), res.0.as_u128() as f64);
    }
    fn make_uuid_system(mut res: ResMut<LatestUuid>) {
        res.0 = uuid::Uuid::new_v4();
    }
    let scenario: Scenario =
        serde_json::from_str(r#"{"bot":{"type":"chaos","seed":99},"duration_s":0.05}"#).unwrap();

    let run = || {
        let mut app = App::new();
        app.add_plugins(PlaytestPlugin);
        app.init_resource::<LatestUuid>();
        app.insert_resource(TestApi {
            score: 0,
            active_players: 0,
            custom_numeric: Default::default(),
            custom_text: Default::default(),
        });
        app.add_systems(Update, make_uuid_system);
        app.add_systems(Last, spawn_uuid_system);
        app.insert_resource(bevy_swarm::determinism::StateTrace::default());
        let rep = run_scenario(&mut app, &scenario).unwrap();
        rep.state_trace
            .iter()
            .flatten()
            .map(|d| d.hash)
            .collect::<Vec<_>>()
    };

    let d1 = run();
    let d2 = run();
    assert_eq!(
        d1, d2,
        "same scenario seed must reproduce identical digests"
    );
    assert!(!d1.is_empty());
}
#[test]
fn error_layout_assertions() {
    // Verify getrandom 0.3 and 0.4 Error types have identical layout.
    // (We only depend on 0.4, but assert against 0.3's definition
    // as documented in the bead.)
    use std::mem::{align_of, size_of};

    // getrandom 0.4 Error is a tuple struct with NonZeroU16 (or similar).
    // Assert our assumptions hold.
    assert_eq!(size_of::<getrandom04::Error>(), 4); // NonZeroU32
    assert_eq!(align_of::<getrandom04::Error>(), 4);
    assert_eq!(size_of::<Result<(), getrandom04::Error>>(), 4);
}
/// FX3 GREEN: getrandom outside enter_run must NOT abort — it draws
/// from the process-global fallback stream and increments the counter.
/// (RED on main: the old extern "C" hook panic!ed, aborting the whole
/// test process — see PR description for the SIGABRT proof.)
#[test]
fn draw_outside_run_does_not_abort() {
    bevy_swarm::entropy::reset_fallback_counter();
    let before = bevy_swarm::entropy::fallback_draw_count();
    let mut buf = [0u8; 8];
    // Must not panic or abort.
    getrandom04::fill(&mut buf).unwrap();
    let after = bevy_swarm::entropy::fallback_draw_count();
    assert_eq!(after, before + 1, "unattributed draw must be counted");
}

/// FX3 GREEN: same seed → identical values; different seed → different.
/// (Acceptance bullet: rand-style draws inside a game system.)
#[test]
fn same_seed_reproduces_across_two_apps() {
    use bevy::prelude::*;

    #[derive(Resource, Deref, DerefMut)]
    struct Drawn(Vec<u64>);

    let run = |seed: u64| {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(Drawn(Vec::new()));
        // Enter BEFORE any system draws.
        bevy_swarm::entropy::enter_run(seed);
        app.add_systems(Startup, |mut drawn: ResMut<Drawn>| {
            // Draw 4 u64s via getrandom (the underlying source for
            // rand::rng() and uuid v4).
            for _ in 0..4 {
                let mut buf = [0u8; 8];
                getrandom04::fill(&mut buf).unwrap();
                drawn.push(u64::from_ne_bytes(buf));
            }
        });
        // dde: the dev-dep graph (fixture-runner -> multithread_panic)
        // unifies bevy's `multi_threaded` feature into this build, so
        // the default schedule executor is multi-threaded and systems
        // run on task-pool threads — without the thread-local entropy
        // stream the draws fall back to the global stream and
        // same-seed runs diverge. Real harness runs force the
        // single-threaded executor (driver: force_single_threaded,
        // AFTER all systems are registered so lazily-created schedules
        // are covered); mirror that here.
        let mut schedules = app
            .world_mut()
            .remove_resource::<bevy::ecs::schedule::Schedules>()
            .expect("schedules");
        for (_label, sched) in schedules.iter_mut() {
            sched.set_executor(bevy::ecs::schedule::SingleThreadedExecutor::new());
        }
        app.world_mut().insert_resource(schedules);
        app.update();
        bevy_swarm::entropy::exit_run();
        app.world_mut().resource::<Drawn>().0.clone()
    };

    let a1 = run(42);
    let a2 = run(42);
    let b = run(43);
    assert_eq!(a1, a2, "same seed must reproduce identical draws");
    assert_ne!(a1, b, "different seeds must diverge");
}

// ---------------------------------------------------------------------------
// FX3 re-review: entry-order, build-time draw test.
// ---------------------------------------------------------------------------

/// A plugin that draws during build to prove the hook is active before
/// plugin construction.
struct EntropyBuildPlugin;
impl Plugin for EntropyBuildPlugin {
    fn build(&self, app: &mut App) {
        // Draw during build — must NOT panic and must be counted as
        // unattributed if enter_run hasn't happened yet.
        let mut buf = [0u8; 8];
        let _ = getrandom04::fill(&mut buf);
        app.insert_resource(BuildEntropyResource(buf));
    }
}

/// Marker resource storing the entropy drawn during build.
#[derive(Resource)]
struct BuildEntropyResource([u8; 8]);

#[test]
fn fx3_factory_draw_does_not_abort() {
    // A plugin that draws during build must NOT cause a panic even
    // when enter_run hasn't been called yet (fallback stream).
    let mut app = bevy::app::App::new();
    app.add_plugins(bevy::MinimalPlugins);
    // No enter_run yet — this would panic if the hook was wrong.
    app.add_plugins(EntropyBuildPlugin);
    // If we got here, the fallback didn't abort.
    let buf = app
        .world_mut()
        .get_resource::<BuildEntropyResource>()
        .expect("plugin ran build");
    // The fallback drew something.
    assert_ne!(buf.0, [0u8; 8], "fallback should have produced entropy");
}

#[test]
fn fx3_enter_before_factory() {
    // InProcess::run must call enter_run BEFORE the factory (so
    // plugin-build draws are seeded). Verified via a plugin that
    // copies its build-time draw into a shared slot: identical across
    // same-seed runs, different across seeds.
    use bevy_swarm::branch::{InProcess, ScenarioRunner};
    use bevy_swarm::scenario::Scenario;
    use std::sync::{Arc, Mutex};

    struct ProbePlugin(Arc<Mutex<Vec<u8>>>);
    impl Plugin for ProbePlugin {
        fn build(&self, _app: &mut App) {
            let mut buf = [0u8; 16];
            let _ = getrandom04::fill(&mut buf);
            *self.0.lock().unwrap() = buf.to_vec();
        }
    }

    let mut results = Vec::new();
    for seed in [42, 42, 43] {
        let probe = Arc::new(Mutex::new(Vec::new()));
        let p2 = Arc::clone(&probe);
        let runner = InProcess {
            factory: move || {
                let mut app = bevy::app::App::new();
                app.add_plugins(bevy::MinimalPlugins);
                app.add_plugins(ProbePlugin(Arc::clone(&p2)));
                app
            },
        };
        let scenario: Scenario = serde_json::from_str(&format!(
            r#"{{"bot":{{"type":"chaos","seed":{}}},"duration_s":0.1}}"#,
            seed
        ))
        .unwrap();
        runner.run(&scenario).unwrap();
        results.push(probe.lock().unwrap().clone());
    }
    assert_eq!(
        results[0], results[1],
        "same seed must reproduce build-time entropy"
    );
    assert_ne!(results[0], results[2], "different seed must diverge");
}

#[test]
fn fx3_rand_rng_acceptance_matrix() {
    // A game system drawing rand::rng().random::<u64>() gives identical
    // values across 2 same-seed runs and different across seeds, when
    // each run executes on a FRESH thread (ThreadRng is per-thread and
    // cached — the matrix runner's discipline).
    use bevy::prelude::*;
    use bevy_swarm::driver::{run_scenario, PlaytestPlugin};
    use bevy_swarm::scenario::Scenario;
    use rand::RngExt;

    #[derive(Resource, Default)]
    struct RandValue(Option<u64>);

    let run_on_thread = |seed: u64| {
        std::thread::spawn(move || {
            let mut app = bevy::app::App::new();
            app.add_plugins(PlaytestPlugin);
            app.init_resource::<RandValue>();
            app.add_systems(Update, |mut v: ResMut<RandValue>| {
                if v.0.is_none() {
                    v.0 = Some(rand::rng().random::<u64>());
                }
            });
            let scenario: Scenario = serde_json::from_str(&format!(
                r#"{{"bot":{{"type":"chaos","seed":{}}},"duration_s":0.05}}"#,
                seed
            ))
            .unwrap();
            run_scenario(&mut app, &scenario).unwrap();
            v0(app)
        })
        .join()
        .expect("thread panicked")
    };
    fn v0(app: bevy::app::App) -> u64 {
        app.world().resource::<RandValue>().0.expect("drawn")
    }

    let a = run_on_thread(5);
    let b = run_on_thread(5);
    let c = run_on_thread(6);
    assert_eq!(a, b, "same seed must reproduce rand::rng() draws");
    assert_ne!(a, c, "different seed must diverge");
}
// RF10: same-seed run_scenario calls on the SAME thread produce
// identical rand::rng() draws. InProcess::run spawns a fresh thread
// per run when deterministic-entropy is enabled.
#[test]
#[cfg(feature = "deterministic-entropy")]
fn rf10_same_thread_run_scenario_reproducible() {
    use bevy::prelude::*;
    use bevy_swarm::branch::{InProcess, ScenarioRunner};
    use bevy_swarm::driver::PlaytestPlugin;
    use bevy_swarm::scenario::Scenario;
    use rand::RngExt;
    use std::sync::{Arc, Mutex};

    let captured = Arc::new(Mutex::new(Vec::<u64>::new()));

    let factory = |captured: Arc<Mutex<Vec<u64>>>| {
        move || {
            let mut app = bevy::app::App::new();
            app.add_plugins(PlaytestPlugin);
            let cap = captured.clone();
            app.add_systems(Update, move |mut done: Local<bool>| {
                if !*done {
                    *done = true;
                    let val = rand::rng().random::<u64>();
                    cap.lock().unwrap().push(val);
                }
            });
            app
        }
    };

    let scenario: Scenario =
        serde_json::from_str(r#"{"bot":{"type":"chaos","seed":5},"duration_s":0.05}"#).unwrap();

    let runner = InProcess {
        factory: factory(captured.clone()),
    };
    let _ = runner.run(&scenario).unwrap();
    let _ = runner.run(&scenario).unwrap();

    let vals = captured.lock().unwrap();
    assert_eq!(vals.len(), 2, "two runs");
    assert_eq!(
        vals[0], vals[1],
        "same seed must reproduce rand::rng() draws across same-thread runs"
    );
}

// RF10: check_determinism reruns on fresh threads — no false
// nondeterministic violation for rand::rng() games.
#[test]
#[cfg(feature = "deterministic-entropy")]
fn rf10_check_determinism_rand_rng_no_false_violation() {
    use bevy::prelude::*;
    use bevy_swarm::branch::{InProcess, ScenarioRunner};
    use bevy_swarm::driver::PlaytestPlugin;
    use bevy_swarm::scenario::Scenario;
    use rand::RngExt;

    let factory = || {
        let mut app = bevy::app::App::new();
        app.add_plugins(PlaytestPlugin);
        app.add_systems(Update, |mut done: Local<bool>| {
            if !*done {
                *done = true;
                let _val = rand::rng().random::<u64>();
            }
        });
        app
    };

    let scenario: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"chaos","seed":5},"duration_s":0.05,"check_determinism":3}"#,
    )
    .unwrap();

    let runner = InProcess { factory };
    let report = runner.run(&scenario).unwrap();

    // Should NOT have a nondeterministic violation
    let violations = report
        .violations
        .iter()
        .filter(|v| v.rule == "nondeterministic")
        .collect::<Vec<_>>();
    assert!(violations.is_empty(), "rand::rng() game with check_determinism=3 should not report false nondeterministic violation: {:?}", violations);
}

// RF10: build-time and runtime entropy draws are DIFFERENT.
// Previously, run_scenario called enter_run again after InProcess,
// resetting the stream to its start and duplicating the first N bytes.
#[test]
#[cfg(feature = "deterministic-entropy")]
fn rf10_build_time_vs_runtime_entropy_different() {
    use bevy::prelude::*;
    use bevy_swarm::branch::{InProcess, ScenarioRunner};
    use bevy_swarm::driver::PlaytestPlugin;
    use bevy_swarm::scenario::Scenario;
    use std::sync::{Arc, Mutex};

    let captured = Arc::new(Mutex::new((Vec::<u8>::new(), Vec::<u8>::new())));

    let factory = |captured: Arc<Mutex<(Vec<u8>, Vec<u8>)>>| {
        move || {
            let mut app = bevy::app::App::new();
            app.add_plugins(PlaytestPlugin);
            let cap = captured.clone();
            // Plugin build: draw 16 bytes
            let mut buf = [0u8; 16];
            getrandom04::fill(&mut buf).unwrap();
            cap.lock().unwrap().0 = buf.to_vec();

            let cap2 = captured.clone();
            app.add_systems(Startup, move || {
                // Startup system: draw 16 bytes
                let mut buf = [0u8; 16];
                getrandom04::fill(&mut buf).unwrap();
                cap2.lock().unwrap().1 = buf.to_vec();
            });
            app
        }
    };

    let scenario: Scenario =
        serde_json::from_str(r#"{"bot":{"type":"chaos","seed":5},"duration_s":0.05}"#).unwrap();

    let runner = InProcess {
        factory: factory(captured.clone()),
    };
    let _ = runner.run(&scenario).unwrap();

    let guard = captured.lock().unwrap();
    let (build_bytes, runtime_bytes) = (&guard.0, &guard.1);
    assert_ne!(
        build_bytes.as_slice(),
        runtime_bytes.as_slice(),
        "build-time and runtime entropy draws should differ"
    );
}
