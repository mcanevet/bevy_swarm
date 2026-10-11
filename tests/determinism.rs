//! A4: Determinism self-check oracle — bug/fix pair style tests.

use bevy::prelude::*;
use bevy_swarm::contract::TestConventionsPlugin;
use bevy_swarm::contract::{IntentSurface, SurfaceVariant};
use bevy_swarm::determinism::{check_determinism, DigestHooks};
use bevy_swarm::driver::PlaytestPlugin;
use bevy_swarm::harness::Gameplay;
use bevy_swarm::scenario::Scenario;
use std::time::{SystemTime, UNIX_EPOCH};

fn build_game() -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        bevy::transform::TransformPlugin,
        TestConventionsPlugin,
        PlaytestPlugin,
    ));
    app.insert_resource(IntentSurface::new(vec![
        SurfaceVariant::Choice(3),
        SurfaceVariant::Wait,
    ]));
    app.world_mut().spawn((
        Name::new("Ball"),
        Gameplay,
        Transform::from_xyz(0.0, 0.0, 0.0),
    ));
    app
}

fn scenario(seed: u64) -> Scenario {
    serde_json::from_str(&format!(
        r#"{{"bot":{{"type":"chaos","seed":{seed}}},"duration_s":0.3,"invariants":[]}}"#
    ))
    .unwrap()
}

#[test]
fn deterministic_game_passes() {
    let report = check_determinism(build_game, &scenario(5), 2).unwrap();
    assert!(
        report.is_deterministic(),
        "diverged at {:?}",
        report.first_divergence
    );
}

#[test]
fn wall_clock_game_detected() {
    let mut app = build_game();
    app.add_systems(Update, |mut q: Query<&mut Transform, With<Gameplay>>| {
        if let Ok(mut t) = q.single_mut() {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .subsec_nanos();
            // Accumulate full nanos as f32 — two back-to-back runs are
            // essentially guaranteed to diverge within a few frames
            // (subsec_nanos % 7 could collide between adjacent runs).
            t.translation.x += (nanos % 1_000_000) as f32 * 0.001;
        }
    });
    let builder = move || {
        let mut app = build_game();
        app.add_systems(Update, |mut q: Query<&mut Transform, With<Gameplay>>| {
            if let Ok(mut t) = q.single_mut() {
                let nanos = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .subsec_nanos();
                // Accumulate full nanos as f32 — two back-to-back runs are
                // essentially guaranteed to diverge within a few frames
                // (subsec_nanos % 7 could collide between adjacent runs).
                t.translation.x += (nanos % 1_000_000) as f32 * 0.001;
            }
        });
        app
    };
    let _ = app; // (built once above only for symmetry; builder owns the wiring)
    let report = check_determinism(builder, &scenario(3), 2).unwrap();
    assert!(!report.is_deterministic());
    let div = report.first_divergence.expect("divergence");
    assert!(
        div.differing
            .iter()
            .any(|(k, _, _)| k.starts_with("Gameplay[") && k.ends_with("translation.x")),
        "expected a Gameplay translation.x key, got {:?}",
        div.differing
    );
}

#[test]
fn digest_hooks_participate() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let builder = || {
        let mut app = build_game();
        app.insert_resource(DigestHooks(vec![(
            "run_counter".to_string(),
            Box::new(|_w: &bevy::ecs::world::World| {
                COUNTER.fetch_add(1, Ordering::SeqCst).to_string()
            }),
        )]));
        app
    };
    let report = check_determinism(builder, &scenario(3), 2).unwrap();
    assert!(!report.is_deterministic(), "hook must trigger divergence");
    let div = report.first_divergence.expect("divergence");
    assert!(
        div.differing
            .iter()
            .any(|(k, _, _)| k.contains("hook.run_counter")),
        "expected hook key in differing, got {:?}",
        div.differing
    );
}

#[test]
fn no_overhead_without_state_trace() {
    // The digest system early-returns when StateTrace is absent — a
    // normal run's report has no state_trace and identical results.
    let mut app = build_game();
    let rep = bevy_swarm::driver::run_scenario(&mut app, &scenario(9)).unwrap();
    assert!(rep.state_trace.is_none());
}

#[test]
fn fx6_a4_late_divergence_reported_with_fields() {
    // Divergence INTRODUCED at frame 30 must report frame 30 (not a
    // frame-0 comparison producing no differing fields).
    use bevy_swarm::harness::PlaytestState;
    let builder = || {
        let mut app = build_game();
        app.add_systems(
            Update,
            |mut q: Query<&mut Transform, With<Gameplay>>, frame: Res<PlaytestState>| {
                if let Ok(mut t) = q.single_mut() {
                    if frame.frame == 30 {
                        // True divergence at frame 30.
                        t.translation.x += std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap()
                            .subsec_nanos() as f32;
                    }
                }
            },
        );
        app
    };
    let long: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"chaos","seed":3},"duration_s":1.0,"invariants":[]}"#,
    )
    .unwrap();
    let report = check_determinism(builder, &long, 2).unwrap();
    assert!(!report.is_deterministic(), "must diverge after frame 30");
    let div = report.first_divergence.expect("divergence");
    assert!(
        div.differing
            .iter()
            .any(|(k, _, _)| k.contains("translation.x")),
        "differing must name the diverged field at the divergence frame, got {:?}",
        div.differing
    );
}

