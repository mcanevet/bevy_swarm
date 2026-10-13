use bevy::prelude::*;

#[derive(Clone, Default)]
struct GamePlugin;

impl Plugin for GamePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, || {});
    }
}

fn main() {
    App::new().add_plugins(GamePlugin).run();
}
