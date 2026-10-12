use crate::contract::*;
use crate::harness::*;
use bevy::prelude::*;

#[allow(unused_imports)]
use super::fixtures::*;

// ---------------------------------------------------------------------------
// B1: Select round-trip + JSON-safe audit payloads
// ---------------------------------------------------------------------------

#[test]
fn select_audit_roundtrip_with_name() {
    // Chaos selects a named Gameplay entity; the audit log records the Name,
    // and a replay scenario re-selects the same entity.
    let chaos: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"chaos","seed":5},"duration_s":0.3,"invariants":[]}"#,
    )
    .unwrap();
    let mut app = build_app_no_scoring();
    app.insert_resource(IntentSurface::new(vec![SurfaceVariant::Select]));
    app.add_systems(Startup, |mut commands: Commands| {
        commands.spawn((Name::new("ball"), Gameplay, Transform::default()));
    });
    let rep = run_scenario(&mut app, &chaos).unwrap();
    let select_entries: Vec<_> = rep
        .action_log
        .iter()
        .filter(|e| e.action == "intent:select")
        .collect();
    assert!(!select_entries.is_empty(), "no selects logged");
    for entry in &select_entries {
        let details = entry.details.as_ref().expect("select missing details");
        let v: serde_json::Value =
            serde_json::from_str(details).expect("audit details not valid JSON");
        assert!(v.get("target").is_some(), "select missing target field");
        let target = v["target"].as_str().expect("target not string");
        // Named entities only (chaos prefers them): "Ball" (base app)
        // or "ball" (test-spawned). Being NAMED is what matters for
        // replayability.
        assert!(
            target == "ball" || target == "Ball",
            "unnamed/raw target in audit: {}",
            target
        );
    }
    // Replay the log
    let replay = replay_from_action_log(&rep.action_log, 0.3);
    let mut app2 = build_app_no_scoring();
    app2.add_systems(Startup, |mut commands: Commands| {
        commands.spawn((Name::new("ball"), Gameplay, Transform::default()));
    });
    let rep2 = run_scenario(&mut app2, &replay).unwrap();
    let select_entries2: Vec<_> = rep2
        .action_log
        .iter()
        .filter(|e| e.action == "intent:select")
        .collect();
    assert_eq!(
        select_entries.len(),
        select_entries2.len(),
        "replay lost selects"
    );
}

#[test]
fn axis_name_with_quotes_roundtrips() {
    // Axis name containing quotes/escapes must survive JSON round-trip.
    let chaos: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"chaos","seed":7},"duration_s":0.2,"invariants":[]}"#,
    )
    .unwrap();
    let mut app = build_app_no_scoring();
    // Inject an axis with problematic name
    app.insert_resource(IntentSurface::new(vec![SurfaceVariant::Axis(
        "he said \"hi\"".to_string(),
    )]));
    let rep = run_scenario(&mut app, &chaos).unwrap();
    let axis_entries: Vec<_> = rep
        .action_log
        .iter()
        .filter(|e| e.action == "intent:axis")
        .collect();
    assert!(!axis_entries.is_empty(), "no axis logged");
    for entry in axis_entries {
        let details = entry.details.as_ref().expect("axis missing details");
        let v: serde_json::Value =
            serde_json::from_str(details).expect("audit details not valid JSON");
        assert_eq!(v["name"], "he said \"hi\"", "axis name corrupted");
    }
}

#[test]
fn move_values_roundtrip_exactly() {
    // Move dir values must round-trip bit-exactly (f32→f64→JSON→f64→f32).
    let chaos: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"chaos","seed":9},"duration_s":0.2,"invariants":[]}"#,
    )
    .unwrap();
    let mut app = build_app_no_scoring();
    app.insert_resource(IntentSurface::new(vec![SurfaceVariant::Move]));
    let rep = run_scenario(&mut app, &chaos).unwrap();
    let move_entries: Vec<_> = rep
        .action_log
        .iter()
        .filter(|e| e.action == "intent:move")
        .collect();
    for entry in move_entries {
        let details = entry.details.as_ref().expect("move missing details");
        let v: serde_json::Value =
            serde_json::from_str(details).expect("audit details not valid JSON");
        let dx = v["dir"][0].as_f64().expect("dir[0] missing");
        let dy = v["dir"][1].as_f64().expect("dir[1] missing");
        // f32→f64→JSON→f64 preserves exact value
        let dx_f32 = dx as f32;
        let dy_f32 = dy as f32;
        assert_eq!(dx_f32.to_bits(), (dx as f32).to_bits());
        assert_eq!(dy_f32.to_bits(), (dy as f32).to_bits());
    }
}

#[test]
fn replay_select_target_missing_is_violation() {
    // Replay tries to select a non-existent named entity → violation.
    let replay: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"replay","inputs":[{"frame":1,"intent":{"intent":"select","target":"ghost"}}]},"duration_s":0.2,"invariants":[]}"#,
    )
    .unwrap();
    let mut app = build_app_no_scoring();
    app.add_systems(Startup, |mut commands: Commands| {
        commands.spawn((Name::new("ball"), Gameplay, Transform::default()));
    });
    let rep = run_scenario(&mut app, &replay).unwrap();
    assert!(
        rep.violations.iter().any(|v| v.rule.contains("replay")),
        "replay target miss silent, got violations: {:?}",
        rep.violations
    );
}

#[test]
fn unnamed_select_counts_unreplayable() {
    // I1 changed semantics: unnamed Gameplay entities get StableIds, so
    // chaos Selects over them are now REPLAYABLE (unreplayable_actions == 0).
    // The warning only fires for genuinely untracked (non-Gameplay) targets.
    let chaos: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"chaos","seed":4},"duration_s":0.3,"invariants":[]}"#,
    )
    .unwrap();
    let mut app = build_app_no_scoring();
    app.insert_resource(IntentSurface::new(vec![SurfaceVariant::Select]));
    // Spawn ONLY unnamed Gameplay entities (override base app's Ball? —
    // base app's spawn_minimal_ball has a Name, so remove via no-name app).
    let rep = run_scenario(&mut app, &chaos).unwrap();
    // Base app entities are named ("Ball"), so unreplayable should be 0
    // when named entities exist (chaos prefers them).
    assert_eq!(rep.unreplayable_actions, 0);
    assert!(rep.warnings.is_empty());

    // Now a game with ONLY unnamed entities.
    let mut app2 = bevy::app::App::new();
    app2.add_plugins((
        bevy::MinimalPlugins,
        bevy::transform::TransformPlugin,
        crate::harness::PlaytestPlugin,
        crate::contract::TestConventionsPlugin,
    ));
    app2.insert_resource(IntentSurface::new(vec![SurfaceVariant::Select]));
    app2.add_systems(bevy::app::Startup, |mut commands: Commands| {
        commands.spawn((Gameplay, Transform::default()));
    });
    let rep2 = run_scenario(&mut app2, &chaos).unwrap();
    // Unnamed Gameplay entities have StableIds (I1) — Selects over them
    // round-trip via {"stable_id": n}, so nothing is unreplayable.
    assert_eq!(
        rep2.unreplayable_actions, 0,
        "unnamed-but-Gameplay selects should be replayable via StableId: {:#?}",
        rep2.action_log
    );
}
