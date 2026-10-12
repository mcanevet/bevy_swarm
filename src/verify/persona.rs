use crate::contract::*;
use crate::enums::PlaytestStatus;
use crate::harness::*;
use bevy::prelude::*;

#[allow(unused_imports)]
use super::fixtures::*;

#[test]
fn aggressive_persona_no_hang_on_wait_only_surface() {
    // Aggressive persona on a Wait-only surface must NOT loop forever;
    // it draws from the filtered candidate set (empty → yields Wait).
    let mut app = build_app_no_scoring();
    app.insert_resource(IntentSurface::new(vec![SurfaceVariant::Wait]));
    app.insert_resource(crate::oracles::GameReady(true));
    let scen: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"chaos","seed":1,"persona":"aggressive"},"duration_s":0.2,"invariants":[]}"#,
    )
    .unwrap();
    let rep = run_scenario(&mut app, &scen).unwrap();
    assert_eq!(rep.status, PlaytestStatus::Pass);
}

#[test]
fn planner_emits_from_first_intent() {
    // Planner pursuit sequence must start at emit[0] when a primitive
    // activates (previously indexed by ABSOLUTE frame, so the first
    // intent depended on when the goal was activated).
    let scen: Scenario = serde_json::from_str(
        r#"{
            "bot":{"type":"planner","goals":{"kind":"primitive","path":"TestApi.score","check":"ge","value":1,"emit":[{"intent":"wait"}]}},
            "duration_s":0.5
        }"#,
    )
    .unwrap();
    let mut app = build_app();
    let rep = run_scenario(&mut app, &scen).unwrap();
    // The single wait intent should have been emitted (at least one audit entry with intent:wait)
    assert!(rep
        .action_log
        .iter()
        .any(|e| e.action.starts_with("intent:wait")));
}

#[test]
fn planner_respects_input_rate() {
    // Planner pursuit must pace by input_rate_hz (same fire_every as chaos).
    // With tps=60, rate=10, fire_every=6; over 0.5s (30 frames) we expect
    // roughly 5 emissions, not 30.
    let scen: Scenario = serde_json::from_str(
        r#"{
            "bot":{"type":"planner","input_rate_hz":10,"goals":{"kind":"primitive","path":"TestApi.score","check":"ge","value":1,"emit":[{"intent":"wait"},{"intent":"wait"}]}},
            "tps":60,
            "duration_s":0.5
        }"#,
    )
    .unwrap();
    let mut app = build_app();
    let rep = run_scenario(&mut app, &scen).unwrap();
    let wait_count = rep
        .action_log
        .iter()
        .filter(|e| e.action.starts_with("intent:wait"))
        .count();
    // fire_every = 60/10 = 6; 30 frames / 6 ≈ 5 ticks. Allow some slack.
    assert!(
        wait_count <= 8,
        "too many waits: {} (rate not respected)",
        wait_count
    );
}
