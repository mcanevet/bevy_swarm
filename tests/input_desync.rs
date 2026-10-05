//! Benchmark: input desync detection (intent emitted but world ignores it)
//!
//! Clean version: Move intents from the bot translate the player entity.
//! Buggy version: the movement system reads a stale/renamed input source,
//! so the player never moves despite the bot issuing Move intents.
//! Expected: the harness's intent audit / no-movement check flags the desync.

use bevy::prelude::*;
use bevy_swarm::contract::*;
use bevy_swarm::harness::*;
use bevy_swarm::enums::PlaytestStatus;

#[derive(Component, Reflect)]
#[reflect(Component)]
struct Player;

fn setup(mut commands: Commands) {
    commands.spawn((Player, Transform::default(), Gameplay));
}

/// Clean: consume Move intents and move the player.
fn move_system_clean(
    mut events: MessageReader<UserIntent>,
    mut q: Query<&mut Transform, With<Player>>,
) {
    for intent in events.read() {
        let UserIntent::Move { dir } = intent else {
            continue;
        };
        let d = Vec3::new(dir.x, dir.y, 0.0);
        for mut t in &mut q {
            t.translation += d;
        }
    }
}

/// Buggy: input wiring broken — the system never receives/moves anything.
fn move_system_buggy(_events: MessageReader<UserIntent>, _q: Query<&mut Transform, With<Player>>) {
    // dead wiring: intent events are never consumed
}

fn build_app(buggy: bool) -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(TestConventionsPlugin);
    app.add_plugins(PlaytestPlugin);
    app.insert_resource(IntentSurface::new(vec![
        SurfaceVariant::Move,
        SurfaceVariant::Wait,
    ]));
    app.add_systems(Startup, setup);
    if buggy {
        app.add_systems(Update, move_system_buggy);
    } else {
        app.add_systems(Update, move_system_clean);
    }
    app
}

#[test]
fn input_desync_clean_passes() {
    let scenario: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"chaos","seed":7},"duration_s":1.0,"invariants":[],"setup":{}}"#,
    )
    .unwrap();
    let mut app = build_app(false);
    let report = run_scenario(&mut app, &scenario).unwrap();
    assert_eq!(report.status, PlaytestStatus::Pass, "clean: {:?}", report.violations);
}

#[test]
fn input_desync_buggy_detected() {
    // The frozen-world or intent audit must catch that Move intents
    // produce no world change (player frozen despite live input).
    let scenario: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"chaos","seed":7},"duration_s":3.0,"invariants":[],"setup":{}}"#,
    )
    .unwrap();
    let mut app = build_app(true);
    let report = run_scenario(&mut app, &scenario).unwrap();
    assert!(
        report.violations.iter().any(|v| v.rule == "frozen_world"),
        "broken input wiring must be detected: {:?}",
        report.violations
    );
}
