//! Z1 (swarm-m2h.2): tiered contract — tier detection, checked scope,
//! actuator selection, vacuity guard.

use bevy::prelude::*;
use bevy_swarm::contract::{
    CheckedScope, ContractTier, Gameplay, IntentSurface, SurfaceVariant, TestConventionsPlugin,
};
use bevy_swarm::driver::{run_scenario, PlaytestPlugin, PlaytestReport};
use bevy_swarm::scenario::Scenario;

fn scenario_json(bot: &str) -> Scenario {
    serde_json::from_str(bot).unwrap()
}

/// A tier-0 game: only the game's plugin. No adapter, no IntentSurface,
/// no TestApi, no Gameplay markers. Raw input only.
#[derive(Clone, Default)]
struct TierZeroGamePlugin;

impl Plugin for TierZeroGamePlugin {
    fn build(&self, app: &mut App) {
        // Moves an entity on arrow keys — drivable by raw input (Z3).
        app.add_systems(Update, move_on_arrows);
    }
}

fn move_on_arrows(keys: Res<ButtonInput<KeyCode>>, mut q: Query<&mut Transform>) {
    let mut dir = Vec2::ZERO;
    if keys.pressed(KeyCode::ArrowLeft) {
        dir.x -= 1.0;
    }
    if keys.pressed(KeyCode::ArrowRight) {
        dir.x += 1.0;
    }
    if dir != Vec2::ZERO {
        for mut t in &mut q {
            t.translation.x += dir.x * 0.1;
        }
    }
}

#[test]
fn tier_zero_runs_without_any_contract() {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        bevy::input::InputPlugin,
        TierZeroGamePlugin,
        PlaytestPlugin,
    ));
    app.world_mut().spawn((Gameplay, Transform::default()));

    let scenario =
        scenario_json(r#"{"bot":{"type":"chaos","seed":1},"duration_s":0.2,"invariants":[]}"#);
    let report = run_scenario(&mut app, &scenario).unwrap();

    // Rule 1: no ContractMissing / scenario_error / chaos_bot_config.
    for v in &report.violations {
        assert!(
            v.rule != "chaos_bot_config",
            "tier-0 run must not violate chaos_bot_config: {v:?}"
        );
    }
    assert!(report.error.is_none(), "{:?}", report.error);

    // Tier detection: Zero (no observable/semantic signals).
    assert_eq!(report.scope.tier, Some(ContractTier::Zero));
    // Raw input actuator was selected.
    assert!(
        report.scope.actuators.iter().any(|a| a == "raw_input"),
        "actuators: {:?}",
        report.scope.actuators
    );
    // Semantic invariants inactive with a reason.
    assert!(
        report
            .scope
            .inactive
            .iter()
            .any(|(f, _)| f == "semantic_invariants"),
        "inactive: {:?}",
        report.scope.inactive
    );
    // Liveness IS active.
    assert!(
        report.scope.active.iter().any(|f| f == "liveness"),
        "active: {:?}",
        report.scope.active
    );
}

/// A tier-0 game that IGNORES all input: the vacuity guard must fire
/// when enough actions have no effect.
#[derive(Clone, Default)]
struct UndrivableGamePlugin;

impl Plugin for UndrivableGamePlugin {
    fn build(&self, _app: &mut App) {
        // No systems at all — nothing responds to input.
    }
}

#[test]
fn vacuity_guard_fails_undrivable_game() {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        bevy::input::InputPlugin,
        UndrivableGamePlugin,
        PlaytestPlugin,
    ));
    // Observable gameplay state (so the guard can attribute effects)
    // that the game never changes: input is swallowed.
    app.world_mut().spawn((Gameplay, Transform::default()));

    let scenario = scenario_json(
        r#"{"bot":{"type":"chaos","seed":1,"input_rate_hz":60},"duration_s":0.5,"invariants":[]}"#,
    );
    let report = run_scenario(&mut app, &scenario).unwrap();

    // Z1 rule 3: with >= VACUITY_MIN_ACTIONS (20) injected actions and
    // zero effect, harness_could_not_drive_game fires.
    let vacuous = report
        .violations
        .iter()
        .any(|v| v.rule == "harness_could_not_drive_game");
    assert!(
        vacuous,
        "expected harness_could_not_drive_game; violations: {:?}",
        report.violations
    );
}

/// Semantic detection: TestApi + IntentSurface => Semantic.
#[derive(Clone, Default)]
struct SemanticGamePlugin;

impl Plugin for SemanticGamePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(TestConventionsPlugin);
        app.insert_resource(IntentSurface::new(vec![
            SurfaceVariant::Choice(3),
            SurfaceVariant::Wait,
        ]));
    }
}

#[test]
fn tier_detection_semantic() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, SemanticGamePlugin, PlaytestPlugin));
    app.world_mut()
        .spawn((Name::new("Ball"), Gameplay, Transform::default()));

    let scenario =
        scenario_json(r#"{"bot":{"type":"chaos","seed":1},"duration_s":0.2,"invariants":[]}"#);
    let report = run_scenario(&mut app, &scenario).unwrap();
    assert_eq!(report.scope.tier, Some(ContractTier::Semantic));
}

/// Observable detection: Name on gameplay entity, no semantic contract.
#[derive(Clone, Default)]
struct ObservableGamePlugin;

