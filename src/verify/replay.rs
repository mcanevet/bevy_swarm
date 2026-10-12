use crate::harness::*;
use bevy::prelude::*;

#[allow(unused_imports)]
use super::fixtures::*;

// ---------------------------------------------------------------------------
// A2: explicit PlaytestSet ordering for frame-accurate replay
// ---------------------------------------------------------------------------

#[test]
fn replay_of_audit_log_reproduces_audit_log() {
    // Core shrinker guarantee: running a chaos scenario, converting its
    // action_log to a replay scenario and running on a fresh App must
    // yield an IDENTICAL (frame, action, details) sequence.
    let chaos: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"chaos","seed":3},"duration_s":0.5,"invariants":[]}"#,
    )
    .unwrap();
    let mut app1 = build_app();
    let rep1 = run_scenario(&mut app1, &chaos).unwrap();
    assert!(
        !rep1.action_log.is_empty(),
        "chaos run produced no audit entries"
    );
    // Convert to a replay scenario via the minimize helper path.
    let replay_scenario = replay_from_action_log(&rep1.action_log, 0.5);
    let mut app2 = build_app();
    let rep2 = run_scenario(&mut app2, &replay_scenario).unwrap();

    // Compare (frame, action, details) — kind-only comparison would
    // hide drift in the payload (e.g. coordinates dropped on replay).
    let seq1: Vec<(u64, String, Option<String>)> = rep1
        .action_log
        .iter()
        .map(|e| (e.frame, e.action.clone(), e.details.clone()))
        .collect();
    let seq2: Vec<(u64, String, Option<String>)> = rep2
        .action_log
        .iter()
        .map(|e| (e.frame, e.action.clone(), e.details.clone()))
        .collect();
    assert_eq!(
        seq1, seq2,
        "replayed audit log diverged from original: {seq1:?} vs {seq2:?}"
    );
}
