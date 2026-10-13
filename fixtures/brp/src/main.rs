//! Z20 fixture: a pure-Bevy game exposing BRP for the external
//! observe-only tier. No harness dependency.
//!
//! Advances a counter every frame so BRP state polling observes a
//! changing world (liveness), and exits cleanly after N frames.

use bevy::app::ScheduleRunnerPlugin;
use bevy::prelude::*;
use bevy::remote::http::RemoteHttpPlugin;
use bevy::remote::RemotePlugin;

#[derive(Resource, Default)]
struct FrameCounter(usize);

#[derive(Component, bevy::reflect::Reflect)]
#[reflect(Component)]
struct Walker {
    position: usize,
}

fn walk(mut query: Query<&mut Walker>) {
    for mut walker in &mut query {
        walker.position += 1;
    }
}

fn exit_after(frames: Res<FrameCounter>) {
    if frames.0 > 3_000 {
        std::process::exit(0);
    }
}

fn setup(mut commands: Commands) {
    commands.spawn(Walker { position: 0 });
}

fn main() {
    App::new()
        .add_plugins(MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(
            std::time::Duration::from_secs_f64(1.0 / 60.0),
        )))
        .add_plugins(RemotePlugin::default())
        .add_plugins(RemoteHttpPlugin::default())
        .init_resource::<FrameCounter>()
        .add_systems(Startup, setup)
        .add_systems(
            Update,
            (|mut counter: ResMut<FrameCounter>| counter.0 += 1, walk),
        )
        .add_systems(Update, exit_after)
        .run();
}
