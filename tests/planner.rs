//! FX6 B1: planner Select target resolution + violation on miss.

use bevy::prelude::*;
use bevy_swarm::contract::Gameplay;
use bevy_swarm::driver::{run_scenario, PlaytestPlugin};
use bevy_swarm::scenario::Scenario;

fn build_app() -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        TransformPlugin,
        PlaytestPlugin,
        bevy_swarm::contract::TestConventionsPlugin,
    ));
    app
}

/// Planner Select resolving by Name: goal emits Select intent targeting a
/// live named Gameplay entity — no violation, intent emitted.
#[test]
fn fx6_b1_planner_select_by_name_succeeds() {
    let scenario: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"planner","goals":{
            "kind":"primitive","path":"TestApi.score","check":"above","value":9999,
            "emit":[{"intent":"select","target":"player"}]
        }},"duration_s":0.1,"tps":60}"#,
    )
    .unwrap();
    let mut app = build_app();
    app.add_systems(bevy::app::Startup, |mut c: Commands| {
        c.spawn((Gameplay, Name::new("player")));
    });
    let report = run_scenario(&mut app, &scenario).unwrap();
    assert!(
        !report
            .violations
            .iter()
            .any(|v| v.rule == "planner_select_target_missing"),
        "expected no planner_select_target_missing, got: {:?}",
        report.violations
    );
    assert!(
        report.coverage.intents_emitted.contains_key("select"),
        "select intent must be emitted, coverage: {:?}",
        report.coverage.intents_emitted
    );
}

/// Planner Select by StableId: unnamed Gameplay entity works (I1/B1).
#[test]
fn fx6_b1_planner_select_by_stable_id_succeeds() {
    let scenario: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"planner","goals":{
            "kind":"primitive","path":"TestApi.score","check":"above","value":9999,
            "emit":[{"intent":"select","target":{"stable_id":0}}]
        }},"duration_s":0.1,"tps":60}"#,
    )
    .unwrap();
    let mut app = build_app();
    app.add_systems(bevy::app::Startup, |mut c: Commands| {
        c.spawn(Gameplay);
    });
    let report = run_scenario(&mut app, &scenario).unwrap();
    assert!(
        !report
            .violations
            .iter()
            .any(|v| v.rule == "planner_select_target_missing"),
        "expected no planner_select_target_missing, got: {:?}",
        report.violations
    );
    assert!(
        report.coverage.intents_emitted.contains_key("select"),
        "select intent must be emitted, coverage: {:?}",
        report.coverage.intents_emitted
    );
}

/// Planner Select target absent by Name: planner_select_target_missing.
#[test]
fn fx6_b1_planner_select_missing_by_name_reports_violation() {
    let scenario: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"planner","goals":{
            "kind":"primitive","path":"TestApi.score","check":"above","value":9999,
            "emit":[{"intent":"select","target":"ghost"}]
        }},"duration_s":0.1,"tps":60}"#,
    )
    .unwrap();
    let mut app = build_app();
    let report = run_scenario(&mut app, &scenario).unwrap();
    assert!(
        report
            .violations
            .iter()
            .any(|v| v.rule == "planner_select_target_missing"),
        "must report planner_select_target_missing, violations: {:?}",
        report.violations
    );
}

/// Planner Select target absent by StableId: planner_select_target_missing.
#[test]
fn fx6_b1_planner_select_missing_by_stable_id_reports_violation() {
    let scenario: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"planner","goals":{
            "kind":"primitive","path":"TestApi.score","check":"above","value":9999,
            "emit":[{"intent":"select","target":{"stable_id":999}}]
        }},"duration_s":0.1,"tps":60}"#,
    )
    .unwrap();
    let mut app = build_app();
    let report = run_scenario(&mut app, &scenario).unwrap();
    assert!(
        report
            .violations
            .iter()
            .any(|v| v.rule == "planner_select_target_missing"),
        "must report planner_select_target_missing, violations: {:?}",
        report.violations
    );
}
