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
    assert!(
        !v.target.is_empty(),
        "target must carry the system context"
    );
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
    assert!(!b_has, "run B must not see run A's error (routing by thread)");
}
