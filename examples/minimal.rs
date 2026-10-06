//! Minimal example: tiny game + scenario + run_scenario
//!
//! Run: `cargo run --example minimal`

use bevy::prelude::*;
use bevy_swarm::{
    contract::{IntentSurface, SurfaceVariant, TestConventionsPlugin},
    driver::{run_scenario, PlaytestPlugin, PlaytestReport},
    scenario::Scenario,
};

#[derive(Component)]
struct Player;

#[derive(Resource, Default)]
struct Score(i32);

fn spawn_player(mut commands: Commands) {
    commands.spawn((Player, Transform::default()));
}

fn handle_choice(
    mut reader: bevy::ecs::message::MessageReader<bevy_swarm::contract::UserIntent>,
    mut score: ResMut<Score>,
) {
    for intent in reader.read() {
        if let bevy_swarm::contract::UserIntent::Choice { .. } = intent {
            score.0 += 1;
        }
    }
}

fn main() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, TestConventionsPlugin, PlaytestPlugin));
    app.insert_resource(IntentSurface::new(vec![
        SurfaceVariant::Choice(3),
        SurfaceVariant::Wait,
    ]));
    app.insert_resource(Score(0));
    app.add_systems(Startup, spawn_player);
    app.add_systems(Update, handle_choice);

    let scenario: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"chaos","seed":42},"duration_s":0.5,"invariants":[],"setup":{}}"#,
    )
    .unwrap();

    let report: PlaytestReport = run_scenario(&mut app, &scenario).expect("run scenario");
    println!("Status: {:?}", report.status);
    println!("Intents emitted: {:?}", report.coverage.intents_emitted);
}
