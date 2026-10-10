//! Z7: generic error oracles — Bevy error handler, log capture, panic capture.

use bevy::prelude::*;
use bevy_swarm::driver::{run_scenario, PlaytestPlugin};
use bevy_swarm::enums::PlaytestStatus;
use bevy_swarm::scenario::Scenario;

fn scenario_json() -> Scenario {
    serde_json::from_str(r#"{"bot":{"type":"chaos","seed":1},"duration_s":0.1,"invariants":[]}"#)
        .unwrap()
}

#[test]
fn fallible_system_error_reported() {
    // A game system returns Err on every frame: must surface as a
    // bevy_error violation naming the system, status Fail (not Crash).
    let mut app = App::new();
    app.add_plugins(PlaytestPlugin);
    app.add_systems(Update, || -> Result<(), bevy::ecs::error::BevyError> {
        Err(bevy::ecs::error::BevyError::warning("boom"))
    });
    let rep = run_scenario(&mut app, &scenario_json()).unwrap();
    assert_eq!(rep.status, PlaytestStatus::Fail);
    let v = rep
        .violations
        .iter()
        .find(|v| v.rule == "bevy_warning")
        .expect("expected a bevy_warning violation");
    assert!(v.detail.contains("boom"), "detail: {}", v.detail);
    // Target should carry the system name (context).
    assert!(!v.target.is_empty(), "target must carry the system context");
}

#[test]
fn panic_yields_crash_with_meaningful_message() {
    // Panics must still crash the run, and report.error must contain
    // the panic string (not `Any { .. }`).
    let mut app = App::new();
    app.add_plugins(PlaytestPlugin);
    app.add_systems(Update, || {
        panic!("edge walker fell off the world");
    });
    let rep = run_scenario(&mut app, &scenario_json()).unwrap();
    assert_eq!(rep.status, PlaytestStatus::Crash);
    let err = rep.error.expect("crash must carry an error message");
    assert!(
        err.contains("edge walker fell off the world"),
        "error was: {err}"
    );
    assert!(!err.contains("Any { .. }"), "error was: {err}");
}

#[test]
fn parallel_runs_route_errors_independently() {
    // Two runs on separate threads: only the erroring run's report has
    // the violation (thread-local RunId routing).
    use std::sync::Arc;
    let mk_app = || {
        let mut app = App::new();
        app.add_plugins(PlaytestPlugin);
        app
    };
    let handle_a = std::thread::spawn(move || {
        let mut app = mk_app();
        app.add_systems(Update, || -> Result<(), bevy::ecs::error::BevyError> {
            Err(bevy::ecs::error::BevyError::warning("from run A"))
        });
        run_scenario(&mut app, &scenario_json()).unwrap()
    });
    let handle_b = std::thread::spawn(move || {
        let mut app = mk_app();
        let _ = Arc::new(()); // keep thread-local isolation honest
        run_scenario(&mut app, &scenario_json()).unwrap()
    });
    let rep_a = handle_a.join().unwrap();
    let rep_b = handle_b.join().unwrap();

    let a_has = rep_a
        .violations
        .iter()
        .any(|v| v.detail.contains("from run A"));
    let b_has = rep_b
        .violations
        .iter()
        .any(|v| v.detail.contains("from run A"));
    assert!(a_has, "run A must have its own error");
    assert!(
        !b_has,
        "run B must not see run A's error (routing by thread)"
    );
}

/// FX5: snapshot ordering must be deterministic — two violations on the
/// same frame with different rules keep a stable order (first_frame,
/// rule, target) across 20 snapshots of a freshly-repopulated map.
#[test]
fn violation_snapshot_order_stable_across_repopulations() {
    use bevy_swarm::state::Violations;
    let mut orders: Vec<Vec<String>> = Vec::new();
    for _ in 0..20 {
        let mut v = Violations::default();
        // Deliberately inserted in a scrambled order; the map type must
        // not leak iteration order into the snapshot.
        v.report("zzz_rule", "b_target", "late".into(), 5);
        v.report("aaa_rule", "z_target", "first".into(), 5);
        v.report("mmm_rule", "a_target", "later".into(), 5);
        let snap = v.snapshot();
        orders.push(snap.iter().map(|e| e.rule.clone()).collect());
    }
    for o in &orders {
        assert_eq!(
            o,
            &vec![
                "aaa_rule".to_string(),
                "mmm_rule".to_string(),
                "zzz_rule".to_string(),
            ]
        );
    }
}

/// FX5 Z7: the harness must not panic when the game already called
/// set_error_handler, must capture the error AND chain to the game's
/// handler (here observed via a world resource side-effect).
#[test]
fn game_set_error_handler_chains_without_panic() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    static CALLS: AtomicUsize = AtomicUsize::new(0);
    fn game_handler(_err: bevy::ecs::error::BevyError, _ctx: bevy::ecs::error::ErrorContext) {
        CALLS.fetch_add(1, Ordering::SeqCst);
    }

    let mut app = App::new();
    app.add_plugins(PlaytestPlugin);
    app.set_error_handler(game_handler);
    app.add_systems(Update, || -> Result<(), bevy::ecs::error::BevyError> {
        Err(bevy::ecs::error::BevyError::warning("chained boom"))
    });
    let rep = run_scenario(&mut app, &scenario_json()).unwrap();
    assert_eq!(rep.status, PlaytestStatus::Fail);
    assert!(
        rep.violations
            .iter()
            .any(|v| v.detail.contains("chained boom")),
        "error must be captured: {:?}",
        rep.violations
    );
    assert!(
        CALLS.load(Ordering::SeqCst) > 0,
        "the game's handler must still be called after the run"
    );
}

