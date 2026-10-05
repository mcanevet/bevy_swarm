//! Benchmark: memory leak detection (entity spawn without despawn)
//!
//! Clean version: spawns 10 entities over 10 frames, then stops.
//! Buggy version: spawns entities every frame forever.
//! Expected: harness detects entity-count explosion via query-target invariant.

use bevy::prelude::*;
use bevy_playtest::contract::*;
use bevy_playtest::harness::*;

#[derive(Component, Reflect)]
#[reflect(Component)]
struct LeakyEntity;

/// Clean version: spawns exactly 10 entities, then stops.
fn spawn_clean_system(mut commands: Commands, mut frame: Local<u64>) {
    if *frame < 10 {
        commands.spawn((LeakyEntity, Transform::default()));
    }
    *frame += 1;
}

/// Buggy version: spawns entities forever (memory leak).
fn spawn_buggy_system(mut commands: Commands) {
    commands.spawn((LeakyEntity, Transform::default()));
}

fn build_clean_app() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(TestConventionsPlugin);
    app.add_plugins(PlaytestPlugin);
    app.insert_resource(IntentSurface::new(vec![SurfaceVariant::Wait]));
    app.add_systems(Update, spawn_clean_system);
    app
}

fn build_buggy_app() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(TestConventionsPlugin);
    app.add_plugins(PlaytestPlugin);
    app.insert_resource(IntentSurface::new(vec![SurfaceVariant::Wait]));
    app.add_systems(Update, spawn_buggy_system);
    app
}

#[test]
fn memory_leak_clean_passes() {
    // Clean version should pass — entity count stabilizes at 10.
    let scenario: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"chaos","seed":42},"duration_s":0.5,"invariants":[
            {"name":"entity_count_bound","rule":"custom","query":{"with":["LeakyEntity"]},"check":"below","value":15}
        ],"setup":{}}"#,
    ).unwrap();

    let mut app = build_clean_app();
    let report = run_scenario(&mut app, &scenario).unwrap();
    assert_eq!(
        report.status, "pass",
        "Clean version should pass: {:?}",
        report.violations
    );
}

#[test]
fn memory_leak_buggy_fails() {
    // Buggy version should fail — entity count explodes beyond max.
    let scenario: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"chaos","seed":42},"duration_s":0.5,"invariants":[
            {"name":"entity_count_bound","rule":"custom","query":{"with":["LeakyEntity"]},"check":"below","value":15}
        ],"setup":{}}"#,
    ).unwrap();

    let mut app = build_buggy_app();
    let report = run_scenario(&mut app, &scenario).unwrap();
    assert_eq!(report.status, "fail", "Buggy version should fail");
    assert!(
        report
            .violations
            .iter()
            .any(|v| v.rule == "entity_count_bound"),
        "Should detect entity-count violation: {:?}",
        report.violations
    );
}
