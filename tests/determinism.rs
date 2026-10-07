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
