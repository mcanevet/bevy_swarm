//! Fixture runner: adapter plugins + headless app builder + report writer
//!
//! Adapter tier (temporary until zero-contract lands in v0.3):
//! adapters BRIDGE bevy_swarm's contract to a fixture's own types — they
//! tag the fixture's entities as Gameplay and translate UserIntent into
//! the fixture's native messages. They NEVER reimplement game behavior:
//! the fixture's own systems remain the code under test.
//!
//! RV v0.3 must delete these adapters and convert every case to
//! scaffold "zero".

use bevy::prelude::*;
use bevy_swarm::contract::*;
use bevy_swarm::conventions;
use std::fs;

/// Minimal headless app builder for fixtures (v0.2; Z2 replaces with real
/// headless_platform). MinimalPlugins provides Time/task/chedule plumbing.
pub fn headless_app() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app
}

// ============================================================================
// Adapter: Spinner — tag the fixture's Spinner entities as Gameplay
// ============================================================================

pub struct SpinnerAdapterPlugin;

impl Plugin for SpinnerAdapterPlugin {
    fn build(&self, app: &mut App) {
        // FX1: tag via Added<> in Update — the game's Startup command
        // buffers haven't applied when OUR Startup systems ran, so
        // Startup-time queries missed every entity. Added<> catches
        // entities the frame they appear, including mid-run spawns.
        app.add_systems(
            Update,
            |mut commands: Commands, q: Query<Entity, Added<fixture_spinner::Spinner>>| {
                for e in &q {
                    commands.entity(e).insert((Gameplay, RealTime));
                }
            },
        );
    }
}

// ============================================================================
// Adapter: Walker — map UserIntent::Move to the fixture's MoveEvent
// ============================================================================

fn walker_intent_bridge(
    mut intents: MessageReader<UserIntent>,
    mut moves: MessageWriter<fixture_walker::MoveEvent>,
) {
    for intent in intents.read() {
        if let UserIntent::Move { dir } = intent {
            moves.write(fixture_walker::MoveEvent { dir: *dir });
        }
    }
}

pub struct WalkerAdapterPlugin;

impl Plugin for WalkerAdapterPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<fixture_walker::MoveEvent>();
        app.add_systems(Update, walker_intent_bridge);
        app.add_systems(
            Update,
            |mut commands: Commands, q: Query<Entity, Added<fixture_walker::Player>>| {
                for e in &q {
                    commands.entity(e).insert(Gameplay);
                }
            },
        );
    }
}

// ============================================================================
// Adapter: Spawner — tag the fixture's entities as Gameplay
// ============================================================================

pub struct SpawnerAdapterPlugin;

impl Plugin for SpawnerAdapterPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            |mut commands: Commands, q: Query<Entity, Added<fixture_spawner::LeakyEntity>>| {
                for e in &q {
                    // TurnBased: the spawner is bursty by design (spawn
                    // 10, then idle) — real-time liveness semantics
                    // would flag its natural idle as a soft-lock.
                    commands.entity(e).insert((Gameplay, TurnBased));
                }
            },
        );
    }
}

// ============================================================================
// Adapter: TurnBased — bridge UserIntent::EndTurn to the fixture's CardPlayed
// ============================================================================

fn turn_based_intent_bridge(
    mut intents: MessageReader<UserIntent>,
    mut cards: MessageWriter<fixture_turn_based::CardPlayed>,
) {
    for intent in intents.read() {
        if let UserIntent::Choice { index } = intent {
            cards.write(fixture_turn_based::CardPlayed {
                card_id: format!("card_{}", index),
            });
        }
    }
}

pub struct TurnBasedAdapterPlugin;

impl Plugin for TurnBasedAdapterPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<fixture_turn_based::CardPlayed>();
        app.add_systems(Update, turn_based_intent_bridge);
        app.add_systems(
            Update,
            |mut commands: Commands, q: Query<Entity, Added<fixture_turn_based::PlayerCard>>| {
                for e in &q {
                    commands.entity(e).insert((Gameplay, TurnBased));
                }
            },
        );
        // FX1: game-owned resources (Score) must count as liveness —
        // the fixture keeps ALL state in resources.
        app.insert_resource(bevy_swarm::game_types::GameTypes::from_plugin::<
            fixture_turn_based::TurnBasedGamePlugin,
        >());
    }
}

// ============================================================================
// Adapter: RegressionPair — tag fixture entities as Gameplay
// ============================================================================

pub struct RegressionPairAdapterPlugin;

impl Plugin for RegressionPairAdapterPlugin {
    fn build(&self, app: &mut App) {
        // Damage is a resource, not entity-based; no entities to tag.
        // Register score exposure for state anchoring via a counter entity.
        app.add_systems(Startup, |mut commands: Commands| {
            commands.spawn((Gameplay, Transform::default()));
        });
        // FX1: game-owned resources (Damage) count as liveness.
        app.insert_resource(bevy_swarm::game_types::GameTypes::from_plugin::<
            fixture_regression_pair::RegressionPairGamePlugin,
        >());
    }
}

// ============================================================================
// Report writer
// ============================================================================

pub fn write_report(
    report: &bevy_swarm::driver::PlaytestReport,
    run_id: &str,
) -> std::path::PathBuf {
    let output_dir = conventions::runs_dir().join(run_id);
    fs::create_dir_all(&output_dir).expect("create runs dir");
    let path = output_dir.join("report.json");
    let json = serde_json::to_string_pretty(report).expect("serialize report");
    fs::write(&path, json).expect("write report");
    path
}
