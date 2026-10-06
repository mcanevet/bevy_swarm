//! Fixture: walker (clean vs multiple bug variants)
//!
//! Clean: player moves in response to Move events.
//! Bugs:
//!   - desync: input wiring broken (never consumes events)
//!   - dead_left_key: ArrowLeft key never wired
//!
//! Bug selection: set `FIXTURE_BUG=<variant>` at runtime.

use bevy::prelude::*;

#[derive(Component)]
pub struct Player;

#[derive(Message, Clone, Debug)]
pub struct MoveEvent {
    pub dir: Vec2,
}

fn setup(mut commands: Commands) {
    commands.spawn((Player, Transform::default()));
}

/// Clean: consume Move events and move the player.
fn move_system_clean(
    mut events: MessageReader<MoveEvent>,
    mut q: Query<&mut Transform, With<Player>>,
) {
    for ev in events.read() {
        let d = Vec3::new(ev.dir.x, ev.dir.y, 0.0);
        for mut t in &mut q {
            t.translation += d;
        }
    }
}

/// Buggy: input wiring broken — never consumes events.
fn move_system_buggy(_events: MessageReader<MoveEvent>, _q: Query<&mut Transform, With<Player>>) {}

/// Buggy: dead_left_key — the game drops all "move left" intents.
/// Leftward movement is a declared verb on the surface but never takes
/// effect: a dead verb.
fn move_system_dead_left(
    mut events: MessageReader<MoveEvent>,
    mut q: Query<&mut Transform, With<Player>>,
) {
    for ev in events.read() {
        if ev.dir.x < 0.0 {
            continue; // left moves silently dropped — dead verb
        }
        let d = Vec3::new(ev.dir.x, ev.dir.y, 0.0);
        for mut t in &mut q {
            t.translation += d;
        }
    }
}

pub struct WalkerGamePlugin;

impl Plugin for WalkerGamePlugin {
    fn build(&self, app: &mut App) {
        let bug = std::env::var("FIXTURE_BUG").ok().map(|s| s.to_lowercase());

        app.add_message::<MoveEvent>();
        app.add_systems(Startup, setup);

        match bug.as_deref() {
            Some("desync") => {
                app.add_systems(Update, move_system_buggy);
            }
            Some("dead_left_key") => {
                app.add_systems(Update, move_system_dead_left);
            }
            _ => {
                app.add_systems(Update, move_system_clean);
            }
        }
    }
}
