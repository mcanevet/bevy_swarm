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
    // Clean: base damage; v2_damage: regression to the old formula.
    damage.0 = DAMAGE
        .get()
        .copied()
        .unwrap_or(Bug::Clean)
        .formula(damage.0);
}

/// v2_slow_menu only lengthens a MENU animation — it must NOT diverge
/// game state (the Z11 state-anchoring contract). Represented by a
/// slower cosmetic MenuAnim resource, never touching Damage.
#[derive(Resource, Default)]
pub struct MenuAnim {
    pub speed: f32,
}

fn menu_anim_system(mut menu: ResMut<MenuAnim>) {
    menu.speed = DAMAGE.get().copied().unwrap_or(Bug::Clean).menu_speed();
}

/// Selected bug, resolved ONCE at plugin build (unknown values panic).
#[derive(Clone, Copy)]
enum Bug {
    Clean,
    V2Damage,
    V2SlowMenu,
}

impl Bug {
    fn formula(self, _input: f64) -> f64 {
        match self {
            Bug::Clean => 10.0,
            Bug::V2Damage => 15.0,
            _ => 10.0,
        }
    }
    fn menu_speed(self) -> f32 {
        match self {
            Bug::V2SlowMenu => 0.05, // 20x slower menu animation
            _ => 1.0,
        }
    }
}

static DAMAGE: std::sync::OnceLock<Bug> = std::sync::OnceLock::new();

pub struct RegressionPairGamePlugin;

impl Plugin for RegressionPairGamePlugin {
    fn build(&self, app: &mut App) {
        let bug = match std::env::var("FIXTURE_BUG")
            .ok()
            .map(|s| s.to_lowercase())
            .as_deref()
        {
            None => Bug::Clean,
            Some("v2_damage") => Bug::V2Damage,
            Some("v2_slow_menu") => Bug::V2SlowMenu,
            Some(other) => panic!(
                "unknown FIXTURE_BUG '{}'. Known: [\"v2_damage\", \"v2_slow_menu\"]",
                other
            ),
        };
        let _ = DAMAGE.set(bug);
        app.add_systems(Startup, |mut commands: Commands| {
            commands.init_resource::<MenuAnim>();
        });
        app.add_systems(Startup, setup);
        app.add_systems(Update, (damage_system, menu_anim_system));
    }
}

/// FX2: an unknown FIXTURE_BUG is a typo'd test case — panic loudly
/// instead of silently running the clean game (vacuous pass).
#[allow(dead_code)]
fn known_bug(bug: &Option<String>, known: &[&str]) {
    if let Some(b) = bug.as_deref() {
        if !known.contains(&b) {
            panic!("unknown FIXTURE_BUG '{}'. Known: {:?}", b, known);
        }
    }
}
