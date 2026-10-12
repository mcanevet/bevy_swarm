use crate::enums::PlaytestStatus;
use crate::harness::*;

#[allow(unused_imports)]
use super::fixtures::*;

#[test]
fn planner_bot_basic_goal_achieved() {
    // Simple primitive goal: score > 3, emit Choice(0) until achieved.
    let scenario: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"planner","goals":{"kind":"primitive","path":"TestApi.score","check":"above","value":3,"emit":[{"intent":"choice","index":0}]}},"duration_s":1.0,"invariants":[]}"#,
    )
    .unwrap();
    let mut app = build_app();
    let rep = run_scenario(&mut app, &scenario).unwrap();
    assert_eq!(
        rep.status,
        PlaytestStatus::Pass,
        "violations: {:?}",
        rep.violations
    );
}

#[test]
fn planner_bot_seq_goals() {
    // Sequence: first score > 2, then score > 5.
    let scenario: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"planner","goals":{"kind":"seq","children":[{"kind":"primitive","path":"TestApi.score","check":"above","value":2,"emit":[{"intent":"choice","index":0}]},{"kind":"primitive","path":"TestApi.score","check":"above","value":5,"emit":[{"intent":"choice","index":1}]}]}},"duration_s":1.0,"invariants":[]}"#,
    )
    .unwrap();
    let mut app = build_app();
    let rep = run_scenario(&mut app, &scenario).unwrap();
    assert_eq!(
        rep.status,
        PlaytestStatus::Pass,
        "violations: {:?}",
        rep.violations
    );
}

#[test]
fn planner_bot_any_exhausted() {
    // Any with impossible alternatives → exhausted violation.
    let scenario: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"planner","goals":{"kind":"any","max_s":0.05,"children":[{"kind":"primitive","path":"TestApi.score","check":"above","value":1000,"emit":[]},{"kind":"primitive","path":"TestApi.score","check":"above","value":999,"emit":[]}]}},"duration_s":0.3,"invariants":[]}"#,
    )
    .unwrap();
    let mut app = build_app();
    let rep = run_scenario(&mut app, &scenario).unwrap();
    assert!(
        rep.violations
            .iter()
            .any(|v| v.rule == "planner_any_exhausted"),
        "any should exhaust: {:?}",
        rep.violations
    );
}

#[test]
fn planner_bot_unfinished_check_at_end() {
    // Run ends with goal still unmet → planner_goal_unfinished violation.
    let scenario: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"planner","goals":{"kind":"primitive","path":"TestApi.score","check":"above","value":100,"emit":[{"intent":"choice","index":0}]}},"duration_s":0.1,"invariants":[]}"#,
    )
    .unwrap();
    let mut app = build_app();
    let rep = run_scenario(&mut app, &scenario).unwrap();
    assert!(
        rep.violations
            .iter()
            .any(|v| v.rule == "planner_goal_unfinished"),
        "unfinished goal at run end must be reported: {:?}",
        rep.violations
    );
}
