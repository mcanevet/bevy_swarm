//! Fixture: multithread_panic (pool-thread panic capture — dde)
//!
//! The game spawns work on the process-global bevy_tasks
//! AsyncComputeTaskPool. Under BUG=panic a background task panics on
//! a pool thread — historically invisible to the harness (panics were
//! captured via a driver-thread thread-local run id only).

use bevy::prelude::*;
use bevy::tasks::AsyncComputeTaskPool;

#[derive(Resource, Default)]
pub struct FrameCounter(pub u32);

fn setup(mut commands: Commands) {
    commands.spawn((Name::new("multithread_panic_fixture"), Transform::default()));
}

/// Moves the transform slightly every frame — keeps the game
/// observable/liveness-nonzero in clean mode.
fn animate(mut query: Query<&mut Transform, With<Name>>, counter: Res<FrameCounter>) {
    for mut t in query.iter_mut() {
        t.translation.x = counter.0 as f32 * 0.01;
    }
}

fn spawn_background_task(counter: Res<FrameCounter>) {
    // Fire once per ~2s so short scenarios still hit it, and repeat
    // so straggler-attribution tests (multiple seeds) get coverage.
    if counter.0 > 0
        && counter.0.is_multiple_of(60)
        && std::env::var("FIXTURE_BUG").as_deref() == Ok("panic")
    {
        let task = AsyncComputeTaskPool::get().spawn(async move {
            // Small sleep so the panic genuinely lands mid-run,
            // not deterministically inside spawn frame.
            std::thread::sleep(std::time::Duration::from_millis(50));
            panic!("Background task panic from AsyncComputeTaskPool");
        });
        task.detach();
    }
}

fn count_frames(mut counter: ResMut<FrameCounter>) {
    counter.0 += 1;
}

pub struct MultithreadPanicGamePlugin;

impl Plugin for MultithreadPanicGamePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<FrameCounter>()
            .add_systems(Startup, setup)
            .add_systems(
                Update,
                (animate, spawn_background_task, count_frames).chain(),
            );
    }
}
