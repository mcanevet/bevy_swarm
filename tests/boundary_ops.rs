//! FX5: value == bound boundary behaviour at EVERY evaluation site.
//!
//! Every check operator goes through `CheckOp::holds` / `holds_text`
//! (oracles: invariant always/eventually, WHEN guard, expert REQUIRE,
//! count queries) or `primitive_achieved` (planner primitives, same
//! underlying ops). All sites share `CheckOp` as the single source of
//! truth, so this suite is table-driven: for each op, the value exactly
//! AT the bound must behave per the op's inclusive/exclusive semantics
//! (below = Le is inclusive, above = Ge is inclusive, lt/gt exclusive,
//! equals inclusive, ne exclusive at the bound).

use bevy::prelude::*;
use bevy_swarm::driver::{run_scenario, PlaytestPlugin};
use bevy_swarm::enums::CheckOp;
use bevy_swarm::scenario::Scenario;

/// Ops (wire name, enum) and whether value == bound HOLDS.
const BOUNDARY_TABLE: &[(&str, CheckOp, bool)] = &[
    // Inclusive bounds: v == thr passes.
    ("below", CheckOp::Le, true),
    ("le", CheckOp::Le, true),
    ("above", CheckOp::Ge, true),
    ("ge", CheckOp::Ge, true),
    ("equals", CheckOp::Equals, true),
    ("eq", CheckOp::Equals, true),
    // Exclusive bounds: v == thr fails.
    ("lt", CheckOp::Lt, false),
    ("gt", CheckOp::Gt, false),
    ("ne", CheckOp::Ne, false),
];

fn build_app() -> App {
    let mut app = App::new();
    app.add_plugins((
        bevy::MinimalPlugins,
        bevy::transform::TransformPlugin,
        PlaytestPlugin,
        bevy_swarm::contract::TestConventionsPlugin,
    ));
    app.insert_resource(bevy_swarm::contract::TestApi::default());
    app.add_message::<bevy_swarm::contract::UserIntent>();
    app
}

/// Site 1 — the single source of truth: `CheckOp::holds` at v == thr.
#[test]
fn check_op_holds_at_exact_bound() {
    for (wire, op, expected) in BOUNDARY_TABLE {
        assert_eq!(
            op.holds(10.0, 10.0),
            *expected,
            "{wire}: value == bound must {}",
            if *expected { "pass" } else { "fail" }
        );
    }
}

/// Site 2 — `CheckOp::holds_text` at v == bound (Equals/Ne only).
#[test]
fn check_op_holds_text_at_exact_match() {
    assert!(CheckOp::Equals.holds_text("x", "x") == Some(true));
    assert!(CheckOp::Ne.holds_text("x", "x") == Some(false));
    assert!(CheckOp::Le.holds_text("x", "x").is_none());
}

/// Site 3 — invariant oracle (always-mode), via run_scenario: a
/// scenario whose value sits exactly AT each op's bound.
#[test]
fn invariant_at_bound_behaves_per_op_semantics() {
    // score is pinned to 42; each invariant's threshold IS 42.
    let invs: Vec<String> = BOUNDARY_TABLE
        .iter()
        .map(|(wire, _, _)| {
            format!(
                r#"{{"name":"b_{wire}","rule":"custom","path":"TestApi.score","check":"{wire}","value":42}}"#
            )
        })
        .collect();
    let scen: Scenario = serde_json::from_str(&format!(
        r#"{{"bot":{{"type":"replay","inputs":[]}},"duration_s":0.1,"invariants":[{}]}}"#,
        invs.join(",")
    ))
    .unwrap();
    let mut app = build_app();
    app.add_systems(Update, |mut api: ResMut<bevy_swarm::contract::TestApi>| {
        api.score = 42
    });
    let rep = run_scenario(&mut app, &scen).unwrap();
    for (wire, _, expected_holds) in BOUNDARY_TABLE {
        let violated = rep.violations.iter().any(|v| v.rule == format!("b_{wire}"));
        assert_eq!(
            violated,
            !expected_holds,
            "invariant {wire}: v == 42 vs bound 42 must {}",
            if *expected_holds { "hold" } else { "violate" }
        );
    }
}

/// Site 4 — invariant oracle (eventually-mode): a value AT the bound
/// satisfies an inclusive op immediately (no deadline violation), and
/// never satisfies an exclusive op (deadline expires -> violation).
#[test]
fn eventually_at_bound_behaves_per_op_semantics() {
    let scen: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"replay","inputs":[]},"duration_s":0.1,"invariants":[
            {"name":"ev_le","rule":"custom","path":"TestApi.score","check":"le","value":42,"eventually_s":0.05},
            {"name":"ev_gt","rule":"custom","path":"TestApi.score","check":"gt","value":42,"eventually_s":0.05}]}"#,
    )
    .unwrap();
    let mut app = build_app();
    app.add_systems(Update, |mut api: ResMut<bevy_swarm::contract::TestApi>| {
        api.score = 42
    });
    let rep = run_scenario(&mut app, &scen).unwrap();
    assert!(
        !rep.violations.iter().any(|v| v.rule == "ev_le"),
        "le at the bound is satisfied immediately (inclusive)"
    );
    assert!(
        rep.violations.iter().any(|v| v.rule == "ev_gt"),
        "gt at the bound never satisfies -> eventually deadline expires: {:?}",
        rep.violations
    );
}

