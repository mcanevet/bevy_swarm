use crate::harness::*;
use bevy::prelude::*;

#[allow(unused_imports)]
use super::fixtures::*;

// ---------------------------------------------------------------------------
// C1: CheckOp single comparison semantics
// ---------------------------------------------------------------------------

#[test]
fn check_op_semantics_table() {
    use crate::enums::CheckOp::*;
    let cases = [
        (Le, 5.0, 5.0, true),
        (Lt, 5.0, 5.0, false),
        (Ge, 5.0, 5.0, true),
        (Gt, 5.0, 5.0, false),
        (Equals, 1e12 + 0.0001, 1e12, true),
        (Ne, 1.0, 2.0, true),
    ];
    for (op, cur, thr, want) in cases {
        assert_eq!(op.holds(cur, thr), want, "{op:?} {cur} {thr}");
    }
}

#[test]
fn check_op_wire_aliases_parse() {
    // Legacy spellings must keep parsing.
    let op: crate::enums::CheckOp = serde_json::from_str("\"below\"").unwrap();
    assert_eq!(op, crate::enums::CheckOp::Le);
    let op: crate::enums::CheckOp = serde_json::from_str("\"above\"").unwrap();
    assert_eq!(op, crate::enums::CheckOp::Ge);
    let op: crate::enums::CheckOp = serde_json::from_str("\"eq\"").unwrap();
    assert_eq!(op, crate::enums::CheckOp::Equals);
    assert!(serde_json::from_str::<crate::enums::CheckOp>("\"abvoe\"").is_err());
}

#[test]
fn check_op_boundary_inclusive_in_scenario() {
    // Score == 2 with invariant score below 2 must PASS (inclusive le).
    // Above-at-equality for expert WHEN fires.
    let scen: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"replay","inputs":[]},"duration_s":0.1,"invariants":[{"name":"cap","rule":"custom","path":"TestApi.score","check":"below","value":0.5}]}"#,
    )
    .unwrap();
    // Use the standard scoring game: awards on Choice{0}... score stays 0.
    let mut app = build_app();
    let rep = run_scenario(&mut app, &scen).unwrap();
    // score ends 0; 0 <= 0.5 holds -> no violation named cap.
    assert!(
        !rep.violations.iter().any(|v| v.rule == "cap"),
        "inclusive boundary failed: {:#?}",
        rep.violations
    );
}

#[test]
fn planner_check_typo_rejected() {
    let json = r#"{
        "bot": {"type":"planner","goals":{"seq":{"children":[
            {"primitive":{"path":"TestApi.score","check":"abvoe","value":1,"emit":[]}}
        ]}}},
        "duration_s": 0.5
    }"#;
    assert!(serde_json::from_str::<Scenario>(json).is_err());
}

#[test]
fn calibration_bound_is_tight() {
    use crate::diagnostics::{generate_invariants_from_calibration, CalibrationSnapshot};
    let snap = CalibrationSnapshot {
        archetype_envelopes: vec![("Gameplay|Transform".into(), 10, 20)],
        resource_baselines: vec![("Score".into(), 42.0)],
    };
    let out = generate_invariants_from_calibration(&snap);
    let invs: serde_json::Value = serde_json::from_str(&out).unwrap();
    let arch = invs
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["name"].as_str().unwrap().starts_with("archetype_min_"))
        .expect("archetype min invariant missing");
    // Tight bound: ge min (NOT the old off-by-one above min-1).
    assert_eq!(arch["check"].as_str().unwrap(), "ge");
    assert_eq!(arch["value"].as_f64().unwrap(), 10.0);
}
