use bevy::prelude::*;
use fixture_walker::WalkerGamePlugin;

fn main() {
    App::new()
        .add_plugins((
            DefaultPlugins.set(WindowPlugin {
                primary_window: Some(Window {
                    title: "Fixture: Walker".into(),
                    ..default()
                }),
                ..default()
            }),
            WalkerGamePlugin,
        ))
        .run();
}
