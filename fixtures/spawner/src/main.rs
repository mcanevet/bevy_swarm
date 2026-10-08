use bevy::prelude::*;

use fixture_spawner::SpawnerGamePlugin;

fn main() {
    App::new()
        .add_plugins((
            DefaultPlugins.set(WindowPlugin {
                primary_window: Some(Window {
                    title: "Fixture: Spawner".into(),
                    ..default()
                }),
                ..default()
            }),
            SpawnerGamePlugin,
        ))
        .run();
}
