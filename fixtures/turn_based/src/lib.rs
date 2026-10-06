//! Fixture: turn_based (card game with AI)
//!
//! Clean: AI consumes CardPlayed events, ends turn on Enter.
//! Bugs:
//!   - checkop_boundary: score reaches exactly threshold (tests boundary semantics)
//!
//! Bug selection: set `FIXTURE_BUG=<variant>` at runtime.

use bevy::prelude::*;

#[derive(Component)]
pub struct PlayerCard;

#[derive(Message, Clone, Debug)]
pub struct CardPlayed {
    pub card_id: String,
}

#[derive(Resource, Default)]
pub struct Score(pub f64);

fn setup(mut commands: Commands) {
    commands.spawn(PlayerCard);
    commands.init_resource::<Score>();
}

fn card_played_system(mut events: MessageReader<CardPlayed>, mut score: ResMut<Score>) {
    for _ev in events.read() {
        // Each card adds 10 points
        score.0 += 10.0;
    }
}

/// Buggy: score hits exactly 50.0 (boundary case for Below vs Equals).
fn card_played_buggy(mut events: MessageReader<CardPlayed>, mut score: ResMut<Score>) {
    for _ev in events.read() {
        // Force score to exactly 50.0 (boundary)
        score.0 = 50.0;
    }
}

pub struct TurnBasedGamePlugin;

impl Plugin for TurnBasedGamePlugin {
    fn build(&self, app: &mut App) {
        let bug = std::env::var("FIXTURE_BUG").ok().map(|s| s.to_lowercase());

        app.add_message::<CardPlayed>();
        app.add_systems(Startup, setup);

        match bug.as_deref() {
            Some("checkop_boundary") => {
                app.add_systems(Update, card_played_buggy);
            }
            _ => {
                app.add_systems(Update, card_played_system);
            }
        }
    }
}
