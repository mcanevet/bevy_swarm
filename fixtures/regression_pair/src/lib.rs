//! Fixture: regression_pair (one game; FIXTURE_BUG=v2_damage changes damage formula)
//!
//! Clean: damage = 10.0
//! Bugs:
//!   - v2_damage: damage = 15.0 (regression case for golden divergence)
//!   - v2_slow_menu: menu animation slowed (should not diverge with T11 state anchoring)

use bevy::prelude::*;

#[derive(Resource, Default)]
pub struct Damage(pub f64);

fn setup(mut commands: Commands) {
    commands.init_resource::<Damage>();
}

fn damage_system(mut damage: ResMut<Damage>) {
    // Clean: base damage
    let bug = std::env::var("FIXTURE_BUG").ok().map(|s| s.to_lowercase());

    match bug.as_deref() {
        Some("v2_damage") => {
            damage.0 = 15.0;
        }
        _ => {
            damage.0 = 10.0;
        }
    }
}

pub struct RegressionPairGamePlugin;

impl Plugin for RegressionPairGamePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup);
        app.add_systems(Update, damage_system);
    }
}
