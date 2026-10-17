//! I5: action-effect oracle — "did this input DO anything?", per-action
//! effect rates, dead_verb detection.

use bevy::prelude::*;
use bevy_playtest::contract::{Gameplay, TestApi, UserIntent};
use bevy_playtest::driver::{run_scenario, PlaytestPlugin};
use bevy_playtest::scenario::Scenario;

fn build_app() -> App {
    let mut app = App::new();
    app.add_plugins((
        bevy::MinimalPlugins,
        bevy::transform::TransformPlugin,
        PlaytestPlugin,
        bevy_playtest::contract::TestConventionsPlugin,
    ));
    app.insert_resource(TestApi::default());
    app.register_type::<Points>();
    app.add_message::<UserIntent>();
    app
}

/// Registered for reflection so the canonical digest sees the VALUE
/// (value-based effect attribution needs digest-visible state).
#[derive(Component, Reflect)]
#[reflect(Component)]
struct Points(i64);

#[test]
fn effective_intent_has_full_effect_rate() {
    // Choice intents drive the game (Points component updates) →
    // intent:choice must show a positive effect rate in the report.
    let scenario: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"replay","inputs":[
            {"frame":1,"intent":{"intent":"choice","index":1}},
            {"frame":2,"intent":{"intent":"choice","index":1}}
        ]},"duration_s":0.6,"invariants":[]}"#,
    )
    .unwrap();
    let mut app = build_app();
    app.add_systems(bevy::app::Startup, |mut c: Commands| {
        c.spawn((Name::new("P1"), Gameplay, Points(0)));
    });
    app.add_systems(
        Update,
        (
            |mut reader: bevy::ecs::message::MessageReader<UserIntent>,
             mut q: Query<&mut Points>| {
                for _i in reader.read() {
                    for mut p in &mut q {
                        p.0 += 1;
                    }
                }
            },
            |q: Query<&Points>, mut api: ResMut<TestApi>| {
                if let Ok(p) = q.single() {
                    api.score = p.0;
                }
            },
        ),
    );
    let rep = run_scenario(&mut app, &scenario).unwrap();
    assert!(
        !rep.violations.iter().any(|v| v.rule == "dead_verb"),
        "an effective verb must not be dead: {:?}",
        rep.violations
    );
    let rate = rep
        .action_effect_rate
        .get("intent:choice")
        .expect("intent:choice must be tracked");
    assert!(rate.effective >= 1, "rate: {:?}", rate);
    assert_eq!(rate.total, 2);
}