/// FX6: a panicking reset hook must NOT escape run_scenario — reported
/// as reset_hook_panicked, run continues.
#[test]
fn fx6_reset_hook_panick_is_caught_and_reported() {
    use bevy_swarm::contract::ResetHooks;
    let scenario: bevy_swarm::scenario::Scenario = serde_json::from_str(
        r#"{"bot":{"type":"replay","inputs":[]},"duration_s":0.1,"tps":60,"setup":{"resets":[{"kind":"boom"}]}}"#,
    )
    .unwrap();
    let mut app = bevy::prelude::App::new();
    app.add_plugins((
        bevy::prelude::MinimalPlugins,
        bevy::transform::TransformPlugin,
        bevy_swarm::driver::PlaytestPlugin,
    ));
    let mut hooks = ResetHooks::default();
    hooks.register("boom", |_world: &mut bevy::prelude::World| {
        panic!("reset hook exploded");
    });
    app.insert_resource(hooks);
    let report = bevy_swarm::driver::run_scenario(&mut app, &scenario).unwrap();
    assert!(
        report
            .violations
            .iter()
            .any(|v| v.rule == "reset_hook_panicked"),
        "must report reset_hook_panicked, got: {:?}",
        report.violations
    );
}

/// FX6: a panicking cheat hook must be caught and reported as
/// cheat_hook_panicked, and LATER scheduled cheats must still run
/// (the CheatHooks resource survives the panic).
#[test]
fn fx6_cheat_hook_panick_is_caught_and_reported() {
    use bevy_swarm::contract::CheatHooks;
    let scenario: bevy_swarm::scenario::Scenario = serde_json::from_str(
        r#"{"bot":{"type":"replay","inputs":[]},"duration_s":0.2,"tps":60,"setup":{"cheats":[{"frame":1,"kind":"boom"},{"frame":3,"kind":"later"}]}}"#,
    )
    .unwrap();
    let mut app = bevy::prelude::App::new();
    app.add_plugins((
        bevy::prelude::MinimalPlugins,
        bevy::transform::TransformPlugin,
        bevy_swarm::driver::PlaytestPlugin,
    ));
    let mut hooks = CheatHooks::default();
    hooks.register("boom", |_world: &mut bevy::prelude::World| {
        panic!("cheat hook exploded");
    });
    hooks.register("later", |world: &mut bevy::prelude::World| {
        world.insert_resource(LaterCheatRan(true));
    });
    app.insert_resource(hooks);
    app.insert_resource(LaterCheatRan(false));
    let report = bevy_swarm::driver::run_scenario(&mut app, &scenario).unwrap();
    assert!(
        report
            .violations
            .iter()
            .any(|v| v.rule == "cheat_hook_panicked"),
        "must report cheat_hook_panicked, got: {:?}",
        report.violations
    );
    assert!(
        app.world()
            .get_resource::<LaterCheatRan>()
            .is_some_and(|r| r.0),
        "the later cheat must still run after the panicking one"
    );
}

#[derive(bevy::prelude::Resource, Default)]
struct LaterCheatRan(bool);

/// FX6 unwind safety: a panicking TestApi resolver during the planner's
/// taken-Violations region must not lose the Violations resource — the
/// run completes (crash status) and the report is still produced.
#[test]
fn fx6_planner_resolver_panic_keeps_violations_resource() {
    use bevy_swarm::contract::{RegisterTestApi, TestApiResolve};

    #[derive(bevy::prelude::Resource)]
    struct PanicApi;

    impl TestApiResolve for PanicApi {
        fn resolve(&self, _path: &str) -> Option<bevy_swarm::contract::TestFieldValue> {
            panic!("resolver exploded");
        }
    }

    static RESOLVE_CALLED: std::sync::atomic::AtomicBool =
        std::sync::atomic::AtomicBool::new(false);

    let scenario: bevy_swarm::scenario::Scenario = serde_json::from_str(
        r#"{"bot":{"type":"planner","goals":{"kind":"primitive","path":"TestApi.score","check":"above","value":10,"emit":[{"intent":"wait"}]}},"duration_s":0.2,"tps":60,"invariants":[]}"#,
    )
    .unwrap();
    let mut app = bevy::prelude::App::new();
    app.add_plugins((
        bevy::prelude::MinimalPlugins,
        bevy::transform::TransformPlugin,
        bevy_swarm::driver::PlaytestPlugin,
    ));
    app.insert_resource(PanicApi);
    app.register_test_api::<PanicApi>();
    RESOLVE_CALLED.store(true, std::sync::atomic::Ordering::SeqCst);
    // The run must not hang or drop the Violations resource mid-flight.
    let report = bevy_swarm::driver::run_scenario(&mut app, &scenario);
    let rep = report.expect("run must complete (panic caught), not error");
    eprintln!("STATUS: {:?} VIOLATIONS: {:?}", rep.status, rep.violations);
    // After the run, the report's violation extraction already proves
    // Violations was available; a lost resource would have shown up as
    // an unwrap panic inside run_scenario instead of a report.
    assert!(
        app.world()
            .get_resource::<bevy_swarm::harness::Violations>()
            .is_some(),
        "Violations resource must survive a mid-run resolver panic"
    );
}
