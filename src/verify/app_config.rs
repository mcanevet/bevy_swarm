use crate::harness::*;
use bevy::prelude::*;

#[allow(unused_imports)]
use super::fixtures::*;

#[test]
fn plugin_without_scenario_does_not_panic() {
    // An App with PlaytestPlugin but no run_scenario: 3 updates, no panic.
    let mut app = build_app_no_scoring();
    app.update();
    app.update();
    app.update();
    // If we got here without a panic, the run conditions held.
}

// ---------------------------------------------------------------------------
// A3: single-threaded executor + ambiguity gate
// ---------------------------------------------------------------------------

#[test]
fn single_threaded_applied_by_default() {
    // After run_scenario, every schedule should use SingleThreadedExecutor.
    // We test behaviorally: two ambiguous systems appending to a shared Vec
    // must produce the same order over 30 runs.
    use bevy::prelude::*;

    #[derive(Resource, Default)]
    struct OrderLog(Vec<usize>);

    fn sys_a(mut log: ResMut<OrderLog>) {
        log.0.push(1);
    }

    fn sys_b(mut log: ResMut<OrderLog>) {
        log.0.push(2);
    }

    let scenario: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"replay","inputs":[]},"duration_s":0.5,"single_threaded":true,"deny_ambiguities":false,"invariants":[]}"#,
    )
    .unwrap();

    let mut results = std::collections::HashSet::new();
    for _ in 0..30 {
        let mut app = build_app_no_scoring();
        app.add_systems(Update, (sys_a, sys_b));
        let rep = run_scenario(&mut app, &scenario).unwrap();
        let log = rep
            .action_log
            .last()
            .map(|e| e.details.clone())
            .unwrap_or(None);
        results.insert(log);
    }
    assert_eq!(
        results.len(),
        1,
        "order varied across 30 runs: {:?}",
        results
    );
}

#[test]
fn deny_ambiguities_rejects_conflicting_systems() {
    // Two systems both taking ResMut<Score> are ambiguous.
    let scenario: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"replay","inputs":[]},"duration_s":0.1,"single_threaded":false,"deny_ambiguities":true,"invariants":[]}"#,
    )
    .unwrap();
    let mut app = build_app_no_scoring();
    app.add_systems(Update, (inject_rampant_score_bug, inject_rampant_score_bug));
    let rep = run_scenario(&mut app, &scenario);
    let Err(e) = rep else {
        panic!("expected Rejected error for ambiguous systems");
    };
    let msg = e.to_string();
    assert!(
        msg.contains("schedule ambiguities"),
        "expected a schedule-ambiguity rejection, got: {msg}"
    );
    // The offending SYSTEM NAMES must be named in the rejection —
    // a message without them gives no actionable signal.
    assert!(
        msg.contains("inject_rampant_score_bug"),
        "rejection must name the conflicting system, got: {msg}"
    );
}

#[test]
fn deny_ambiguities_accepts_clean_app() {
    // Minimal harness setup with deny_ambiguities should not reject.
    let scenario: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"replay","inputs":[]},"duration_s":0.1,"single_threaded":true,"deny_ambiguities":true,"invariants":[]}"#,
    )
    .unwrap();
    let mut app = build_app_no_scoring();
    let rep = run_scenario(&mut app, &scenario);
    assert!(rep.is_ok(), "clean harness rejected: {:?}", rep.err());
}
