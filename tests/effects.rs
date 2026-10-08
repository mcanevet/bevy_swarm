//! I5: action-effect oracle — "did this input DO anything?", per-action
//! effect rates, dead_verb detection.

use bevy::prelude::*;
use bevy_swarm::contract::{Gameplay, TestApi, UserIntent};
use bevy_swarm::driver::{run_scenario, PlaytestPlugin};
use bevy_swarm::scenario::Scenario;

fn build_app() -> App {
    let mut app = App::new();
    app.add_plugins((
        bevy::MinimalPlugins,
        bevy::transform::TransformPlugin,
        PlaytestPlugin,
        bevy_swarm::contract::TestConventionsPlugin,
    ));
    app.insert_resource(TestApi::default());
    app.add_message::<UserIntent>();
    app
}

#[derive(Component)]
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
    // 10+ Wait-or-dead choice intents that the game ignores entirely
    // (no state change anywhere) → dead_verb for that key.
    let inputs: Vec<String> = (1..=12)
        .map(|f| format!(r#"{{"frame":{},"intent":{{"intent":"wait"}}}}"#, f * 4))
        .collect();
    let scenario: Scenario = serde_json::from_str(&format!(
        r#"{{"bot":{{"type":"replay","inputs":[{}]}},"duration_s":2.0,"invariants":[]}}"#,
        inputs.join(",")
    ))
    .unwrap();
    let mut app = build_app();
    app.add_systems(bevy::app::Startup, |mut c: Commands| {
        c.spawn((Name::new("P1"), Gameplay, Points(0)));
    });
    // NO handler: the game ignores Wait entirely. But something must
    // keep the world alive elsewhere or liveness fires — Points static
    // is fine here; liveness may complain but the test only cares
    // about dead_verb.
    let rep = run_scenario(&mut app, &scenario).unwrap();
    assert!(
        rep.violations.iter().any(|v| v.rule == "dead_verb"),
        "a verb with 0% effect over >= 10 samples must be dead, got: {:?}",
        rep.violations
    );
    let rate = rep
        .action_effect_rate
        .get("intent:wait")
        .expect("intent:wait must be tracked");
    assert_eq!(rate.effective, 0);
    assert!(rate.total >= 10);
}
