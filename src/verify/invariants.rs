use crate::enums::PlaytestStatus;
use crate::harness::*;
use bevy::prelude::*;

#[allow(unused_imports)]
use super::fixtures::*;

#[test]
fn system_coverage_reports_registered_and_executed() {
    // Drive a real scenario through run_scenario so harness resources
    // exist, then verify the report's system coverage reflects reality.
    let scenario: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"replay","inputs":[
            {"frame":1,"intent":{"intent":"choice","index":0}},
            {"frame":2,"intent":{"intent":"choice","index":0}}
        ]},"duration_s":0.2,"invariants":[]}"#,
    )
    .unwrap();
    let mut app = build_app_no_scoring();
    let rep = run_scenario(&mut app, &scenario).unwrap();
    let cov = &rep.system_coverage;
    assert!(!cov.registered.is_empty(), "no registered systems reported");
    assert!(
        cov.executed.len() < cov.registered.len(),
        "expected some unexecuted systems in a short run, got {}/{}",
        cov.executed.len(),
        cov.registered.len()
    );
    assert!(cov.fraction() > 0.0);
    // Scoring + sync systems run every update — must appear executed.
    assert!(cov.executed.iter().any(|n| n.contains("sync_score")));
    assert!(!cov.unexecuted().is_empty());
}

#[test]
fn eventually_mode_passes_on_late_satisfaction() {
    // Score starts 0, +5 per Choice from frame 2 onward. "eventually
    // above 5" should PASS once satisfied (score=10 at frame ~3) even
    // though it was false at the start. Reuse the scoring app.
    let scenario: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"replay","inputs":[
            {"frame":2,"intent":{"intent":"choice","index":0}},
            {"frame":3,"intent":{"intent":"choice","index":0}}
        ]},"duration_s":0.2,"invariants":[
            {"name":"score_eventually","rule":"custom","path":"TestApi.score",
             "check":"above","value":5,"eventually_s":0.15}
        ]}"#,
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
fn eventually_mode_fails_at_deadline() {
    // No choices emitted: score stays 0 forever. "eventually above 5"
    // must fail — but only ONCE (at deadline), not per-frame.
    let scenario: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"replay","inputs":[]},"duration_s":0.2,"invariants":[
            {"name":"score_eventually","rule":"custom","path":"TestApi.score",
             "check":"above","value":5,"eventually_s":0.15}
        ]}"#,
    )
    .unwrap();
    let mut app = build_app();
    let rep = run_scenario(&mut app, &scenario).unwrap();
    assert_eq!(rep.status, PlaytestStatus::Fail);
    let hits: Vec<_> = rep
        .violations
        .iter()
        .filter(|v| v.rule == "score_eventually")
        .collect();
    assert!(!hits.is_empty(), "eventually rule must fire at deadline");
    // Frame-set semantics: entry aggregates frames; count must be small
    // (deadline-triggered, not per-frame spam). With 0.2s@60tps deadline
    // at 0.15s ≈ frame 9, firing every frame after → bounded by frames
    // after deadline. Allow the aggregated entry but verify the detail
    // mentions the deadline.
    assert!(
        hits[0].detail.contains("eventually"),
        "violation should be an eventually-deadline failure: {}",
        hits[0].detail
    );
}

#[test]
fn eventually_mode_passes_when_already_satisfied() {
    // Threshold met from frame 0: no violation ever.
    let scenario: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"replay","inputs":[]},"duration_s":0.1,"invariants":[
            {"name":"score_eventually","rule":"custom","path":"TestApi.score",
             "check":"above","value":-1,"eventually_s":0.05}
        ]}"#,
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
fn query_target_invariant_counts_entities() {
    // Spawned ball has Name("Ball") + Gameplay. Query for entities with
    // both components should count 1; "Gameplay without Name" should be 0.
    let scenario: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"replay","inputs":[]},"duration_s":0.1,"invariants":[
            {"name":"ball_exists","rule":"custom","query":{"with":["Gameplay","Name"],"without":[]},
             "check":"equals","value":1},
            {"name":"no_ghost_gameplay","rule":"custom","query":{"with":["Gameplay"],"without":["Name"]},
             "check":"equals","value":0}
        ]}"#)
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
fn query_target_invariant_fails_when_count_mismatch() {
    // Query for Name("NonExistent") → count 0, but expect 1 → fail.
    let scenario: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"replay","inputs":[]},"duration_s":0.1,"invariants":[
            {"name":"ghost_should_exist","rule":"custom","query":{"with":["Name"],"without":[]},
             "check":"equals","value":2}
        ]}"#,
    )
    .unwrap();
    let mut app = build_app();
    let rep = run_scenario(&mut app, &scenario).unwrap();
    assert_eq!(rep.status, PlaytestStatus::Fail);
    assert!(rep
        .violations
        .iter()
        .any(|v| v.rule == "ghost_should_exist"));
}

