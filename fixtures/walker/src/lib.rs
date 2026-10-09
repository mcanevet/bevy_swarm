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

/// Logical position (game state) — separate from visual Transform.
/// Desync bug: Transform moves but LogicalPosition stays frozen.
/// Pure-bevy component; the harness oracle finds it by type name
/// via reflection (games never import harness types).
#[derive(Component, Reflect, Default, Clone, Copy)]
#[reflect(Component)]
pub struct LogicalPosition(pub Vec2);

#[derive(Message, Clone, Debug)]
pub struct MoveEvent {
    pub dir: Vec2,
}

fn setup(mut commands: Commands) {
    commands.spawn((Player, Transform::default(), LogicalPosition::default()));
}

/// Tier-0 input: translate keyboard state (ButtonInput<KeyCode>) into
/// MoveEvents. Plain Bevy — a zero-contract run can drive this game.
fn keyboard_system(keys: Res<ButtonInput<KeyCode>>, mut events: MessageWriter<MoveEvent>) {
    let mut dir = Vec2::ZERO;
    if keys.pressed(KeyCode::ArrowUp) || keys.pressed(KeyCode::KeyW) {
        dir.y += 1.0;
    }
    if keys.pressed(KeyCode::ArrowDown) || keys.pressed(KeyCode::KeyS) {
        dir.y -= 1.0;
    }
    if keys.pressed(KeyCode::ArrowRight) || keys.pressed(KeyCode::KeyD) {
        dir.x += 1.0;
    }
    if keys.pressed(KeyCode::ArrowLeft) || keys.pressed(KeyCode::KeyA) {
        dir.x -= 1.0;
    }
    if dir != Vec2::ZERO {
        events.write(MoveEvent { dir });
    }
}

/// Tier-0 input with the dead_left_key bug: ArrowLeft is never wired
/// (both Left arrows), so pressing it moves nothing — a dead verb.
fn keyboard_system_dead_left(
    keys: Res<ButtonInput<KeyCode>>,
    mut events: MessageWriter<MoveEvent>,
) {
    let mut dir = Vec2::ZERO;
    if keys.pressed(KeyCode::ArrowUp) || keys.pressed(KeyCode::KeyW) {
        dir.y += 1.0;
    }
    if keys.pressed(KeyCode::ArrowDown) || keys.pressed(KeyCode::KeyS) {
        dir.y -= 1.0;
    }
    if keys.pressed(KeyCode::ArrowRight) || keys.pressed(KeyCode::KeyD) {
        dir.x += 1.0;
    }
    // BUG: ArrowLeft / KeyA never wired — left is dead.
    if dir != Vec2::ZERO {
        events.write(MoveEvent { dir });
    }
}

/// Clean: consume Move events and move the player.
fn move_system_clean(
    mut events: MessageReader<MoveEvent>,
    mut q: Query<(&mut Transform, &mut LogicalPosition), With<Player>>,
) {
    for ev in events.read() {
        let d = Vec3::new(ev.dir.x, ev.dir.y, 0.0);
        for (mut t, mut lp) in &mut q {
            t.translation += d;
            lp.0 += Vec2::new(ev.dir.x, ev.dir.y);
        }
    }
}

/// Buggy: desync — Transform moves but LogicalPosition stays frozen.
/// Detectable: rendered position ≠ logical position.
fn move_system_desync(
    mut events: MessageReader<MoveEvent>,
    mut q: Query<(&mut Transform, &mut LogicalPosition), With<Player>>,
) {
    for ev in events.read() {
        let d = Vec3::new(ev.dir.x, ev.dir.y, 0.0);
        for (mut t, _lp) in &mut q {
            t.translation += d;
            // BUG: LogicalPosition not updated — desync!
        }
    }
}

/// Buggy: dead_left_key — the game drops all "move left" intents.
/// Leftward movement is a declared verb on the surface but never takes
/// effect: a dead verb.
fn move_system_dead_left(
    mut events: MessageReader<MoveEvent>,
    mut q: Query<(&mut Transform, &mut LogicalPosition), With<Player>>,
) {
    for ev in events.read() {
        if ev.dir.x < 0.0 {
            continue; // left moves silently dropped — dead verb
        }
        let d = Vec3::new(ev.dir.x, ev.dir.y, 0.0);
        for (mut t, mut lp) in &mut q {
            t.translation += d;
            lp.0 += Vec2::new(ev.dir.x, ev.dir.y);
        }
    }
}

/// Child entity for the child_out_of_bounds bug: a sprite whose
/// child sits outside the bounds the invariant checks.
#[derive(Component)]
pub struct ChildSprite;

/// Buggy: stuck_corner — the player wedges when trying to move
/// diagonally AWAY FROM THE ORIGIN (corner-like behavior at start).
/// This triggers reliably on the first diagonal intent.
fn move_system_stuck_corner(
    mut events: MessageReader<MoveEvent>,
    mut q: Query<(&mut Transform, &mut LogicalPosition), With<Player>>,
) {
    for ev in events.read() {
        for (mut t, mut lp) in &mut q {
            let dist = t.translation.length();
            let diagonal = ev.dir.x != 0.0 && ev.dir.y != 0.0;
            let away_from_origin = (ev.dir.x * t.translation.x + ev.dir.y * t.translation.y) > 0.0;
            if dist < 1.0 && diagonal && away_from_origin {
                continue; // stuck at corner (origin) — input swallowed
            }
            t.translation += Vec3::new(ev.dir.x, ev.dir.y, 0.0);
            lp.0 += Vec2::new(ev.dir.x, ev.dir.y);
        }
    }
}

/// Buggy: nan_rotation — a rotation built from a zero axis produces a
/// NaN quaternion, then propagates.
fn move_system_nan_rotation(
    mut events: MessageReader<MoveEvent>,
    mut q: Query<(&mut Transform, &mut LogicalPosition), With<Player>>,
) {
    for ev in events.read() {
        let d = Vec3::new(ev.dir.x, ev.dir.y, 0.0);
        for (mut t, mut lp) in &mut q {
            t.translation += d;
            lp.0 += Vec2::new(ev.dir.x, ev.dir.y);
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

#[derive(Clone, Default)]
pub struct WalkerGamePlugin;

impl Plugin for WalkerGamePlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<LogicalPosition>();
        let bug = std::env::var("FIXTURE_BUG").ok().map(|s| s.to_lowercase());

        app.add_message::<MoveEvent>();
        app.add_systems(Startup, setup);
        // Clean: keyboard drives MoveEvents.
        app.add_systems(Update, keyboard_system);

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
                app.add_systems(Update, move_system_desync);
            }
            Some("dead_left_key") => {
                // Buggy keyboard (ArrowLeft dead) + buggy move handler.
                app.add_systems(Update, (keyboard_system_dead_left, move_system_dead_left));
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
