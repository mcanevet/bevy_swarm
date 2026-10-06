//! Fixture: turn_based game (cards vs AI; Enter ends turn)
//!
//! Clean: cards played incrementally add score.
//! Bugs (runtime-select via FIXTURE_BUG):
//!   - checkop_boundary: score is forced to hit exactly 50.0 (boundary check semantics)

use bevy::prelude::*;

fn main() {
    let mut app = App::new();
    app.add_plugins(fixture_turn_based::TurnBasedGamePlugin);
    app.run();
}
