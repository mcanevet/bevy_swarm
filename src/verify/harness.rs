use crate::contract::*;
use crate::enums::PlaytestStatus;
use crate::harness::*;
use bevy::prelude::*;

#[allow(unused_imports)]
use super::fixtures::*;

#[test]
fn test_api_resolves_numeric_fields() {
    let api = TestApi {
        score: 42,
        active_players: 3,
        ..Default::default()
    };
    assert_eq!(
        api.resolve("TestApi.score"),
        Some(TestFieldValue::Numeric(42.0))
    );
    assert_eq!(
        api.resolve("TestApi.active_players"),
        Some(TestFieldValue::Numeric(3.0))
    );
    assert_eq!(api.resolve("TestApi.nonexistent"), None);
}

#[test]
fn branch_matrix_runs_all_variants_and_passes() {
    let scenarios: Vec<(String, Scenario)> = vec![
        (
            "chaos-a".into(),
            serde_json::from_str(
                r#"{"bot":{"type":"chaos","seed":1},"duration_s":0.5,"invariants":[],"setup":{}}"#,
            )
            .unwrap(),
        ),
        (
            "chaos-b".into(),
            serde_json::from_str(
                r#"{"bot":{"type":"chaos","seed":2},"duration_s":0.5,"invariants":[],"setup":{}}"#,
            )
            .unwrap(),
        ),
    ];
    let matrix = run_branch_matrix(build_app, scenarios).expect("matrix runs");
    assert_eq!(matrix.outcomes.len(), 2);
    assert!(
        matrix.all_passed(),
        "failed: {:?}",
        matrix.failed_variants()
    );
}

#[test]
fn coverage_records_intents_emitted() {
    let scen: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"chaos","seed":1},"duration_s":0.5,"invariants":[],"setup":{}}"#,
    )
    .unwrap();
    let mut app = build_app();
    let rep = run_scenario(&mut app, &scen).unwrap();
    assert!(!rep.coverage.intents_emitted.is_empty(), "coverage empty!");
}

#[test]
fn violations_carry_intent_context() {
    let bad: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"chaos","seed":7},"duration_s":0.2,
            "invariants":[{"name":"phase_lock","rule":"custom","path":"TestApi.game_phase","check":"equals","value":"GameOver"}],
            "setup":{}}"#,
    )
    .unwrap();
    let mut app = build_app();
    let rep = run_scenario(&mut app, &bad).unwrap();
    assert_eq!(rep.status, PlaytestStatus::Fail);
    let ctx = &rep.violations[0].detail;
    assert!(ctx.starts_with("[intent: "), "missing context: {}", ctx);
}

#[test]
fn frame_timing_stats_flag_only_true_anomalies() {
    let mut fts = FrameTimingStats::default();
    for _ in 0..200 {
        fts.observe(1.0);
    }
    assert!(!fts.is_anomalous(1.2), "small jitter must not be anomalous");
    assert!(fts.is_anomalous(50.0), "50x spike must be anomalous");
}

#[test]
fn differential_invariants_are_accepted() {
    let diff_scen: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"chaos","seed":9},"duration_s":0.1,"invariants":[
            {"name":"score_monotonic","rule":"custom","path":"TestApi.score","differential":"no_decrease"}
        ],"setup":{}}"#,
    )
    .unwrap();
    let mut app = build_app();
    let rep = run_scenario(&mut app, &diff_scen).unwrap();
    let _ = rep;
}

#[test]
fn replay_bot_emits_frame_indexed_intents() {
    let scen: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"replay","inputs":[
            {"frame":1,"intent":{"intent":"choice","index":0}},
            {"frame":2,"intent":{"intent":"choice","index":0}},
            {"frame":3,"intent":{"intent":"choice","index":0}},
            {"frame":4,"intent":{"intent":"choice","index":0}}
        ]},"duration_s":0.2,"invariants":[
            {"name":"score_grew","rule":"custom","path":"TestApi.score","check":"above","value":5,"after_s":0.1}
        ],"setup":{}}"#,
    )
    .unwrap();
    let mut app = build_app();
    let rep = run_scenario(&mut app, &scen).unwrap();
    assert!(
        rep.coverage.intents_emitted.contains_key("choice"),
        "replay emitted no choice intents"
    );
    assert_eq!(
        rep.status,
        PlaytestStatus::Pass,
        "unexpected violations: {:?}",
        rep.violations
    );
}

#[test]
fn expert_rule_passes_on_compliant_policy() {
    let scen: Scenario = serde_json::from_str(
         r#"{"bot":{"type":"chaos","seed":11},"duration_s":0.1,"invariants":[
            {"name":"heal_policy","rule":"custom","path":"TestApi.active_players","check":"below","value":5,
             "requires_path":"TestApi.active_players","requires_check":"above","requires_value":0}
        ],"setup":{}}"#,
    )
    .unwrap();
    let mut app = build_app();
    let rep = run_scenario(&mut app, &scen).unwrap();
    assert_eq!(
        rep.status,
        PlaytestStatus::Pass,
        "expert rule false positive: {:?}",
        rep.violations
    );
}

