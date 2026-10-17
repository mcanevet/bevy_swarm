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
