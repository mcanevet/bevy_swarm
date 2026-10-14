use bevy::prelude::*;
use fixture_multithread_panic::MultithreadPanicGamePlugin;

fn main() {
    App::new()
        .add_plugins(MinimalPlugins)
        .add_plugins(MultithreadPanicGamePlugin)
        .run();
}