#[test]
fn expert_rule_fires_on_violation() {
    let scen: Scenario = serde_json::from_str(
         r#"{"bot":{"type":"chaos","seed":11},"duration_s":0.1,"invariants":[
            {"name":"impossible_policy","rule":"custom","path":"TestApi.active_players","check":"below","value":5,
             "requires_path":"TestApi.active_players","requires_check":"above","requires_value":999}
        ],"setup":{}}"#,
    )
    .unwrap();
    let mut app = build_app();
    let rep = run_scenario(&mut app, &scen).unwrap();
    assert_eq!(
        rep.status,
        PlaytestStatus::Fail,
        "expert rule failed to fire"
    );
    assert!(rep.violations.iter().any(|v| v.rule == "impossible_policy"));
}

#[test]
fn aggressive_persona_never_waits() {
    let scen: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"chaos","seed":13,"persona":"aggressive"},"duration_s":0.5,"invariants":[],"setup":{}}"#,
    )
    .unwrap();
    let mut app = build_app();
    let rep = run_scenario(&mut app, &scen).unwrap();
    let waits = rep
        .coverage
        .intents_emitted
        .get("wait")
        .copied()
        .unwrap_or(0);
    assert_eq!(waits, 0, "aggressive persona must never Wait");
    let total: u64 = rep.coverage.intents_emitted.values().sum();
    assert!(total > 0, "aggressive persona emitted nothing");
}

#[test]
fn mutation_bounds_bug_is_detected_and_control_passes() {
    let scen: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"chaos","seed":42},"duration_s":0.3,"invariants":[
            {"name":"ball_in_bounds","rule":"nodes_in_bounds",
             "min_x":-100,"max_x":100,"min_y":-100,"max_y":100,"min_z":-50,"max_z":50,
             "targets":["Ball"]}
        ],"setup":{}}"#,
    )
    .unwrap();
    {
        let mut app = build_app();
        app.add_systems(Update, inject_bounds_bug);
        let rep = run_scenario(&mut app, &scen).unwrap();
        assert_eq!(rep.status, PlaytestStatus::Fail, "bounds bug not detected");
        assert!(
            rep.violations.iter().any(|v| v.rule == "ball_in_bounds"),
            "no ball_in_bounds violation: {:?}",
            rep.violations
        );
    }
    {
        let mut app = build_app();
        let rep = run_scenario(&mut app, &scen).unwrap();
        assert_eq!(
            rep.status,
            PlaytestStatus::Pass,
            "false positive on healthy game"
        );
    }
}

#[test]
fn mutation_rampant_score_is_detected_and_control_passes() {
    let scen: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"chaos","seed":42},"duration_s":1.5,"invariants":[
            {"name":"score_rate","rule":"custom","path":"TestApi.score",
             "check":"above","value":-1,
             "max_delta_per_sec":50.0}
        ],"setup":{}}"#,
    )
    .unwrap();
    {
        let mut app = build_app();
        app.add_systems(Update, inject_rampant_score_bug);
        let rep = run_scenario(&mut app, &scen).unwrap();
        assert_eq!(
            rep.status,
            PlaytestStatus::Fail,
            "rampant score not detected"
        );
        assert!(
            rep.violations.iter().any(|v| v.rule == "score_rate"),
            "no score_rate violation: {:?}",
            rep.violations
        );
    }
    {
        let mut app = build_app();
        let rep = run_scenario(&mut app, &scen).unwrap();
        assert_eq!(
            rep.status,
            PlaytestStatus::Pass,
            "false positive on healthy game"
        );
    }
}

#[test]
fn mutation_missing_points_detected_by_differential() {
    let scen: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"replay","inputs":[
            {"frame":1,"intent":{"intent":"choice","index":0}},
            {"frame":2,"intent":{"intent":"choice","index":0}},
            {"frame":3,"intent":{"intent":"choice","index":0}}
        ]},"duration_s":0.5,"invariants":[
            {"name":"score_monotonic","rule":"custom","path":"TestApi.score","differential":"no_decrease"}
        ],"setup":{}}"#,
    )
    .unwrap();
    {
        let mut app = build_app_no_scoring();
        app.add_systems(Update, inject_missing_points_bug);
        let rep = run_scenario(&mut app, &scen).unwrap();
        assert_eq!(
            rep.status,
            PlaytestStatus::Fail,
            "missing-points bug not detected"
        );
        assert!(
            rep.violations.iter().any(|v| v.rule == "score_monotonic"),
            "no monotonic violation: {:?}",
            rep.violations
        );
    }
    {
        let mut app = build_app();
        let rep = run_scenario(&mut app, &scen).unwrap();
        assert_eq!(
            rep.status,
            PlaytestStatus::Pass,
            "false positive on healthy game"
        );
    }
}

#[test]
fn synthetic_pointer_dead_chain_is_loudly_rejected() {
    let scen: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"synthetic_pointer","pointer_clicks":[
            {"frame":1,"target":"Ball"}
        ],"seed":1},"duration_s":0.2,"invariants":[],"setup":{}}"#,
    )
    .unwrap();
    let mut app = build_app();
    let rep = run_scenario(&mut app, &scen).unwrap();
    assert!(
        rep.violations
            .iter()
            .any(|v| v.rule == "synthetic_pointer_config"
                && (v.detail.contains("no primary window") || v.detail.contains("no camera"))),
        "headless app must loudly reject pointer synthesis, got: {:?}",
        rep.violations
    );
    assert_eq!(
        rep.status,
        PlaytestStatus::Fail,
        "dead pointer chain passed silently"
    );
}
