use bevy::prelude::*;

#[derive(Resource, Default)]
struct FrameCounter(usize);

fn exit_after(mut counter: ResMut<FrameCounter>) {
    counter.0 += 1;
    if counter.0 > 200 {
        // Clean exit (the fixture simulates a successful run; the
        // exact mechanism is unimportant to the smoke test).
        std::process::exit(0);
    }
}

fn main() {
    App::new()
        .add_plugins(MinimalPlugins)
        .init_resource::<FrameCounter>()
        .add_systems(Update, exit_after)
        .run();
}
