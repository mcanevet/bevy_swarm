use crate::harness::*;
use bevy::prelude::*;

#[allow(unused_imports)]
use super::fixtures::*;

#[test]
fn oracles_see_same_frame_changes() {
    // A game system moves an entity out of bounds at frame 10; the
    // bounds oracle must report first_frame == 10 (not 9 or 11).
    let scenario: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"replay","inputs":[]},"duration_s":0.5,"invariants":[{"name":"bounds","rule":"nodes_in_bounds"}]}"#,
    )
    .unwrap();
    let mut app = build_app_no_scoring();
    app.add_systems(Update, inject_bounds_bug);
    let rep = run_scenario(&mut app, &scenario).unwrap();
    let bounds_violations: Vec<_> = rep
        .violations
        .iter()
        .filter(|v| v.rule == "bounds")
        .collect();
    assert!(
        !bounds_violations.is_empty(),
        "bounds oracle did not fire, got: {:?}",
        rep.violations
    );
    for v in bounds_violations {
        assert_eq!(
            v.first_frame, 10,
            "bounds violation must be seen at the frame it happened (got {}, log: {:?})",
            v.first_frame, rep.violations
        );
    }
}