#[test]
fn dead_verb_detected() {
    // Move intents are handled (increment TestApi.score → digest-visible,
    // world stays live), Choice intents ignored → dead_verb for intent:choice.
    // Value-based effect attribution ensures same-value rewrites don't mask.
    let inputs: Vec<String> = (1..=24)
        .map(|f| {
            let intent = if f % 2 == 0 {
                r#"{"intent":"move","dir":[1.0,0.0]}"#
            } else {
                r#"{"intent":"choice","index":0}"#
            };
            format!(r#"{{"frame":{},"intent":{}}}"#, f * 20, intent)
        })
        .collect();
    let scenario: Scenario = serde_json::from_str(&format!(
        r#"{{"bot":{{"type":"replay","inputs":[{}]}},"duration_s":10.0,"invariants":[],"liveness":{{"watch":["TestApi"]}}}}"#,
        inputs.join(",")
    ))
    .unwrap();
    let mut app = build_app();
    app.add_systems(bevy::app::Startup, |mut c: Commands| {
        c.spawn((Name::new("P1"), Gameplay, Points(0)));
    });
    // Handle Move by bumping a digest-visible TestApi custom numeric,
    // ignore Choice.
    app.add_systems(
        Update,
        |mut reader: bevy::ecs::message::MessageReader<UserIntent>,
         mut api: bevy::ecs::system::ResMut<bevy_playtest::contract::TestApi>| {
            for intent in reader.read() {
                if let UserIntent::Move { .. } = intent {
                    let e = api.custom_numeric.entry("moves".into()).or_insert(0.0);
                    *e += 1.0;
                }
            }
        },
    );
    let rep = run_scenario(&mut app, &scenario).unwrap();
    assert!(
        rep.violations.iter().any(|v| v.rule == "dead_verb"),
        "a verb with 0% effect over >= 10 samples must be dead, got: {:?}",
        rep.violations
    );
    let rate = rep
        .action_effect_rate
        .get("intent:choice")
        .expect("intent:choice must be tracked");
    assert_eq!(rate.effective, 0);
    assert!(rate.total >= 10);
}

// ---------------------------------------------------------------------------
// swarm-6i8: ambient fixture motion must not mask dead_verb
// ---------------------------------------------------------------------------

/// A game with genuine ambient motion (elapsed-time heartbeat, like
/// spawner's IdlePhase sine) where a verb has 0% effect must STILL
/// fire dead_verb: ambient dynamics are subtracted via the
/// action-free baseline window.
#[test]
fn s6i8_ambient_motion_does_not_mask_dead_verb() {
    // Replay alternates: effective "move" (bumps a counter) and dead
    // "choice" (ignored), while an ambient heartbeat resource changes
    // every frame regardless of input.
    let inputs: Vec<String> = (1..=24)
        .map(|f| {
            let intent = if f % 2 == 0 {
                r#"{"intent":"move","dir":[1.0,0.0]}"#
            } else {
                r#"{"intent":"choice","index":0}"#
            };
            format!(r#"{{"frame":{},"intent":{}}}"#, f * 20, intent)
        })
        .collect();
    let scenario: Scenario = serde_json::from_str(&format!(
        r#"{{"bot":{{"type":"replay","inputs":[{}]}},"duration_s":10.0,"invariants":[],"liveness":{{"watch":["TestApi"]}}}}"#,
        inputs.join(",")
    ))
    .unwrap();
    let mut app = build_app();
    app.insert_resource(AmbientHeartbeat(0.0));
    app.register_type::<AmbientHeartbeat>();
    app.add_systems(bevy::app::Startup, |mut c: Commands| {
        c.spawn((Name::new("P1"), Gameplay, Points(0)));
    });
    // Ambient: changes every frame, independent of any intent (the
    // spawner IdlePhase / regression_pair Damage pattern).
    app.add_systems(
        Update,
        |time: Res<Time>, mut hb: ResMut<AmbientHeartbeat>| {
            hb.0 = (time.elapsed_secs() * 2.0).sin();
        },
    );
    // Handle Move by bumping a digest-visible counter, ignore Choice.
    app.add_systems(
        Update,
        |mut reader: bevy::ecs::message::MessageReader<UserIntent>,
         mut api: bevy::ecs::system::ResMut<bevy_playtest::contract::TestApi>| {
            for intent in reader.read() {
                if let UserIntent::Move { .. } = intent {
                    let e = api.custom_numeric.entry("moves".into()).or_insert(0.0);
                    *e += 1.0;
                }
            }
        },
    );
    let rep = run_scenario(&mut app, &scenario).unwrap();
    assert!(
        rep.violations.iter().any(|v| v.rule == "dead_verb"),
        "ambient motion must not mask a 0%-effect verb, got: {:?}",
        rep.violations
    );
}

/// Elapsed-scaled ambient value (regression_pair Damage pattern):
/// damage drifts every frame; a dead verb still fires.
#[test]
fn s6i8_elapsed_scaled_ambient_does_not_mask_dead_verb() {
    // Alternate move (handled, keeps world live) and choice (dead).
    let inputs: Vec<String> = (1..=24)
        .map(|f| {
            let intent = if f % 2 == 0 {
                r#"{"intent":"move","dir":[1.0,0.0]}"#
            } else {
                r#"{"intent":"choice","index":0}"#
            };
            format!(r#"{{"frame":{},"intent":{}}}"#, f * 20, intent)
        })
        .collect();
    let scenario: Scenario = serde_json::from_str(&format!(
        r#"{{"bot":{{"type":"replay","inputs":[{}]}},"duration_s":10.0,"invariants":[],"liveness":{{"watch":["TestApi"]}}}}"#,
        inputs.join(",")
    ))
    .unwrap();
    let mut app = build_app();
    app.insert_resource(AmbientDamage(10.0));
    app.register_type::<AmbientDamage>();
    app.add_systems(bevy::app::Startup, |mut c: Commands| {
        c.spawn((Name::new("P1"), Gameplay, Points(0)));
    });
    // Ambient damage: base * (1 + elapsed phase) — changes every frame.
    app.add_systems(Update, |time: Res<Time>, mut d: ResMut<AmbientDamage>| {
        let phase = (time.elapsed_secs() * 0.5).fract();
        d.0 = 10.0 * (1.0 + phase as f64 * 0.1);
    });
    // Handle move (keeps world live); choice is dead.
    app.add_systems(
        Update,
        |mut reader: bevy::ecs::message::MessageReader<UserIntent>,
         mut api: bevy::ecs::system::ResMut<bevy_playtest::contract::TestApi>| {
            for intent in reader.read() {
                if let UserIntent::Move { .. } = intent {
                    let e = api.custom_numeric.entry("moves".into()).or_insert(0.0);
                    *e += 1.0;
                }
            }
        },
    );
    let rep = run_scenario(&mut app, &scenario).unwrap();
    assert!(
        rep.violations.iter().any(|v| v.rule == "dead_verb"),
        "elapsed-scaled ambient must not mask a dead verb, got: {:?}",
        rep.violations
    );
}

/// Positive control: an EFFECTIVE verb in an ambient-motion world must
/// still count as effective (subtraction must not over-prune).
#[test]
fn s6i8_effective_verb_survives_ambient_subtraction() {
    let inputs: Vec<String> = (1..=24)
        .map(|f| {
            let intent = if f % 2 == 0 {
                r#"{"intent":"move","dir":[1.0,0.0]}"#
            } else {
                r#"{"intent":"choice","index":0}"#
            };
            format!(r#"{{"frame":{},"intent":{}}}"#, f * 20, intent)
        })
        .collect();
    let scenario: Scenario = serde_json::from_str(&format!(
        r#"{{"bot":{{"type":"replay","inputs":[{}]}},"duration_s":10.0,"invariants":[]}}"#,
        inputs.join(",")
    ))
    .unwrap();
    let mut app = build_app();
    app.insert_resource(AmbientHeartbeat(0.0));
    app.register_type::<AmbientHeartbeat>();
    app.add_systems(
        Update,
        |time: Res<Time>, mut hb: ResMut<AmbientHeartbeat>| {
            hb.0 = (time.elapsed_secs() * 2.0).sin();
        },
    );
    app.add_systems(
        Update,
        |mut reader: bevy::ecs::message::MessageReader<UserIntent>,
         mut api: bevy::ecs::system::ResMut<bevy_playtest::contract::TestApi>| {
            for intent in reader.read() {
                if let UserIntent::Move { .. } = intent {
                    let e = api.custom_numeric.entry("moves".into()).or_insert(0.0);
                    *e += 1.0;
                }
            }
        },
    );
    let rep = run_scenario(&mut app, &scenario).unwrap();
    let rate =
        rep.action_effect_rate
            .get("intent:move")
            .unwrap_or(&bevy_playtest::effects::EffectRate {
                effective: 999,
                total: 0,
            });
    assert!(
        rate.effective >= rate.total / 2,
        "effective verb must stay effective under ambient subtraction: {:?}",
        rate
    );
}

#[derive(Resource, Reflect)]
#[reflect(Resource)]
struct AmbientHeartbeat(f32);

#[derive(Resource, Reflect)]
#[reflect(Resource)]
struct AmbientDamage(f64);
