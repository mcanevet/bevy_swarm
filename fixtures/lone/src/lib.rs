//! Standalone zero-contract game: pure Bevy, no test harness anywhere.
//! Moves an entity on arrow keys — drivable by raw chaos input.

use bevy::prelude::*;

#[derive(Component)]
pub struct LoneMarker;

#[derive(Message, Debug, Clone)]
pub enum MoveEvent {
    Left,
    Right,
    Up,
    Down,
}

pub struct LoneGamePlugin;

impl Plugin for LoneGamePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_player);
        app.add_systems(Update, move_player);
    }
}

fn spawn_player(mut cmds: Commands) {
    cmds.spawn((Name::new("Player"), Transform::default(), LoneMarker));
}

fn move_player(keys: Res<ButtonInput<KeyCode>>, mut q: Query<&mut Transform, With<LoneMarker>>) {
    let mut dir = Vec2::ZERO;
    if keys.pressed(KeyCode::ArrowLeft) {
        dir.x -= 1.0;
    }
    if keys.pressed(KeyCode::ArrowRight) {
        dir.x += 1.0;
    }
    if keys.pressed(KeyCode::ArrowUp) {
        dir.y += 1.0;
    }
    if keys.pressed(KeyCode::ArrowDown) {
        dir.y -= 1.0;
    }
    for mut t in &mut q {
        t.translation += dir.extend(0.0) * 0.1;
    }
}