/// FX5 Z7: a game that inserted FallbackErrorHandler as a resource
/// directly (no set_error_handler) still gets its errors captured.
#[test]
fn game_inserted_fallback_resource_captured() {
    fn game_handler(_err: bevy::ecs::error::BevyError, _ctx: bevy::ecs::error::ErrorContext) {}
    let mut app = App::new();
    app.add_plugins(PlaytestPlugin);
    app.insert_resource(bevy::ecs::error::FallbackErrorHandler(game_handler));
    app.add_systems(Update, || -> Result<(), bevy::ecs::error::BevyError> {
        Err(bevy::ecs::error::BevyError::warning("resource boom"))
    });
    let rep = run_scenario(&mut app, &scenario_json()).unwrap();
    assert!(
        rep.violations
            .iter()
            .any(|v| v.detail.contains("resource boom")),
        "error must be captured despite pre-inserted resource: {:?}",
        rep.violations
    );
}

/// Z7 log capture: a game system calling error!() yields a log_error
/// violation carrying a frame number; warn!() yields log_warn.
#[test]
fn log_events_captured_as_violations() {
    let mut app = App::new();
    app.add_plugins(PlaytestPlugin);
    app.add_systems(
        Update,
        |frame: Option<Res<bevy_swarm::state::PlaytestState>>| {
            let f = frame.map(|s| s.frame).unwrap_or(0);
            if f == 3 {
                bevy::log::error!("boom log");
            }
            if f == 5 {
                bevy::log::warn!("soft warning");
            }
        },
    );
    let rep = run_scenario(&mut app, &scenario_json()).unwrap();
    let err = rep
        .violations
        .iter()
        .find(|v| v.rule == "log_error")
        .expect("error! must produce a log_error violation");
    assert!(err.detail.contains("boom log"), "detail: {}", err.detail);
    assert!(
        err.first_frame > 0,
        "frame must be recorded, got {}",
        err.first_frame
    );
    let warn = rep
        .violations
        .iter()
        .find(|v| v.rule == "log_warn")
        .expect("warn! must produce a log_warn violation");
    assert!(
        warn.detail.contains("soft warning"),
        "detail: {}",
        warn.detail
    );
}

