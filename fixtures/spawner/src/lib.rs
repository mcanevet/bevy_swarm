//! Fixture: spawner (clean vs buggy)
//!
//! Clean: spawns exactly 10 entities, then stops.
//! Buggy: spawns entities every frame forever (memory leak).
//!
//! Bug selection: set `FIXTURE_BUG=leak` at runtime.

use bevy::prelude::*;
use bevy::ecs::reflect::AppTypeRegistry;

#[derive(Component, Reflect)]
#[reflect(Component)]
pub struct LeakyEntity;

fn spawn_clean_system(mut commands: Commands, mut frame: Local<u64>) {
    if *frame < 10 {
        commands.spawn((LeakyEntity, Transform::default()));
    }
    *frame += 1;
}

fn spawn_buggy_system(mut commands: Commands) {
    commands.spawn((LeakyEntity, Transform::default()));
}

pub struct SpawnerGamePlugin;

impl Plugin for SpawnerGamePlugin {
    fn build(&self, app: &mut App) {
        let buggy = std::env::var("FIXTURE_BUG")
            .ok()
            .map(|s| s.to_lowercase())
            .as_deref()
            == Some("leak");

        let registry = app.world_mut().resource_mut::<AppTypeRegistry>();
        registry.0.write().register::<LeakyEntity>();

        if buggy {
            app.add_systems(Update, spawn_buggy_system);
        } else {
            app.add_systems(Update, spawn_clean_system);
        }
    }
}
