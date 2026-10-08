//! Fixture spinner: real playable binary (headless test uses lib only)

use bevy::prelude::*;

use fixture_spinner::SpinnerGamePlugin;

fn main() {
    App::new()
        .add_plugins((
            DefaultPlugins.set(WindowPlugin {
                primary_window: Some(Window {
                    title: "Fixture: Spinner".into(),
                    ..default()
                }),
                ..default()
            }),
            SpinnerGamePlugin,
        ))
        .run();
}
