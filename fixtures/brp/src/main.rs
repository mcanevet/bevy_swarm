//! Z20 fixture: a pure-Bevy game exposing BRP for the external
//! observe-only tier. No harness dependency.
//!
//! Moves an entity forever so BRP state polling observes a changing
//! world (liveness), and exits cleanly after N frames.

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
    // Headless-ish DefaultPlugins (no audio/render avoids GPU deps while
    // keeping the full task-pool/async machinery that BRP needs).
    // WinitPlugin is disabled (no window) so we supply our own
    // ScheduleRunnerPlugin to drive the app loop.
    let plugins = DefaultPlugins
        .build()
        .disable::<bevy::winit::WinitPlugin>()
        .add(bevy::app::ScheduleRunnerPlugin::run_loop(
            std::time::Duration::from_secs_f64(1.0 / 60.0),
        ));
    App::new()
        .add_plugins(plugins)
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
