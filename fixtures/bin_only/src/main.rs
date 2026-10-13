use bevy::prelude::*;

fn panic_system() {
    panic!("simulated crash at frame 120");
}

fn main() {
    App::new()
        .add_plugins(MinimalPlugins)
        .add_systems(Startup, panic_system)
        .run();
}
