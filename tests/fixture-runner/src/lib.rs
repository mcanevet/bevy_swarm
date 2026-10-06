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
        // RealTime marker: arms the frozen-world oracle (TurnBased exempts).
        app.add_systems(
            Startup,
            |mut commands: Commands,
             q: Query<Entity, With<fixture_spinner::Spinner>>| {
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
            Startup,
            |mut commands: Commands,
             q: Query<Entity, With<fixture_walker::Player>>| {
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
            |mut commands: Commands,
             q: Query<Entity, With<fixture_spawner::LeakyEntity>>| {
                for e in &q {
                    commands.entity(e).insert(Gameplay);
                }
            },
        );
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