/// Site 5 — WHEN guard of an expert rule: WHEN v == bound fires for
/// inclusive ops (so a failing REQUIRE is reported), does NOT fire for
/// exclusive ops (REQUIRE failure is masked — vacuous truth).
#[test]
fn expert_when_at_bound_gates_per_op_semantics() {
    let scen: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"replay","inputs":[]},"duration_s":0.1,"invariants":[
            {"name":"when_le","rule":"custom","path":"TestApi.score","check":"le","value":42,
             "requires_path":"TestApi.active_players","requires_check":"lt","requires_value":1},
            {"name":"when_gt","rule":"custom","path":"TestApi.score","check":"gt","value":42,
             "requires_path":"TestApi.active_players","requires_check":"lt","requires_value":1}]}"#,
    )
    .unwrap();
    let mut app = build_app();
    app.add_systems(Update, |mut api: ResMut<bevy_swarm::contract::TestApi>| {
        api.score = 42;
        api.active_players = 5; // REQUIRE (lt 1) always fails
    });
    let rep = run_scenario(&mut app, &scen).unwrap();
    // WHEN le @ 42 holds -> REQUIRE failure reported.
    assert!(
        rep.violations.iter().any(|v| v.rule == "when_le"),
        "WHEN le at the bound fires (inclusive): {:?}",
        rep.violations
    );
    // WHEN gt @ 42 does NOT hold -> REQUIRE failure masked.
    assert!(
        !rep.violations.iter().any(|v| v.rule == "when_gt"),
        "WHEN gt at the bound does not fire (exclusive): {:?}",
        rep.violations
    );
}

/// Site 6 — compiled count-query invariant: count == bound behaves per
/// the op (goes through the compiled fast path, CompiledKind::
/// QueryCount, via oracles).
#[test]
fn count_query_at_bound_behaves_per_op_semantics() {
    use bevy_swarm::contract::Gameplay;
    let scen: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"replay","inputs":[]},"duration_s":0.1,"invariants":[
            {"name":"cq_ge","rule":"custom","query":{"with":["Gameplay"],"without":[]},"check":"ge","value":3},
            {"name":"cq_gt","rule":"custom","query":{"with":["Gameplay"],"without":[]},"check":"gt","value":3},
            {"name":"cq_le","rule":"custom","query":{"with":["Gameplay"],"without":[]},"check":"le","value":3},
            {"name":"cq_lt","rule":"custom","query":{"with":["Gameplay"],"without":[]},"check":"lt","value":3}]}"#,
    )
    .unwrap();
    let mut app = build_app();
    app.add_systems(bevy::app::Startup, |mut commands: Commands| {
        for _ in 0..3 {
            commands.spawn(Gameplay);
        }
    });
    let rep = run_scenario(&mut app, &scen).unwrap();
    let violated = |rule: &str| rep.violations.iter().any(|v| v.rule == rule);
    assert!(!violated("cq_ge"), "ge with count == bound holds");
    assert!(!violated("cq_le"), "le with count == bound holds");
    assert!(violated("cq_gt"), "gt with count == bound fails");
    assert!(violated("cq_lt"), "lt with count == bound fails");
}

/// Site 7 — planner primitive goal achieved check: a primitive with
/// value AT the bound achieves (completes without violation) for
/// inclusive ops and remains unfinished for exclusive ops.
#[test]
fn planner_primitive_at_bound_behaves_per_op_semantics() {
    let mk = |check: &str, value: i64| {
        format!(
            r#"{{"bot":{{"type":"planner","goals":{{"kind":"primitive","path":"TestApi.score","check":"{check}","value":{value},"emit":[{{"intent":"wait"}}]}}}},"duration_s":0.1,"invariants":[]}}"#
        )
    };
    // score pinned to 42; primitive bounds are 42.
    let run = |scenario_json: String| {
        let scen: Scenario = serde_json::from_str(&scenario_json).unwrap();
        let mut app = build_app();
        app.add_systems(Update, |mut api: ResMut<bevy_swarm::contract::TestApi>| {
            api.score = 42
        });
        run_scenario(&mut app, &scen).unwrap()
    };

    for (wire, _, expected_holds) in BOUNDARY_TABLE {
        // Numeric ops only (planner string comparisons accept
        // equals/ne and are covered by holds_text tests).
        let rep = run(mk(wire, 42));
        let unfinished = rep
            .violations
            .iter()
            .any(|v| v.rule == "planner_goal_unfinished");
        assert_eq!(
            unfinished,
            !expected_holds,
            "planner primitive {wire}: v == 42 vs bound 42 must {}",
            if *expected_holds {
                "achieve"
            } else {
                "stay unfinished"
            }
        );
    }
}
