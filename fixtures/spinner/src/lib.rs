//! Fixture: spinner (clean vs buggy)
//!
//! Clean: entities rotate continuously.
//! Buggy: rotation stops after frame 30 (simulated soft-lock).
//!
//! Bug selection: set `FIXTURE_BUG=frozen` at runtime (env var read by the game).

use bevy::prelude::*;

#[derive(Component)]
pub struct Spinner;

fn spin_system(time: Res<Time>, mut q: Query<&mut Transform, With<Spinner>>) {
    for mut t in &mut q {
        t.rotate_z(0.1 + time.delta_secs() * 60.0);
    }
}

/// Buggy: rotation stops after frame 30 — simulated soft-lock.
fn spin_system_buggy(
    mut frame: Local<u64>,
    time: Res<Time>,
    mut q: Query<&mut Transform, With<Spinner>>,
) {
    *frame += 1;
    if *frame > 30 {
        return;
    }
    for mut t in &mut q {
        t.rotate_z(0.1 + time.delta_secs() * 60.0);
    }
}

pub struct SpinnerGamePlugin;

impl Plugin for SpinnerGamePlugin {
    fn build(&self, app: &mut App) {
        let bug = std::env::var("FIXTURE_BUG").ok().map(|s| s.to_lowercase());
        known_bug(&bug, &["frozen"]);
        let buggy = bug.as_deref() == Some("frozen");

        app.add_systems(Startup, |mut commands: Commands| {
            commands.spawn((Spinner, Transform::default()));
        });

        if buggy {
            app.add_systems(Update, spin_system_buggy);
        } else {
            app.add_systems(Update, spin_system);
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
