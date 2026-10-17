//! swarm-uz8: pass_through rule — firing, silenced, and opt-out paths.

use bevy::prelude::*;
use bevy_playtest::contract::{IntentSurface, SurfaceVariant, TestConventionsPlugin};
use bevy_playtest::driver::{run_scenario, PlaytestPlugin};
use bevy_playtest::harness::Gameplay;
use bevy_playtest::scenario::Scenario;

fn build_app() -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        bevy::transform::TransformPlugin,
        TestConventionsPlugin,
        PlaytestPlugin,
    ));
    app.insert_resource(IntentSurface::new(vec![
        SurfaceVariant::Wait,
        SurfaceVariant::Choice(1),
    ]));
    app
}

fn scenario(extra: &str) -> Scenario {
    serde_json::from_str(&format!(
        r#"{{"bot":{{"type":"chaos","seed":1}},"duration_s":1.0,"tps":20,"simulated_time":true,"invariants":[]{}}}"#,
        extra
    ))
    .unwrap()
}

fn pass_through_count(app: &App) -> usize {
    app.world()
        .get_resource::<bevy_playtest::state::Violations>()
        .expect("violations present after run")
        .entries
        .values()
        .filter(|e| e.rule == "pass_through")
        .count()
}

/// Marker for the mover that travels straight through the obstacle.
#[derive(Component)]
struct Traveller;

/// Marker for the mover that bounces off its partner (collision).
#[derive(Component)]
struct Bouncer;

fn bodies() -> impl Bundle {
    (
        Gameplay,
        Sprite {
            custom_size: Some(Vec2::ONE),
            ..Default::default()
        },
    )
}

#[test]
fn pass_through_fires_when_collision_semantics_exist_elsewhere() {
    let mut app = build_app();
    // Obstacle + traveller: traveller crosses at constant velocity.
    app.world_mut()
        .spawn((Name::new("Obstacle"), bodies()))
        .insert(Transform::from_xyz(0.0, 0.0, 0.0));
    app.world_mut()
        .spawn((Name::new("Traveller"), bodies(), Traveller))
        .insert(Transform::from_xyz(-2.0, 0.0, 0.0));
    // Bouncer + partner: bouncer approaches then retreats (reversal).
    app.world_mut()
        .spawn((Name::new("Anchor"), bodies()))
        .insert(Transform::from_xyz(10.0, 10.0, 0.0));
    app.world_mut()
        .spawn((Name::new("Bouncer"), bodies(), Bouncer))
        .insert(Transform::from_xyz(10.0, 12.0, 0.0));

    app.add_systems(
        Update,
        |time: Res<Time>,
         mut p: ParamSet<(
            Query<&mut Transform, With<Traveller>>,
            Query<&mut Transform, With<Bouncer>>,
        )>| {
            for mut tr in &mut p.p0() {
                tr.translation.x += 4.0 * time.delta().as_secs_f32();
            }
            // Approach until deep overlap, then retreat (reversal).
            for mut bu in &mut p.p1() {
                if bu.translation.y > 10.55 {
                    bu.translation.y -= 4.0 * time.delta().as_secs_f32();
                } else {
                    bu.translation.y += 4.0 * time.delta().as_secs_f32();
                }
            }
        },
    );

    let report = run_scenario(&mut app, &scenario("")).unwrap();
    assert_eq!(
        pass_through_count(&app),
        1,
        "expected exactly one pass_through, status={:?}, rules fired: {:?}",
        report.status,
        app.world()
            .get_resource::<bevy_playtest::state::Violations>()
            .unwrap()
            .entries
            .values()
            .map(|e| e.rule.clone())
            .collect::<Vec<_>>()
    );
}

