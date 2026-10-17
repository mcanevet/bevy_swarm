//! Z1 (swarm-m2h.2, consolidated amendment 2): artifact-level tier
//! fidelity. The STANDALONE fixture crate `fixture_lone` (no adapter
//! plugin, no bevy_playtest dep) is run at each tier; report.json must
//! carry contract_tier and an ACCURATE unchecked scope. Adding each
//! contract piece moves the tier; inactive oracle families are listed
//! with reasons.

use bevy::prelude::*;
use bevy_playtest::contract::{ContractTier, IntentSurface, SurfaceVariant, TestConventionsPlugin};
use bevy_playtest::driver::{run_scenario, PlaytestPlugin, PlaytestReport};
use bevy_playtest::scenario::Scenario;

fn run_matrix(app: &mut App) -> PlaytestReport {
    let scenario: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"chaos","seed":7},"duration_s":0.3,"invariants":[]}"#,
    )
    .unwrap();
    run_scenario(app, &scenario).unwrap()
}

fn base_app() -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        bevy::transform::TransformPlugin,
        bevy::input::InputPlugin,
        PlaytestPlugin,
    ));
    app
}

/// Tier ZERO: the standalone game plugin, nothing else. The harness
/// drives it via raw input (arrows) and reports tier zero.
#[test]
fn matrix_standalone_fixture_reports_zero() {
    let mut app = base_app();
    app.add_plugins(fixture_lone::LoneGamePlugin);
    let report = run_matrix(&mut app);
    // The fixture declares NO harness-visible contract (its own
    // Gameplay component is not the harness marker; no GameTypes
    // resource): the detected tier must be exactly Zero.
    assert_eq!(
        report.scope.tier,
        Some(ContractTier::Zero),
        "zero-contract fixture must report tier zero"
    );
    // No chaos_bot_config: the game runs without any adapter.
    for v in &report.violations {
        assert_ne!(v.rule, "chaos_bot_config", "unexpected: {v:?}");
    }
    // Raw actuator was used (no IntentSurface).
    assert!(
        report.scope.actuators.iter().any(|a| a == "raw_input"),
        "actuators: {:?}",
        report.scope.actuators
    );
    // And semantic invariants are listed inactive with a reason.
    assert!(
        report
            .scope
            .inactive
            .iter()
            .any(|(f, r)| f == "semantic_invariants" && !r.is_empty()),
        "inactive: {:?}",
        report.scope.inactive
    );
}

/// Adding the semantic contract (IntentSurface) moves the tier to
/// Semantic and switches the actuator to intents.
#[test]
fn matrix_adding_contract_pieces_moves_tier() {
    let mut app = base_app();
    app.add_plugins((fixture_lone::LoneGamePlugin, TestConventionsPlugin));
    app.insert_resource(IntentSurface::new(vec![SurfaceVariant::Wait]));
    let report = run_matrix(&mut app);
    assert_eq!(
        report.scope.tier,
        Some(ContractTier::Semantic),
        "adding IntentSurface must move the tier"
    );
    assert!(
        report.scope.actuators.iter().any(|a| a == "intents"),
        "actuators: {:?}",
        report.scope.actuators
    );
    // And the previously-inactive family is no longer listed.
    assert!(
        !report
            .scope
            .inactive
            .iter()
            .any(|(f, _)| f == "semantic_invariants"),
        "inactive: {:?}",
        report.scope.inactive
    );
}

/// The report artifact (serialized report.json) carries contract_tier
/// and an accurate unchecked scope — black-box view of the artifact.
#[test]
fn matrix_artifact_carries_tier_and_scope() {
    let mut app = base_app();
    app.add_plugins(fixture_lone::LoneGamePlugin);
    let report = run_matrix(&mut app);
    let json = serde_json::to_value(&report).unwrap();
    let tier = json["scope"]["tier"].as_str().expect("scope.tier string");
    assert!(
        ["zero", "observable", "semantic"].contains(&tier),
        "artifact tier must use U1 vocabulary, got {tier}"
    );
    let inactive = json["scope"]["inactive"]
        .as_array()
        .expect("inactive array");
    assert!(
        inactive
            .iter()
            .all(|e| e.as_array().is_some_and(|pair| pair.len() == 2)),
        "every inactive family must carry a reason: {inactive:?}"
    );
    // Validated against the published schema.
    let schema: serde_json::Value =
        serde_json::from_str(include_str!("../docs/report-schema.json")).unwrap();
    let validator = jsonschema::validator_for(&schema).unwrap();
    let errors: Vec<String> = validator
        .iter_errors(&json)
        .map(|e| e.to_string())
        .collect();
    assert!(errors.is_empty(), "schema errors: {errors:?}");
}

/// Observer-level addition: a Name on an explicitly-Gameplay entity.
#[test]
fn matrix_observable_signal() {
    let mut app = base_app();
    app.add_plugins(fixture_lone::LoneGamePlugin);
    // The fixture already spawns Name + its own Gameplay marker
    // component (fixture_lone::Gameplay is NOT the harness marker) —
    // spawn a harness-tagged entity to confirm Observable detection.
    app.world_mut()
        .spawn((Name::new("Tagged"), bevy_playtest::contract::Gameplay));
    let report = run_matrix(&mut app);
    assert_eq!(report.scope.tier, Some(ContractTier::Observable));
}
