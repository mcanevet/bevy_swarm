use crate::harness::*;

#[allow(unused_imports)]
use super::fixtures::*;

#[test]
fn duplicate_invariant_names_rejected() {
    let scen: Scenario = serde_json::from_str(
        r#"{
        "bot": {"type": "chaos", "seed": 1},
        "duration_s": 0.2,
        "invariants": [
            {"name": "dup", "rule": "custom", "path": "TestApi.score"},
            {"name": "dup", "rule": "custom", "path": "TestApi.score"}
        ],
        "setup": {}
    }"#,
    )
    .unwrap();
    let err = validate_scenario(&scen).unwrap_err();
    assert!(
        matches!(err, crate::scenario::ScenarioError::Rejected(ref m) if m.contains("duplicate invariant name")),
        "got: {:?}",
        err
    );

    // Same scenario with unique names validates fine.
    let scen: Scenario = serde_json::from_str(
        r#"{
        "bot": {"type": "chaos", "seed": 1},
        "duration_s": 0.2,
        "invariants": [
            {"name": "a", "rule": "custom", "path": "TestApi.score"},
            {"name": "b", "rule": "custom", "path": "TestApi.score"}
        ],
        "setup": {}
    }"#,
    )
    .unwrap();
    assert!(validate_scenario(&scen).is_ok());
}

#[test]
fn c6_load_time_validation_rejections() {
    fn rejects(json: &str, needle: &str) {
        let scen: Scenario = serde_json::from_str(json).unwrap();
        let err = validate_scenario(&scen).unwrap_err();
        assert!(
            matches!(err, crate::scenario::ScenarioError::Rejected(ref m) if m.contains(needle)),
            "expected rejection containing {:?}, got: {:?}",
            needle,
            err
        );
    }
    // duration_s not finite or <= 0
    rejects(
        r#"{"bot":{"type":"chaos"},"duration_s":0,"invariants":[],"setup":{}}"#,
        "duration_s",
    );
    // tps >= 1
    rejects(
        r#"{"bot":{"type":"chaos"},"duration_s":0.2,"tps":0,"invariants":[],"setup":{}}"#,
        "tps",
    );
    // planner without goals
    rejects(
        r#"{"bot":{"type":"planner"},"duration_s":0.2,"invariants":[],"setup":{}}"#,
        "goals",
    );
    // pursuit without agent_target/target
    rejects(
        r#"{"bot":{"type":"pursuit"},"duration_s":0.2,"invariants":[],"setup":{}}"#,
        "agent_target",
    );
    // synthetic_pointer with empty pointer_clicks
    rejects(
        r#"{"bot":{"type":"synthetic_pointer"},"duration_s":0.2,"invariants":[],"setup":{}}"#,
        "pointer_clicks",
    );
    // synthetic_keyboard with empty key_presses
    rejects(
        r#"{"bot":{"type":"synthetic_keyboard"},"duration_s":0.2,"invariants":[],"setup":{}}"#,
        "key_presses",
    );
    // unknown key names at load time
    rejects(
        r#"{"bot":{"type":"synthetic_keyboard","key_presses":[{"frame":1,"key":"notakey"}]},"duration_s":0.2,"invariants":[],"setup":{}}"#,
        "unknown key name",
    );
    // eventually_s > 0
    rejects(
        r#"{"bot":{"type":"chaos"},"duration_s":0.2,"invariants":[{"name":"e","rule":"custom","path":"TestApi.score","eventually_s":0}],"setup":{}}"#,
        "eventually_s",
    );
    // after_s < before_s
    rejects(
        r#"{"bot":{"type":"chaos"},"duration_s":0.2,"invariants":[{"name":"e","rule":"custom","path":"TestApi.score","after_s":2.0,"before_s":1.0}],"setup":{}}"#,
        "after_s",
    );
    // bounds min <= max
    rejects(
        r#"{"bot":{"type":"chaos"},"duration_s":0.2,"invariants":[{"name":"b","rule":"nodes_in_bounds","min_x":5.0,"max_x":1.0}],"setup":{}}"#,
        "min_x",
    );
}

#[test]
fn c6_replay_empty_inputs_warns_not_rejects() {
    let scen: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"replay"},"duration_s":0.2,"invariants":[],"setup":{}}"#,
    )
    .unwrap();
    assert!(validate_scenario(&scen).is_ok());
    let mut app = build_app_no_scoring();
    let rep = run_scenario(&mut app, &scen).unwrap();
    assert!(
        rep.warnings.iter().any(|w| w.contains("empty inputs")),
        "warnings: {:?}",
        rep.warnings
    );
}

#[test]
fn c6_pointer_button_parses_as_enum() {
    let scen: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"synthetic_pointer","pointer_clicks":[{"frame":1,"target":"Ball","button":"secondary"}]},"duration_s":0.2,"invariants":[],"setup":{}}"#,
    )
    .unwrap();
    assert_eq!(
        scen.bot.pointer_clicks[0].button,
        crate::enums::PointerButton::Secondary
    );
    assert!(validate_scenario(&scen).is_ok());
}

#[test]
fn missing_contract_is_structured_error() {
    // Missing harness plugin / contract pieces produce a structured
    // ContractMissing error, never a panic or a vacuous pass.
    // A bare app without PlaytestPlugin cannot run a scenario at all.
    let mut app = bevy::app::App::new();
    let scen: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"chaos","seed":1},"duration_s":0.1,"invariants":[{"name":"c","rule":"custom","path":"TestApi.score","check":"above","value":1}],"setup":{"resets":[{"kind":"anything"}]}}"#,
    )
    .unwrap();
    let err = run_scenario(&mut app, &scen).unwrap_err();
    assert!(matches!(
        err,
        crate::scenario::ScenarioError::ContractMissing(_)
    ));

    // A TestApi.* path without any registered resolver is also a
    // structured load error (D1).
    let mut app2 = bevy::app::App::new();
    app2.add_plugins((bevy::MinimalPlugins, crate::harness::PlaytestPlugin));
    let scen2: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"chaos","seed":1},"duration_s":0.1,"invariants":[{"name":"c","rule":"custom","path":"TestApi.score","check":"above","value":1}],"setup":{}}"#,
    )
    .unwrap();
    let err2 = run_scenario(&mut app2, &scen2).unwrap_err();
    assert!(
        matches!(err2, crate::scenario::ScenarioError::ContractMissing(_)),
        "got {:?}",
        err2
    );
}

#[test]
fn unknown_reset_kind_is_structured_error() {
    use crate::contract::ResetHooks;
    use crate::contract::TestApi;
    let mut app = bevy::app::App::new();
    app.insert_resource(ResetHooks::default());
    app.insert_resource(TestApi::default());
    let scen: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"chaos","seed":1},"duration_s":0.1,"invariants":[],
            "setup":{"resets":[{"kind":"never_registered"}]}}"#,
    )
    .unwrap();
    let err = run_scenario(&mut app, &scen).unwrap_err();
    match err {
        crate::scenario::ScenarioError::UnknownReset { kind, known } => {
            assert_eq!(kind, "never_registered");
            assert!(known.is_empty());
        }
        other => panic!("expected UnknownReset, got {:?}", other),
    }
}
