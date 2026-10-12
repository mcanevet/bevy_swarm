//! Fixture: spawner (clean vs buggy)
//!
//! Clean: spawns exactly 10 entities, then stops.
//! Buggy: spawns entities every frame forever (memory leak).
//!
//! Bug selection: set `FIXTURE_BUG=leak` at runtime.

use bevy::ecs::reflect::AppTypeRegistry;
use bevy::prelude::*;

#[derive(Component, Reflect)]
#[reflect(Component)]
pub struct LeakyEntity;

/// Idle-phase resource: a real "attract mode" — even a quiescent,
/// self-limiting game normally animates (menus breathe, sprites
/// pulse). le3.8 (RF4): the old liveness:off exemption is gone; the
/// clean spawner stays live through this genuine gameplay state.
#[derive(Resource, Reflect, Default)]
#[reflect(Resource)]
pub struct IdlePhase(pub f32);

fn spawn_clean_system(
    mut commands: Commands,
    mut frame: Local<u64>,
    mut phase: ResMut<IdlePhase>,
    time: Res<Time>,
) {
    if *frame < 10 {
        commands.spawn((LeakyEntity, Transform::default()));
    }
    *frame += 1;
    // Genuine idle animation: the spawn indicator pulses.
    phase.0 = (time.elapsed_secs() * 2.0).sin();
}

fn spawn_buggy_system(mut commands: Commands) {
    commands.spawn((LeakyEntity, Transform::default()));
}

#[derive(Clone)]
pub struct SpawnerGamePlugin;

impl Plugin for SpawnerGamePlugin {
    fn build(&self, app: &mut App) {
        let bug = std::env::var("FIXTURE_BUG").ok().map(|s| s.to_lowercase());
        known_bug(&bug, &["leak"]);
        let buggy = bug.as_deref() == Some("leak");

        let registry = app.world_mut().resource_mut::<AppTypeRegistry>();
        registry.0.write().register::<LeakyEntity>();
        registry.0.write().register::<IdlePhase>();
        app.init_resource::<IdlePhase>();

        if buggy {
            app.add_systems(Update, spawn_buggy_system);
        } else {
            app.add_systems(Update, spawn_clean_system);
        }
    }
}

/// FX2: an unknown FIXTURE_BUG is a typo'd test case — panic loudly
/// instead of silently running the clean game (vacuous pass).
fn known_bug(bug: &Option<String>, known: &[&str]) {
    if let Some(b) = bug.as_deref() {
        if !known.contains(&b) {
            panic!("unknown FIXTURE_BUG '{}'. Known: {:?}", b, known);
        }
    }
}