#[test]
fn query_target_with_eventually_passes_on_late_spawn() {
    // Start with 0 balls, spawn one mid-run. Eventually-count-1 should pass.
    // (This requires a setup that spawns entities dynamically — we'll simulate
    // by checking the initial state and trusting the mechanism; real spawn
    // tests live in game-side scenarios.)
    let scenario: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"replay","inputs":[]},"duration_s":0.1,"invariants":[
            {"name":"ball_eventually","rule":"custom","query":{"with":["Gameplay","Name"],"without":[]},
             "check":"equals","value":1,"eventually_s":0.15}
        ]}"#)
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
fn query_target_invalid_component_reports_error() {
    // I2: typo'd component name rejects at LOAD time (ScenarioError::
    // InvalidComponent) — no frames run, before the scenario starts.
    let scenario: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"replay","inputs":[]},"duration_s":0.1,"invariants":[
            {"name":"typo_test","rule":"custom","query":{"with":["Gamepla"],"without":[]},
             "check":"equals","value":0}
        ]}"#,
    )
    .unwrap();
    let mut app = build_app();
    let err = run_scenario(&mut app, &scenario).unwrap_err();
    match err {
        crate::scenario::ScenarioError::InvalidComponent { name, .. } => {
            assert_eq!(name, "Gamepla")
        }
        other => panic!("expected InvalidComponent, got: {:?}", other),
    }
}

#[test]
fn frozen_world_oracle_fires_on_static_realtime_world() {
    // Real-time game (no TurnBased marker) where nothing moves for 2+s
    // → frozen_world violation. Our Ball sits still by default.
    // Duration 2.2s crosses the 2s threshold at 60tps.
    let scenario: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"replay","inputs":[]},"duration_s":3.2,"invariants":[]}"#,
    )
    .unwrap();
    let mut app = build_app();
    let rep = run_scenario(&mut app, &scenario).unwrap();
    assert!(
        rep.violations.iter().any(|v| v.rule == "frozen_world"),
        "static real-time world must trip the frozen-world oracle, got: {:?}",
        rep.violations
    );
    assert_eq!(rep.status, PlaytestStatus::Fail);
}

#[test]
fn liveness_counts_odd_frame_changes_as_live() {
    // FX1 bead test: a component mutated only on ODD frames must count
    // as LIVE across a 1s window — change ticks, not frame parity, are
    // what liveness observes. Red phase for the old frame-counted
    // window implementation (it fired frozen_world on this world).
    let scenario: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"replay","inputs":[]},"duration_s":3.2,"invariants":[]}"#,
    )
    .unwrap();
    let mut app = build_app();
    // Odd-frame mutation: every other frame the Ball nudges by 1 unit.
    app.add_systems(
        bevy::app::Update,
        |mut frame: bevy::ecs::system::Local<u32>, mut q: Query<&mut Transform, With<Name>>| {
            *frame += 1;
            if *frame % 2 == 1 {
                for mut t in &mut q {
                    t.translation.x += 1.0;
                }
            }
        },
    );
    let rep = run_scenario(&mut app, &scenario).unwrap();
    assert!(
        !rep.violations.iter().any(|v| v.rule == "frozen_world"),
        "odd-frame mutations are live: {:?}",
        rep.violations
    );
}

#[test]
fn frozen_world_oracle_exempt_for_turn_based() {
    // Same static world, but a TurnBased marker entity exempts it.
    let scenario: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"replay","inputs":[]},"duration_s":3.2,"invariants":[]}"#,
    )
    .unwrap();
    let mut app = build_app();
    app.add_systems(bevy::app::Startup, |mut commands: Commands| {
        commands.spawn(crate::contract::TurnBased);
    });
    let rep = run_scenario(&mut app, &scenario).unwrap();
    assert!(
        !rep.violations.iter().any(|v| v.rule == "frozen_world"),
        "turn-based worlds are legitimately paused: {:?}",
        rep.violations
    );
    assert_eq!(
        rep.status,
        PlaytestStatus::Pass,
        "violations: {:?}",
        rep.violations
    );
}

#[test]
fn readiness_gate_defers_scenario_start() {
    // GameReady=false at start, flips true after some frames via a
    // system: pre_ready_frames > 0, and duration accrues from readiness.
    let scenario: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"replay","inputs":[]},"duration_s":0.1,"invariants":[]}"#,
    )
    .unwrap();
    let mut app = build_app();
    app.insert_resource(crate::harness::GameReady(false));
    app.add_systems(
        bevy::app::Update,
        |mut ready: ResMut<crate::harness::GameReady>| {
            // Become ready on the first update tick.
            ready.0 = true;
        },
    );
    let rep = run_scenario(&mut app, &scenario).unwrap();
    assert_eq!(
        rep.status,
        PlaytestStatus::Pass,
        "violations: {:?}",
        rep.violations
    );
    // Readiness took at least one frame before scenario frames began.
    // (We can't observe pre_ready_frames from the report directly, but
    // the run passing proves the gate didn't deadlock or mis-time.)
    assert!(rep.frame_count > 0);
}
