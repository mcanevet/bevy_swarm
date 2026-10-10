//! I2: compiled invariants — load-time rejection, archetype-level counts.

use bevy::prelude::*;
use bevy_swarm::contract::Gameplay;
use bevy_swarm::driver::{run_scenario, PlaytestPlugin};
use bevy_swarm::scenario::{Scenario, ScenarioError};

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
fn unknown_component_rejected_at_load() {
    // A typo'd component in a query invariant must reject at LOAD time,
    // before any frame runs (I2), with ScenarioError::InvalidComponent.
    let scenario: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"replay","inputs":[]},"duration_s":0.1,"invariants":[
            {"name":"typo","rule":"custom","query":{"with":["DefinitelyNotAType"]},
             "check":"equals","value":0}
        ]}"#,
    )
    .unwrap();
    let mut app = build_app();
    let err = run_scenario(&mut app, &scenario).unwrap_err();
    match err {
        ScenarioError::InvalidComponent { name, reason } => {
            assert_eq!(name, "DefinitelyNotAType");
            assert!(reason.contains("not registered"), "{}", reason);
        }
        other => panic!("expected InvalidComponent, got: {:?}", other),
    }
}

#[test]
fn count_is_archetype_level() {
    // 10k entities across 2 archetypes: the compiled count must equal
    // the number of Gameplay entities and must not scale with entity
    // count (archetype-length summation, no per-entity iteration).
    let scenario: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"replay","inputs":[]},"duration_s":0.1,"invariants":[
            {"name":"gcount","rule":"custom","query":{"with":["Name"],"without":[]},
             "check":"equals","value":10000}
        ]}"#,
    )
    .unwrap();
    let mut app = build_app();
    app.add_systems(bevy::app::Startup, |mut commands: Commands| {
        for i in 0..10_000 {
            // Two archetypes: half with Transform, half without.
            if i % 2 == 0 {
                commands.spawn((Gameplay, Name::new(format!("e{}", i))));
            } else {
                commands.spawn((Gameplay, Name::new(format!("e{}", i)), Transform::default()));
            }
        }
    });
    let rep = run_scenario(&mut app, &scenario).unwrap();
    assert!(
        !rep.violations.iter().any(|v| v.rule == "gcount"),
        "count invariant failed: {:?}",
        rep.violations
    );
}

#[test]
fn lazy_registered_component_count_works() {
    // A component type whose storage only registers at Startup (spawned
    // there, never present when run_scenario compiles) must still count
    // correctly at runtime via lazy id resolution.
    #[derive(Component, Reflect, Default)]
    #[reflect(Component)]
    struct SpawnedAtStartup;

    let scenario: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"replay","inputs":[]},"duration_s":0.1,"invariants":[
            {"name":"late_count","rule":"custom","query":{"with":["SpawnedAtStartup"]},
             "check":"equals","value":5}
        ]}"#,
    )
    .unwrap();
    let mut app = build_app();
    app.register_type::<SpawnedAtStartup>();
    app.add_systems(bevy::app::Startup, |mut commands: Commands| {
        for _ in 0..5 {
            commands.spawn((Gameplay, SpawnedAtStartup, Transform::default()));
        }
    });
    let rep = run_scenario(&mut app, &scenario).unwrap();
    assert!(
        !rep.violations.iter().any(|v| v.rule == "late_count"),
        "lazy count failed: {:?}",
        rep.violations
    );
}

/// I2: Custom query-count invariant without a numeric value must reject at LOAD.
#[test]
fn custom_query_count_requires_numeric_value() {
    // Missing value field — should reject with ScenarioError::InvalidInvariant.
    let scenario: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"replay","inputs":[]},"duration_s":0.1,"invariants":[
            {"name":"cnt","rule":"custom","query":{"with":["Name"],"without":[]},
             "check":"ge"}
        ]}"#,
    )
    .unwrap();
    let mut app = build_app();
    let err = run_scenario(&mut app, &scenario).unwrap_err();
    match &err {
        ScenarioError::Rejected(reason) => {
            assert!(reason.contains("numeric value"), "{}", reason);
        }
        other => panic!("expected Rejected, got: {:?}", other),
    }
}

/// I2: a registered-but-never-spawned component counts as 0 — NOT an
/// "unresolved component type" violation (load-time checks already
/// rejected true typos).
#[test]
fn never_spawned_component_counts_as_zero() {
    #[derive(bevy::reflect::Reflect, bevy::prelude::Component)]
    #[reflect(Component)]
    struct NeverSpawned;

    let scenario: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"replay","inputs":[]},"duration_s":0.1,"invariants":[
            {"name":"zero","rule":"custom","query":{"with":["NeverSpawned"],"without":[]},
             "check":"equals","value":0}
        ]}"#,
    )
    .unwrap();
    let mut app = build_app();
    app.register_type::<NeverSpawned>();
    let result = run_scenario(&mut app, &scenario);
    match result {
        Ok(report) => {
            assert!(
                !report.violations.iter().any(|v| v.rule == "zero"),
                "never-spawned component must count as 0, got violations: {:?}",
                report.violations
            );
        }
        Err(e) => panic!("scenario must run, got: {e:?}"),
    }
}
