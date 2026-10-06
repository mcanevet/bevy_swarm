//! Fixture: walker (clean vs buggy)
//!
//! Clean: player moves in response to Move events.
//! Buggy: input wiring broken — player never moves despite Move events.
//!
//! Bug selection: set `FIXTURE_BUG=desync` at runtime.

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

pub struct WalkerGamePlugin;

impl Plugin for WalkerGamePlugin {
    fn build(&self, app: &mut App) {
        let buggy = std::env::var("FIXTURE_BUG")
            .ok()
            .map(|s| s.to_lowercase())
            .as_deref()
            == Some("desync");

        app.add_message::<MoveEvent>();
        app.add_systems(Startup, setup);

        if buggy {
            app.add_systems(Update, move_system_buggy);
        } else {
            app.add_systems(Update, move_system_clean);
        }
    }
}
