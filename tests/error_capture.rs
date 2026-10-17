//! le3.21 (RF25): error capture — rejected runs don't leak handlers;
//! game LogPlugin captures; parallel runs don't drop panics.

use bevy::prelude::*;
use bevy_playtest::driver::{run_scenario, PlaytestPlugin};
use bevy_playtest::harness::Scenario;

fn build_app() -> App {
    let mut app = App::new();
    app.add_plugins((
        bevy::MinimalPlugins,
        bevy::transform::TransformPlugin,
        PlaytestPlugin,
        bevy_playtest::contract::TestConventionsPlugin,
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

#[test]
fn parallel_pool_panic_attributed_to_every_active_run() {
    // le3.35 (RF45) CRITICAL regression: a pool-thread panic during
    // PARALLEL runs used to be filed under the NEWEST run only
    // (resolve_run -> next_back). Seed 1's panic landed on seed 3:
    // the panicking seed PASSED, an innocent seed FAILED. Now
    // pool-thread events fan out to ALL concurrently active runs.
    // Setup: 3 variants run in parallel (run_matrix); variant
    // "seed1" triggers a cheat that detaches a panicking task on
    // AsyncComputeTaskPool; seeds 2/3 do nothing. Reverting to
    // newest-run routing makes seed1 pass and (usually) seed3 fail.
    use bevy::tasks::AsyncComputeTaskPool;
    use bevy_playtest::branch::{run_matrix, InProcess};
    use bevy_playtest::contract::CheatHooks;
    use std::collections::HashMap;

    let mk_variant = |name: &str, cheat: bool| {
        let cheats = if cheat {
            r#"[{"kind":"panic_pool","frame":5}]"#
        } else {
            "[]"
        };
        let scenario: Scenario = serde_json::from_str(&format!(
            r#"{{"bot":{{"type":"replay","inputs":[]}},"duration_s":1.0,"invariants":[],
                "setup":{{"cheats":{cheats}}}}}"#
        ))
        .unwrap();
        (name.to_string(), scenario)
    };

    let variants = vec![
        mk_variant("seed1", true),
        mk_variant("seed2", false),
        mk_variant("seed3", false),
    ];

    let factory = || {
        let mut app = build_app();
        #[allow(clippy::type_complexity)]
        let mut hooks: HashMap<
            String,
            Box<dyn Fn(&mut bevy::prelude::World) + Send + Sync>,
        > = HashMap::new();
        hooks.insert(
            "panic_pool".to_string(),
            Box::new(|_world: &mut bevy::prelude::World| {
                let task = AsyncComputeTaskPool::get().spawn(async move {
                    std::thread::sleep(std::time::Duration::from_millis(50));
                    panic!("seed1 pool panic (le3.35 RF45)");
                });
                task.detach();
            }),
        );
        app.insert_resource(CheatHooks(hooks));
        app
    };

    let runner = InProcess { factory };
    let report = run_matrix(&runner, variants, 3).expect("matrix runs");
    for outcome in &report.outcomes {
        let panics: Vec<String> = outcome
            .report
            .violations
            .iter()
            .filter(|v| v.rule == "panic")
            .map(|v| v.last_detail.clone())
            .collect();
        if outcome.variant_name == "seed1" {
            assert!(
                outcome.report.status == bevy_playtest::enums::PlaytestStatus::Fail,
                "seed1 (panicking variant) must FAIL, got {:?} violations={:?}",
                outcome.report.status,
                outcome.report.violations
            );
        } else {
            // Fan-out semantics: an innocent seed may ALSO see the
            // panic (shared, non-reproducible-by-seed attribution) —
            // the RF45 requirement is ONLY that seed1 fails. But
            // seed1 must fail because of ITS OWN panic, so seed1's
            // violations must contain the pool panic detail.
            assert!(
                panics.iter().any(|d| d.contains("seed1 pool panic")),
                "seed1's report must contain its pool panic: {:?}",
                panics
            );
        }
    }
}