#[test]
fn pass_through_silent_when_no_collision_semantics_anywhere() {
    // Only the traveller + obstacle: no bouncer, no collision
    // anywhere — genre-neutral gate keeps the rule silent.
    let mut app = build_app();
    app.world_mut()
        .spawn((Name::new("Obstacle"), bodies()))
        .insert(Transform::from_xyz(0.0, 0.0, 0.0));
    app.world_mut()
        .spawn((Name::new("Traveller"), bodies(), Traveller))
        .insert(Transform::from_xyz(-2.0, 0.0, 0.0));
    app.add_systems(
        Update,
        |time: Res<Time>, mut q: Query<&mut Transform, With<Traveller>>| {
            for mut tr in &mut q {
                tr.translation.x += 4.0 * time.delta().as_secs_f32();
            }
        },
    );
    run_scenario(&mut app, &scenario("")).unwrap();
    assert_eq!(
        pass_through_count(&app),
        0,
        "no collision semantics anywhere — pass_through must stay silent"
    );
}

#[test]
fn pass_through_opt_out_via_collision_policy_off() {
    let mut app = build_app();
    app.world_mut()
        .spawn((Name::new("Obstacle"), bodies()))
        .insert(Transform::from_xyz(0.0, 0.0, 0.0));
    app.world_mut()
        .spawn((Name::new("Traveller"), bodies(), Traveller))
        .insert(Transform::from_xyz(-2.0, 0.0, 0.0));
    app.world_mut()
        .spawn((Name::new("Anchor"), bodies()))
        .insert(Transform::from_xyz(10.0, 10.0, 0.0));
    app.world_mut()
        .spawn((Name::new("Bouncer"), bodies(), Bouncer))
        .insert(Transform::from_xyz(10.0, 12.0, 0.0));
    app.add_systems(
        Update,
        |time: Res<Time>,
         mut p: ParamSet<(
            Query<&mut Transform, With<Traveller>>,
            Query<&mut Transform, With<Bouncer>>,
        )>| {
            for mut tr in &mut p.p0() {
                tr.translation.x += 4.0 * time.delta().as_secs_f32();
            }
            for mut bu in &mut p.p1() {
                if bu.translation.y > 10.55 {
                    bu.translation.y -= 4.0 * time.delta().as_secs_f32();
                } else {
                    bu.translation.y += 4.0 * time.delta().as_secs_f32();
                }
            }
        },
    );
    run_scenario(&mut app, &scenario(r#","collision_policy":"off""#)).unwrap();
    assert_eq!(
        pass_through_count(&app),
        0,
        "collision_policy off must silence the rule"
    );
}

#[test]
fn pass_through_silent_on_despawn_mid_overlap() {
    // Traveller overlaps the obstacle, then the obstacle despawns
    // (pickup/projectile pattern) — the pair must not count.
    let mut app = build_app();
    app.world_mut()
        .spawn((Name::new("Pickup"), bodies(), DespawnTimer(0.3)))
        .insert(Transform::from_xyz(0.0, 0.0, 0.0));
    app.world_mut()
        .spawn((Name::new("Traveller"), bodies(), Traveller))
        .insert(Transform::from_xyz(-2.0, 0.0, 0.0));
    app.world_mut()
        .spawn((Name::new("Anchor"), bodies()))
        .insert(Transform::from_xyz(10.0, 10.0, 0.0));
    app.world_mut()
        .spawn((Name::new("Bouncer"), bodies(), Bouncer))
        .insert(Transform::from_xyz(10.0, 12.0, 0.0));
    app.add_systems(
        Update,
        (
            |time: Res<Time>, mut q: Query<&mut Transform, With<Traveller>>| {
                for mut tr in &mut q {
                    tr.translation.x += 4.0 * time.delta().as_secs_f32();
                }
            },
            |time: Res<Time>, mut cmds: Commands, q: Query<(Entity, &DespawnTimer)>| {
                for (e, t) in &q {
                    if time.elapsed_secs() > t.0 {
                        cmds.entity(e).despawn();
                    }
                }
            },
            |time: Res<Time>, mut b: Query<&mut Transform, With<Bouncer>>| {
                for mut bu in &mut b {
                    if bu.translation.y > 10.55 {
                        bu.translation.y -= 4.0 * time.delta().as_secs_f32();
                    } else {
                        bu.translation.y += 4.0 * time.delta().as_secs_f32();
                    }
                }
            },
        ),
    );
    run_scenario(&mut app, &scenario("")).unwrap();
    assert_eq!(
        pass_through_count(&app),
        0,
        "despawn mid-overlap must be excluded"
    );
}

#[derive(Component)]
struct DespawnTimer(f32);
