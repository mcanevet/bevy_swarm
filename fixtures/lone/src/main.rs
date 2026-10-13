use bevy::prelude::*;
use fixture_lone::LoneGamePlugin;

fn main() {
    App::new()
        .add_plugins((
            DefaultPlugins.set(WindowPlugin {
                primary_window: Some(Window {
                    title: "Fixture: Lone (zero-contract)".into(),
                    ..default()
                }),
                ..default()
            }),
            LoneGamePlugin,
        ))
        .run();
}
