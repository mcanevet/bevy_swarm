use bevy::prelude::*;

#[derive(Component)]
pub struct Player;

pub fn plugin(app: &mut App) {
    app.add_systems(Startup, setup);
}

fn setup(mut commands: Commands) {
    commands.spawn(Player);
}
