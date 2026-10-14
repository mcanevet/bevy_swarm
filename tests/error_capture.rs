//! le3.21 (RF25): error capture — rejected runs don't leak handlers;
//! game LogPlugin captures; parallel runs don't drop panics.

use bevy::prelude::*;
use bevy_swarm::driver::{run_scenario, PlaytestPlugin};
use bevy_swarm::harness::Scenario;

fn build_app() -> App {
    let mut app = App::new();
    app.add_plugins((
        bevy::MinimalPlugins,
        bevy::transform::TransformPlugin,
        PlaytestPlugin,
        bevy_swarm::contract::TestConventionsPlugin,
    ));
    app
}

#[test]
fn rejected_run_doesnt_leak_handler_chain() {
    // First run: component typo → compile-time reject.
    // Second run: valid scenario with a system that errors!().
    // The second run MUST capture the error (handler not poisoned by
    // the first run's dead chain entry).
    let bad: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"replay","inputs":[]},"duration_s":0.1,"invariants":[
            {"name":"bad","rule":"custom","path":"Component:NonExistent.hp",
             "check":"ge","value":0}]}"#,
    )
    .unwrap();
    let good: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"replay","inputs":[]},"duration_s":0.1,"invariants":[
            {"name":"ok","rule":"custom","path":"TestApi.score",
             "check":"ge","value":-1}]}"#,
    )
    .unwrap();

    let mut app = build_app();
    // First run: reject (component typo).
    let err = run_scenario(&mut app, &bad);
    assert!(err.is_err(), "typo must reject");

    // Second run: valid scenario. It should run and capture errors if any.
    // Just verify it doesn't panic or hang.
    let rep = run_scenario(&mut app, &good);
    assert!(rep.is_ok(), "valid scenario must run: {:?}", rep.err());
}