impl Plugin for ObservableGamePlugin {
    fn build(&self, _app: &mut App) {}
}

#[test]
fn tier_detection_observable() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, ObservableGamePlugin, PlaytestPlugin));
    app.world_mut()
        .spawn((Name::new("Hero"), Gameplay, Transform::default()));

    let scenario =
        scenario_json(r#"{"bot":{"type":"chaos","seed":1},"duration_s":0.2,"invariants":[]}"#);
    let report = run_scenario(&mut app, &scenario).unwrap();
    assert_eq!(report.scope.tier, Some(ContractTier::Observable));
}

/// Resolver-only game (no TestApi resource) is still Semantic.
#[test]
fn tier_detection_resolver_only() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, SemanticGamePlugin, PlaytestPlugin));
    // Remove TestApi but keep a resolver registered by TestConventionsPlugin.
    app.world_mut()
        .remove_resource::<bevy_swarm::contract::TestApi>();

    let scenario =
        scenario_json(r#"{"bot":{"type":"chaos","seed":1},"duration_s":0.2,"invariants":[]}"#);
    let report = run_scenario(&mut app, &scenario).unwrap();
    assert_eq!(
        report.scope.tier,
        Some(ContractTier::Semantic),
        "resolver-based games must detect as Semantic"
    );
}

/// Actuator forcing: "raw" against a game WITH an IntentSurface still
/// uses the raw path (no intent chaos).
#[test]
fn actuator_forced_raw() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, SemanticGamePlugin, PlaytestPlugin));
    app.world_mut()
        .spawn((Name::new("Ball"), Gameplay, Transform::default()));

    let scenario = scenario_json(
        r#"{"bot":{"type":"chaos","seed":1,"actuator":"raw"},"duration_s":0.2,"invariants":[]}"#,
    );
    let report = run_scenario(&mut app, &scenario).unwrap();
    assert!(
        report.scope.actuators.iter().any(|a| a == "raw_input"),
        "actuators: {:?}",
        report.scope.actuators
    );
}

/// Invalid actuator value rejects at load time (C6).
#[test]
fn actuator_invalid_rejects_at_load() {
    let result: Result<Scenario, _> = serde_json::from_str(
        r#"{"bot":{"type":"chaos","actuator":"voice"},"duration_s":0.2,"invariants":[]}"#,
    );
    // Deserializes fine (field is a String); validation happens in
    // validate_scenario.
    let scenario = result.unwrap();
    let err = bevy_swarm::scenario::validate_scenario(&scenario);
    assert!(err.is_err(), "invalid actuator must reject at load");
}

/// The walker-with-keyboard case from the bead text: a clean game with
/// raw input must NOT fire the vacuity guard.
#[derive(Clone, Default)]
struct CleanWalkerPlugin;

impl Plugin for CleanWalkerPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, move_on_arrows);
    }
}

#[test]
fn vacuity_guard_quiet_on_drivable_game() {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        bevy::input::InputPlugin,
        CleanWalkerPlugin,
        PlaytestPlugin,
    ));
    app.world_mut().spawn((Gameplay, Transform::default()));

    let scenario = scenario_json(
        r#"{"bot":{"type":"chaos","seed":1,"input_rate_hz":60},"duration_s":0.5,"invariants":[]}"#,
    );
    let report = run_scenario(&mut app, &scenario).unwrap();
    let vacuous = report
        .violations
        .iter()
        .any(|v| v.rule == "harness_could_not_drive_game");
    assert!(
        !vacuous,
        "drivable game must not fire the vacuity guard; violations: {:?}",
        report.violations
    );
}

/// Scope is present in every report (crash path included).
#[test]
fn scope_present_on_scenario_error() {
    let rep = PlaytestReport::scenario_error("bad scenario");
    let json = serde_json::to_value(&rep).unwrap();
    assert!(json.get("scope").is_some(), "scope must serialize always");
}

/// CheckedScope serializes with snake_case tier names (U1 vocabulary).
#[test]
fn scope_serializes_snake_case() {
    let scope = CheckedScope {
        tier: Some(ContractTier::Semantic),
        active: vec!["liveness".into()],
        inactive: vec![("semantic_invariants".into(), "reason".into())],
        actuators: vec!["intents".into()],
    };
    let json = serde_json::to_value(&scope).unwrap();
    assert_eq!(json["tier"], "semantic");
}

/// Reports validate against the report schema including scope.
#[test]
fn report_schema_includes_scope() {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        bevy::input::InputPlugin,
        TierZeroGamePlugin,
        PlaytestPlugin,
    ));
    let scenario =
        scenario_json(r#"{"bot":{"type":"chaos","seed":1},"duration_s":0.2,"invariants":[]}"#);
    let report = run_scenario(&mut app, &scenario).unwrap();
    let json = serde_json::to_value(&report).unwrap();
    let schema: serde_json::Value =
        serde_json::from_str(include_str!("../docs/report-schema.json")).unwrap();
    let validator = jsonschema::validator_for(&schema).unwrap();
    let errors: Vec<String> = validator
        .iter_errors(&json)
        .map(|e| e.to_string())
        .collect();
    assert!(errors.is_empty(), "schema errors: {errors:?}");
}
