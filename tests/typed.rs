//! I3: Typed Rust API — oracles, goal predicates, bot policies as systems.

use bevy::prelude::*;
use bevy_swarm::contract::{TestApi, UserIntent};
use bevy_swarm::driver::{run_scenario, PlaytestPlugin};
use bevy_swarm::scenario::Scenario;
use bevy_swarm::typed::PlaytestAppExt;

/// Simple game: Choice{n} awards n points (up to 3 per press once).
#[derive(Resource)]
struct Score(pub i64);

fn build_game_app() -> App {
    let mut app = App::new();
    app.add_plugins((
        bevy::MinimalPlugins,
        bevy::transform::TransformPlugin,
        PlaytestPlugin,
        bevy_swarm::contract::TestConventionsPlugin,
    ));
    app.insert_resource(Score(0));
    app.insert_resource(TestApi::default());
    app.add_message::<UserIntent>();
    app.add_systems(
        Update,
        (
            |mut msgs: MessageReader<UserIntent>, mut score: ResMut<Score>| {
                for m in msgs.read() {
                    if let UserIntent::Choice { index } = m {
                        score.0 += (*index as i64).max(0);
                    }
                }
            },
            |score: Res<Score>, mut api: ResMut<TestApi>| {
                api.score = score.0;
            },
        ),
    );
    app
}

fn scenario(json: &str) -> Scenario {
    serde_json::from_str(json).unwrap()
}

#[test]
fn typed_oracle_detects_violation() {
    // Oracle fires when Score exceeds 3 (points awarded bug: presses
    // past the cap). Here Choice{2} pressed twice → 4 > 3.
    let mut app = build_game_app();
    app.add_oracle("score_capped", |score: Res<Score>| {
        if score.0 > 3 {
            Err(format!("score {} exceeds cap 3", score.0))
        } else {
            Ok(())
        }
    });
    let sc = scenario(
        r#"{"bot":{"type":"replay","inputs":[{"frame":1,"intent":{"intent":"choice","index":2}},{"frame":10,"intent":{"intent":"choice","index":2}}]},"duration_s":0.5,"oracles":["score_capped"]}"#,
    );
    let rep = run_scenario(&mut app, &sc).unwrap();
    assert!(
        rep.violations.iter().any(|v| v.rule == "score_capped"),
        "typed oracle must fire: {:?}",
        rep.violations
    );
}

#[test]
fn typed_oracle_passes_when_ok() {
    let mut app = build_game_app();
    app.add_oracle("score_capped", |score: Res<Score>| {
        if score.0 > 3 {
            Err(format!("score {} exceeds cap 3", score.0))
        } else {
            Ok(())
        }
    });
    let sc = scenario(
        r#"{"bot":{"type":"replay","inputs":[{"frame":1,"intent":{"intent":"choice","index":1}}]},"duration_s":0.3,"oracles":["score_capped"]}"#,
    );
    let rep = run_scenario(&mut app, &sc).unwrap();
    assert!(
        !rep.violations.iter().any(|v| v.rule == "score_capped"),
        "no violation expected: {:?}",
        rep.violations
    );
}

#[test]
fn typed_oracle_unknown_name_rejected() {
    let mut app = build_game_app();
    app.add_oracle("known", |_: Res<Score>| Ok(()));
    let sc = scenario(
        r#"{"bot":{"type":"replay","inputs":[]},"duration_s":0.1,"oracles":["nonexistent"]}"#,
    );
    let err = run_scenario(&mut app, &sc).unwrap_err();
    match err {
        bevy_swarm::scenario::ScenarioError::Rejected(msg) => {
            assert!(msg.contains("nonexistent"), "{}", msg);
        }
        other => panic!("expected Rejected, got {:?}", other),
    }
}