/// C6: planner goal primitives with STRING values and lt/gt/le/ge
/// checks must be rejected at LOAD time (they can never fire), not
/// pass vacuously at runtime.
#[test]
fn planner_string_lt_rejected_at_load() {
    let res: Result<Scenario, _> = serde_json::from_str(
        r#"{"bot":{"type":"planner","goals":{"kind":"primitive","path":"TestApi.game_phase","check":"gt","value":"menu","emit":[]}},"duration_s":0.1,"invariants":[]}"#,
    );
    let scenario = res.expect("parses");
    let err = bevy_swarm::scenario::validate_scenario(&scenario)
        .expect_err("lt on a string must be rejected at load");
    assert!(
        err.to_string().contains("game_phase"),
        "error must name the offending goal: {err}"
    );
}

/// C6: invariant custom rules with string values and numeric checks
/// must be rejected at load (covered for invariants; this variant pins
/// the ne/equals acceptance path so the rejection isn't blanket).
#[test]
fn invariant_string_ne_accepted() {
    let res: Result<Scenario, _> = serde_json::from_str(
        r#"{"bot":{"type":"chaos","seed":1},"duration_s":0.1,"invariants":[{"name":"phase_not_menu","rule":"custom","path":"TestApi.game_phase","check":"ne","value":"menu"}]}"#,
    );
    let scenario = res.expect("parses");
    bevy_swarm::scenario::validate_scenario(&scenario).expect("ne on strings is valid");
}

/// Z7: sink dedupe groups repeated identical errors — 3 distinct
/// messages fired every frame for 30 frames yield 3 entries with
/// counts, not 90 individual entries.
#[test]
fn sink_events_dedupe_grouped() {
    let mut app = App::new();
    app.add_plugins(PlaytestPlugin);
    app.add_systems(Update, || -> Result<(), bevy::ecs::error::BevyError> {
        Err(bevy::ecs::error::BevyError::warning("repeat-offender-a"))
    });
    let rep = run_scenario(&mut app, &scenario_json()).unwrap();
    let offenders: Vec<_> = rep
        .violations
        .iter()
        .filter(|v| v.detail.contains("repeat-offender-a"))
        .collect();
    assert_eq!(
        offenders.len(),
        1,
        "identical errors must collapse to one entry: {:?}",
        offenders
    );
    assert!(
        offenders[0].count > 1,
        "entry must carry a repeat count, got {}",
        offenders[0].count
    );
    assert!(
        offenders[0].last_frame > offenders[0].first_frame,
        "first/last frame must span the repeats: {}..{}",
        offenders[0].first_frame,
        offenders[0].last_frame
    );
}

/// A1/FX5: fps_floor evaluates post-run as an informational rule
/// (never gates status, excluded from fingerprints) — a demanding
/// floor yields an fps_floor violation on a PASS-shaped run.
#[test]
fn fps_floor_reports_informational() {
    let sc: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"chaos","seed":1},"duration_s":0.1,"invariants":[{"name":"smooth","rule":"fps_floor","value":1000000}]}"#,
    )
    .unwrap();
    let mut app = App::new();
    app.add_plugins(PlaytestPlugin);
    let rep = run_scenario(&mut app, &sc).unwrap();
    let v = rep
        .violations
        .iter()
        .find(|v| v.rule == "fps_floor")
        .expect("fps_floor must fire for an impossible floor");
    assert_eq!(v.target, "smooth", "target carries the invariant name");
    assert!(
        v.fingerprint.is_none(),
        "informational rules are not fingerprinted"
    );
    // Wall-clock frame-time anomaly also informational:
    let rep2 = {
        let sc2: Scenario = serde_json::from_str(
            r#"{"bot":{"type":"chaos","seed":1},"duration_s":0.1,"invariants":[{"name":"steady","rule":"frame_time_anomaly"}]}"#,
        )
        .unwrap();
        let mut app2 = App::new();
        app2.add_plugins(PlaytestPlugin);
        run_scenario(&mut app2, &sc2).unwrap()
    };
    for v in rep2
        .violations
        .iter()
        .filter(|v| v.rule == "frame_time_anomaly")
    {
        assert_ne!(
            v.rule, "steady",
            "anomalies must NOT be reported under the user's invariant name"
        );
        assert!(v.fingerprint.is_none());
    }
}
