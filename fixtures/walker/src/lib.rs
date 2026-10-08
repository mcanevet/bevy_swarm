//! Fixture: walker (clean vs multiple bug variants)
//!
//! Clean: player moves in response to Move events.
//! Bugs:
//!   - desync: input wiring broken (never consumes events)
//!   - dead_left_key: ArrowLeft key never wired
//!
//! Bug selection: set `FIXTURE_BUG=<variant>` at runtime.

use bevy::prelude::*;

#[derive(Component)]
pub struct Player;

#[derive(Message, Clone, Debug)]
pub struct MoveEvent {
    pub dir: Vec2,
}

fn setup(mut commands: Commands) {
    commands.spawn((Player, Transform::default()));
}

/// Clean: consume Move events and move the player.
fn move_system_clean(
    mut events: MessageReader<MoveEvent>,
    mut q: Query<&mut Transform, With<Player>>,
) {
    for ev in events.read() {
        let d = Vec3::new(ev.dir.x, ev.dir.y, 0.0);
        for mut t in &mut q {
            t.translation += d;
        }
    }
}

/// Buggy: input wiring broken — never consumes events.
fn move_system_buggy(_events: MessageReader<MoveEvent>, _q: Query<&mut Transform, With<Player>>) {}

/// Buggy: dead_left_key — the game drops all "move left" intents.
/// Leftward movement is a declared verb on the surface but never takes
/// effect: a dead verb.
fn move_system_dead_left(
    mut events: MessageReader<MoveEvent>,
    mut q: Query<&mut Transform, With<Player>>,
) {
    for ev in events.read() {
        if ev.dir.x < 0.0 {
            continue; // left moves silently dropped — dead verb
        }
        let d = Vec3::new(ev.dir.x, ev.dir.y, 0.0);
        for mut t in &mut q {
            t.translation += d;
        }
    }
}

/// Child entity for the child_out_of_bounds bug: a sprite whose
/// child sits outside the bounds the invariant checks.
#[derive(Component)]
pub struct ChildSprite;

/// Buggy: stuck_corner — velocity is zeroed when moving diagonally, so
/// the player wedges in place forever (frozen world).
fn move_system_stuck_corner(
    mut events: MessageReader<MoveEvent>,
    mut q: Query<&mut Transform, With<Player>>,
) {
    for ev in events.read() {
        // Zero the velocity when BOTH axes are nonzero (corner bug).
        if ev.dir.x != 0.0 && ev.dir.y != 0.0 {
            continue;
        }
        let d = Vec3::new(ev.dir.x, ev.dir.y, 0.0);
        for mut t in &mut q {
            t.translation += d;
        }
    }
}

/// Buggy: nan_rotation — a rotation built from a zero axis produces a
/// NaN quaternion, then propagates.
fn move_system_nan_rotation(
    mut events: MessageReader<MoveEvent>,
    mut q: Query<&mut Transform, With<Player>>,
) {
    for ev in events.read() {
        let d = Vec3::new(ev.dir.x, ev.dir.y, 0.0);
        for mut t in &mut q {
            t.translation += d;
            // Classic bug: normalizing a zero vector (0/0) yields NaN.
            let zero = Vec3::ZERO.length(); // 0.0
            #[allow(clippy::eq_op)] // intentional 0/0 to produce NaN
            let s = zero / zero; // NaN
            t.rotation = Quat::from_xyzw(s, s, s, 1.0);
        }
    }
}

/// Buggy: panic_on_edge — index out of bounds when the player walks
/// off a hardcoded patrol array.
fn move_system_panic_on_edge(
    mut events: MessageReader<MoveEvent>,
    mut step: bevy::ecs::system::Local<usize>,
    mut q: Query<&mut Transform, With<Player>>,
) {
    let patrol = [Vec3::X, Vec3::NEG_X];
    for ev in events.read() {
        let _ = &mut step;
        let _ = &mut q;
        if ev.dir.length_squared() > 0.5 {
            // Walked past the last patrol point → OOB panic.
            let idx = patrol.len();
            let _ = patrol[idx];
        }
    }
}

/// Buggy: panic_on_load — panics in a Startup asset-loading system.
fn panic_on_load_setup() {
    panic!("asset 'player.png' failed to load: corrupt PNG header");
}

/// Target component referencing another entity (for the dangling
/// reference oracle): despawn the target so the reference dangles.
#[derive(Component, Debug)]
pub struct ChaseTarget(pub Entity);

fn dangling_target_setup(mut commands: Commands) {
    let target = commands.spawn_empty().id();
    commands.entity(target).despawn();
    // The player now holds a dead reference.
    commands.spawn((Player, ChaseTarget(target), Transform::default()));
}

pub struct WalkerGamePlugin;

impl Plugin for WalkerGamePlugin {
    fn build(&self, app: &mut App) {
        let bug = std::env::var("FIXTURE_BUG").ok().map(|s| s.to_lowercase());

        app.add_message::<MoveEvent>();
        app.add_systems(Startup, setup);

        known_bug(
            &bug,
            &[
                "desync",
                "dead_left_key",
                "stuck_corner",
                "nan_rotation",
                "child_out_of_bounds",
                "panic_on_edge",
                "panic_on_load",
                "select_unnamed",
                "dangling_target",
            ],
        );
        match bug.as_deref() {
            Some("desync") => {
                app.add_systems(Update, move_system_buggy);
            }
            Some("dead_left_key") => {
                app.add_systems(Update, move_system_dead_left);
            }
            Some("stuck_corner") => {
                app.add_systems(Update, move_system_stuck_corner);
            }
            Some("nan_rotation") => {
                app.add_systems(Update, move_system_nan_rotation);
            }
            Some("child_out_of_bounds") => {
                app.add_systems(
                    Startup,
                    |mut commands: Commands, q: Query<Entity, With<Player>>| {
                        let _ = &q;
                        // Spawn at origin, drift right until out of bounds.
                        commands.spawn((
                            ChildSprite,
                            Name::new("drifting_child"),
                            Transform::default(),
                        ));
                    },
                );
                app.add_systems(
                    Update,
                    (
                        move_system_clean,
                        |mut q: Query<&mut Transform, With<ChildSprite>>, time: Res<Time>| {
                            for mut t in &mut q {
                                t.translation.x += 5000.0 * time.delta_secs();
                            }
                        },
                    ),
                );
            }
            Some("panic_on_edge") => {
                app.add_systems(Update, move_system_panic_on_edge);
            }
            Some("panic_on_load") => {
                app.add_systems(Startup, panic_on_load_setup);
            }
            Some("select_unnamed") => {
                app.add_systems(Update, move_system_clean);
            }
            Some("dangling_target") => {
                app.add_systems(Startup, dangling_target_setup);
                app.add_systems(Update, move_system_clean);
            }
            _ => {
                app.add_systems(Update, move_system_clean);
            }
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