#[test]
fn all_oracles_run_by_default() {
    // No `oracles` field: every registered oracle runs.
    let mut app = build_game_app();
    app.add_oracle(
        "always_fires",
        |_: Res<Score>| Err("deliberate".to_string()),
    );
    app.add_oracle("never_fires", |_: Res<Score>| Ok(()));
    let sc = scenario(r#"{"bot":{"type":"replay","inputs":[]},"duration_s":0.2}"#);
    let rep = run_scenario(&mut app, &sc).unwrap();
    assert!(rep.violations.iter().any(|v| v.rule == "always_fires"));
    assert!(!rep.violations.iter().any(|v| v.rule == "never_fires"));
}

#[test]
fn planner_predicate_goal_achieved() {
    // Goal tree uses a typed predicate instead of a TestApi path.
    let mut app = build_game_app();
    app.add_goal_predicate("score_at_least_two", |score: Res<Score>| score.0 >= 2);
    let sc = scenario(
        r#"{"bot":{"type":"planner","input_rate_hz":30,"goals":{
            "kind":"predicate","name":"score_at_least_two",
            "emit":[{"intent":"choice","index":2}]
        }},"duration_s":1.0}"#,
    );
    let rep = run_scenario(&mut app, &sc).unwrap();
    // Goal achieved → no planner_goal_unfinished violation.
    assert!(
        !rep.violations
            .iter()
            .any(|v| v.rule == "planner_goal_unfinished"),
        "predicate goal should be achieved: {:?}",
        rep.violations
    );
    assert!(
        !rep.violations
            .iter()
            .any(|v| v.rule == "planner_any_exhausted"),
        "goal stack should drain: {:?}",
        rep.violations
    );
}

#[test]
fn custom_bot_policy_drives_game() {
    // Policy emits Choice{2} until score >= 2 (smarter than replay).
    let mut app = build_game_app();
    app.add_bot_policy("until_two", |score: Res<Score>| {
        if score.0 < 2 {
            Some(UserIntent::Choice { index: 2 })
        } else {
            None
        }
    });
    let sc = scenario(
        r#"{"bot":{"type":"custom","policy":"until_two","input_rate_hz":30},"duration_s":1.0,"invariants":[
            {"name":"reached","rule":"custom","path":"TestApi.score","check":"ge","value":2,"eventually_s":0.5}
        ]}"#,
    );
    let rep = run_scenario(&mut app, &sc).unwrap();
    assert!(
        !rep.violations.iter().any(|v| v.rule == "reached"),
        "custom bot must drive score to 2: {:?}",
        rep.violations
    );
    assert!(
        !rep.violations.iter().any(|v| v.rule == "custom_bot_config"),
        "policy must be configured correctly: {:?}",
        rep.violations
    );
}

#[test]
fn custom_bot_unknown_policy_rejected() {
    let mut app = build_game_app();
    let sc = scenario(r#"{"bot":{"type":"custom","policy":"missing_policy"},"duration_s":0.1}"#);
    let err = run_scenario(&mut app, &sc).unwrap_err();
    match err {
        bevy_swarm::scenario::ScenarioError::Rejected(msg) => {
            assert!(msg.contains("missing_policy"), "{}", msg);
        }
        other => panic!("expected Rejected, got {:?}", other),
    }
}

#[test]
fn minimize_typed_oracle_failure() {
    use bevy_swarm::enums::PlaytestStatus;
    use bevy_swarm::harness::{minimize_failure, FailureSignature};
    // B2 minimizes a failure detected by a TYPED oracle: the oracle fires
    // when score > 3; two Choice{2} presses (frames 0 and 30) cause it.
    let sc = scenario(
        r#"{"bot":{"type":"replay","inputs":[
            {"frame":1,"intent":{"intent":"choice","index":2}},
            {"frame":30,"intent":{"intent":"choice","index":2}}
        ]},"duration_s":1.0,"oracles":["score_capped"]}"#,
    );
    let mut app = build_game_app();
    app.add_oracle("score_capped", |score: Res<Score>| {
        if score.0 > 3 {
            Err(format!("score {} exceeds cap 3", score.0))
        } else {
            Ok(())
        }
    });
    let report = run_scenario(&mut app, &sc).unwrap();
    assert!(
        report.violations.iter().any(|v| v.rule == "score_capped"),
        "setup: oracle must fire on the full scenario"
    );
    let builder = || {
        let mut app = build_game_app();
        app.add_oracle("score_capped", |score: Res<Score>| {
            if score.0 > 3 {
                Err(format!("score {} exceeds cap 3", score.0))
            } else {
                Ok(())
            }
        });
        app
    };
    let outcome = minimize_failure(
        builder,
        &sc,
        &report,
        Some(FailureSignature::Violation {
            rule: "score_capped".into(),
            target: None,
        }),
    )
    .unwrap();
    // Sanity: the minimized regression scenario still reproduces the
    // typed-oracle violation.
    let mut app = builder();
    let rep = run_scenario(&mut app, &outcome.regression_scenario).unwrap();
    assert!(
        rep.violations.iter().any(|v| v.rule == "score_capped"),
        "minimized scenario must still violate: {:?}",
        rep.violations
    );
    assert_eq!(rep.status, PlaytestStatus::Fail);
}
