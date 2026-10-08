//! J0: ChoiceStream — record/replay of bot random choices.

use bevy::prelude::*;
use bevy_swarm::contract::{IntentSurface, SurfaceVariant, TestApi, UserIntent};
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
    app.insert_resource(IntentSurface::new(vec![
        SurfaceVariant::Choice(3),
        SurfaceVariant::Move,
        SurfaceVariant::Wait,
    ]));
    app.add_message::<UserIntent>();
    app
}

fn chaos_scenario(seed: u64, choices: Option<serde_json::Value>) -> Scenario {
    let choices_json = match &choices {
        Some(v) => v.to_string(),
        None => "null".into(),
    };
    serde_json::from_str(&format!(
        r#"{{"bot":{{"type":"chaos","seed":{seed},"input_rate_hz":30,"choices":{choices_json}}},"duration_s":0.8,"invariants":[]}}"#
    ))
    .unwrap()
}

#[test]
fn chaos_run_records_choices() {
    let mut app = build_app();
    let rep = run_scenario(&mut app, &chaos_scenario(9, None)).unwrap();
    assert!(!rep.choices.is_empty(), "chaos must record choices");
    for c in &rep.choices {
        assert!(c.value < c.bound.max(1), "reduced value in range: {c:?}");
    }
}

#[test]
fn choices_replay_reproduces_run() {
    // Run A: chaos seed 9. Run B: bot.choices = A's recorded choices
    // (any seed) → identical action_log.
    let mut app = build_app();
    let rep_a = run_scenario(&mut app, &chaos_scenario(9, None)).unwrap();
    let choices_json = serde_json::to_value(&rep_a.choices).unwrap();

    let mut app = build_app();
    let rep_b = run_scenario(
        &mut app,
        &chaos_scenario(
            12345, // completely different seed — replay must dominate
            Some(choices_json),
        ),
    )
    .unwrap();

    let summarize = |rep: &bevy_swarm::driver::PlaytestReport| {
        rep.action_log
            .iter()
            .map(|e| format!("{}:{:?}", e.frame, e.details))
            .collect::<Vec<_>>()
    };
    let (a, b) = (summarize(&rep_a), summarize(&rep_b));
    assert_eq!(
        a, b,
        "replaying recorded choices must reproduce the audit log"
    );
}

#[test]
fn out_of_range_choice_clamps_no_panic() {
    // Replayed choice 999 with bound 3 must clamp, not panic.
    let scenario: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"chaos","seed":1,"input_rate_hz":30,
             "choices":[{"frame":0,"value":999,"bound":3},{"frame":0,"value":18446744073709551615,"bound":5}],
             "continuation":"zeros"},"duration_s":0.3,"invariants":[]}"#,
    )
    .unwrap();
    let mut app = build_app();
    let rep = run_scenario(&mut app, &scenario).unwrap();
    assert!(!rep.choices.is_empty());
    for c in &rep.choices {
        assert!(c.value < c.bound.max(1));
    }
}

#[test]
fn zero_choices_are_simplest() {
    // All-zero replay + zeros continuation: roll lands on variant 0
    // of the surface and Move dirs are the canonical lo (-1.0).
    let scenario: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"chaos","seed":1,"input_rate_hz":30,
             "choices":[],"continuation":"zeros"},"duration_s":0.3,"invariants":[]}"#,
    )
    .unwrap();
    let mut app = build_app();
    let rep = run_scenario(&mut app, &scenario).unwrap();
    for c in &rep.choices {
        assert_eq!(c.value, 0, "zeros continuation draws zero");
    }
}
