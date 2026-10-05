//! Benchmark: frozen-world soft-lock detection (animation stops updating)
//!
//! Clean version: a gameplay entity rotates continuously.
//! Buggy version: transform updates are skipped after frame 30 (simulated
//! stuck game loop / soft-lock).
//! Expected: frozen-world oracle detects the stall in a real-time game.

use bevy::prelude::*;
use bevy_swarm::contract::*;
use bevy_swarm::harness::*;

#[derive(Component, Reflect)]
#[reflect(Component)]
struct Spinner;

fn spin_system(time: Res<Time>, mut q: Query<&mut Transform, With<Spinner>>) {
    for mut t in &mut q {
        t.rotate_z(0.1 + time.delta_secs() * 60.0);
    }
}

/// Buggy: rotation stops after frame 30 — simulated soft-lock.
fn spin_system_buggy(
    mut frame: Local<u64>,
    time: Res<Time>,
    mut q: Query<&mut Transform, With<Spinner>>,
) {
    *frame += 1;
    if *frame > 30 {
        return; // frozen: simulates a stuck loop or dead system
    }
    for mut t in &mut q {
        t.rotate_z(0.1 + time.delta_secs() * 60.0);
    }
}

fn build_app(buggy: bool) -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(TestConventionsPlugin);
    app.add_plugins(PlaytestPlugin);
    app.insert_resource(IntentSurface::new(vec![SurfaceVariant::Wait]));
    // RealTime marker: arms the frozen-world oracle (TurnBased exempts).
    app.add_systems(Startup, |mut commands: Commands| {
        commands.spawn((Spinner, Transform::default(), Gameplay));
    });
    if buggy {
        app.add_systems(Update, spin_system_buggy);
    } else {
        app.add_systems(Update, spin_system);
    }
    app
}

#[test]
fn frozen_world_clean_passes() {
    let scenario: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"chaos","seed":42},"duration_s":1.0,"invariants":[],"setup":{}}"#,
    )
    .unwrap();
    let mut app = build_app(false);
    let report = run_scenario(&mut app, &scenario).unwrap();
    assert!(
        !report.violations.iter().any(|v| v.rule == "frozen_world"),
        "clean animation must not trip frozen-world: {:?}",
        report.violations
    );
}

#[test]
fn frozen_world_buggy_detected() {
    let scenario: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"chaos","seed":42},"duration_s":3.0,"invariants":[],"setup":{}}"#,
    )
    .unwrap();
    let mut app = build_app(true);
    let report = run_scenario(&mut app, &scenario).unwrap();
    assert!(
        report.violations.iter().any(|v| v.rule == "frozen_world"),
        "buggy (stalled) animation must trip frozen-world: {:?}",
        report.violations
    );
    assert_eq!(report.status, "fail");
}
