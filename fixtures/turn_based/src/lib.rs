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

/// Buggy: softlock_turn_4 — once turn 4 is reached, the turn never
/// advances again: every subsequent card is swallowed (soft-lock).
fn card_played_softlock(
    mut events: MessageReader<CardPlayed>,
    mut score: ResMut<Score>,
    mut turn: Local<u32>,
) {
    for _ev in events.read() {
        if *turn >= 4 {
            // Turn 4+ never advances — intents swallowed forever.
            continue;
        }
        *turn += 1;
        score.0 += 10.0;
    }
}

pub struct TurnBasedGamePlugin;

impl Plugin for TurnBasedGamePlugin {
    fn build(&self, app: &mut App) {
        let bug = std::env::var("FIXTURE_BUG").ok().map(|s| s.to_lowercase());

        app.add_message::<CardPlayed>();
        app.add_systems(Startup, setup);

        known_bug(
            &bug,
            &[
                "checkop_boundary",
                "softlock_turn_4",
                "thread_rng",
                "hashmap_order",
                "unconsumed_event",
            ],
        );
        match bug.as_deref() {
            Some("checkop_boundary") => {
                app.add_systems(Update, card_played_buggy);
            }
            Some("softlock_turn_4") => {
                app.add_systems(Update, card_played_softlock);
            }
            Some("thread_rng") => {
                // Pending (swarm-1w8.13): nondeterministic entropy —
                // clean behavior otherwise, expectation marked pending.
                app.add_systems(Update, card_played_system);
            }
            Some("hashmap_order") => {
                // Pending (swarm-1w8.13): iteration-order nondeterminism.
                app.add_systems(Update, card_played_system);
            }
            Some("unconsumed_event") => {
                // Pending (swarm-716.24): unconsumed_message oracle.
                app.add_systems(Update, card_played_system);
            }
            _ => {
                app.add_systems(Update, card_played_system);
            }
        }
    }
}

/// FX2: an unknown FIXTURE_BUG is a typo'd test case — panic loudly
/// instead of silently running the clean game (vacuous pass).
fn known_bug(bug: &Option<String>, known: &[&str]) {
    if let Some(b) = bug.as_deref() {
        if !known.contains(&b) {
            panic!("unknown FIXTURE_BUG '{}'. Known: {:?}", b, known);
        }
    }
}
