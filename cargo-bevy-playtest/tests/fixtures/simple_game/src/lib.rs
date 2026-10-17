use bevy::prelude::*;

#[derive(Component)]
pub struct Player;

#[derive(Clone, Default)]
pub struct GamePlugin;

impl Plugin for GamePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup);
    }
}

fn setup(mut commands: Commands) {
    commands.spawn(Player);
}
