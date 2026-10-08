//! Z6: gameplay inference from the game's crate path — end to end.

use bevy::prelude::*;
use bevy_swarm::contract::{Gameplay, TestApi, UserIntent};
use bevy_swarm::driver::{run_scenario, PlaytestPlugin};
use bevy_swarm::game_types::{GameTypes, InferredGameplay};
use bevy_swarm::scenario::Scenario;

#[derive(Component)]
struct PlayerHealth(u32);

#[derive(Component)]
struct EnemyDamage;

#[derive(Clone)]
struct TestGamePlugin;

impl Plugin for TestGamePlugin {
    fn build(&self, _app: &mut App) {}
}

fn build_app() -> App {
    let mut app = App::new();
    app.add_plugins((
        bevy::MinimalPlugins,
        bevy::transform::TransformPlugin,
        bevy_swarm::contract::TestConventionsPlugin,
        PlaytestPlugin,
    ));
    // What headless_app does for a real game (Z6): classify game-owned
    // types from the game plugin's crate path.
    app.insert_resource(GameTypes::from_plugin::<TestGamePlugin>());
    app.add_systems(bevy::app::Startup, |mut commands: Commands| {
        // Player and enemy: gameplay by inference.
        commands.spawn((Name::new("Player"), PlayerHealth(100)));
        commands.spawn((Name::new("Enemy"), EnemyDamage));
        // Camera: must NOT be inferred as gameplay.
        commands.spawn((Name::new("Cam"), bevy::camera::Camera::default()));
    });
    app.add_systems(bevy::app::Update, |mut q: Query<&mut PlayerHealth>| {
        for mut h in &mut q {
            h.0 += 1;
        }
    });
    app.insert_resource(TestApi::default());
    app.add_message::<UserIntent>();
    app
}

#[test]
fn infers_player_and_enemy_not_camera_or_ui() {
    let scenario: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"replay","inputs":[]},"duration_s":0.3,"invariants":[]}"#,
    )
    .unwrap();
    let mut app = build_app();
    let rep = run_scenario(&mut app, &scenario).unwrap();

    let world = app.world_mut();
    let players = world
        .query_filtered::<&PlayerHealth, With<InferredGameplay>>()
        .iter(world)
        .count();
    let enemies = world
        .query_filtered::<&EnemyDamage, With<InferredGameplay>>()
        .iter(world)
        .count();
    let cameras = world
        .query_filtered::<&bevy::camera::Camera, With<InferredGameplay>>()
        .iter(world)
        .count();
    assert_eq!(players, 1);
    assert_eq!(enemies, 1);
    assert_eq!(cameras, 0, "cameras must not be inferred as gameplay");
    // Liveness (I4) sees the inferred gameplay state: the ticking
    // PlayerHealth must keep the world alive (no frozen_world).
    assert!(
        !rep.violations.iter().any(|v| v.rule == "frozen_world"),
        "inferred gameplay must satisfy liveness: {:?}",
        rep.violations
    );
}

#[test]
fn explicit_marker_disables_inference_e2e() {
    let scenario: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"replay","inputs":[]},"duration_s":0.3,"invariants":[]}"#,
    )
    .unwrap();
    let mut app = build_app();
    // The game places ONE explicit marker: inference turns off.
    app.add_systems(bevy::app::Startup, |mut commands: Commands| {
        commands.spawn((Name::new("Explicit"), Gameplay, PlayerHealth(1)));
    });
    let _rep = run_scenario(&mut app, &scenario).unwrap();
    let world = app.world_mut();
    let inferred = world
        .query_filtered::<(), With<InferredGameplay>>()
        .iter(world)
        .count();
    assert_eq!(inferred, 0, "explicit Gameplay marker disables inference");
}

#[test]
fn game_types_prefix_helpers() {
    let gt = GameTypes::from_plugin::<TestGamePlugin>();
    // type_name of TestGamePlugin is "game_inference::TestGamePlugin".
    assert!(gt.is_game_type("game_inference::SomeSystem"));
    assert!(!gt.is_game_type("bevy_transform::Transform"));
    let gt2 = gt.with_crate("other_game_core");
    assert!(gt2.is_game_type("other_game_core::Health"));
}
