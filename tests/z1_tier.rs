//! Z1 (swarm-m2h.2): tiered contract — tier detection, checked scope,
//! actuator selection, vacuity guard.

use bevy::prelude::*;
use bevy_playtest::contract::{
    CheckedScope, ContractTier, Gameplay, IntentSurface, SurfaceVariant, TestConventionsPlugin,
};
use bevy_playtest::driver::{run_scenario, PlaytestPlugin, PlaytestReport};
use bevy_playtest::enums::PlaytestStatus;
use bevy_playtest::scenario::Scenario;

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
    // le3.22 (RF26): the vacuity guard must affect STATUS (computed
    // after the guard, not before) and carry a fingerprint (was None,
    // excluded from autotest aggregation).
    assert_eq!(
        report.status,
        PlaytestStatus::Fail,
        "vacuity guard must gate status"
    );
    let entry = report
        .violations
        .iter()
        .find(|v| v.rule == "harness_could_not_drive_game")
        .unwrap();
    assert!(
        entry.fingerprint.is_some(),
        "vacuity guard entry must be fingerprinted"
    );
}

/// le3.22 (RF26): a game frozen from FRAME 0 (never live after
/// readiness) with inputs ATTEMPTED must report the GATING
/// frozen_world rule — not be downgraded to informational
/// undrivable_game. The old downgrade triggered on effect_rate==0
/// regardless of whether the world was ever alive, hiding startup
/// deadlocks behind a status Pass.
#[test]
fn frozen_from_frame_0_with_inputs_fails_not_downgraded() {
    // A game with observable gameplay entities that NEVER change and
    // never respond to inputs (startup deadlock shape).
    #[derive(Clone, Default)]
    struct DeadGamePlugin;
    impl Plugin for DeadGamePlugin {
        fn build(&self, _app: &mut App) {}
    }
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        bevy::input::InputPlugin,
        bevy::transform::TransformPlugin,
        DeadGamePlugin,
        PlaytestPlugin,
    ));
    // IntentSurface so the chaos bot ATTEMPTS inputs; the game
    // ignores them all (effect rate 0).
    app.insert_resource(IntentSurface::new(vec![SurfaceVariant::Move]));
    // Observable gameplay state that never changes.
    app.world_mut().spawn((Gameplay, Transform::default()));

    let scenario = scenario_json(
        r#"{"bot":{"type":"chaos","seed":1,"input_rate_hz":60},"duration_s":1.5,"invariants":[],"liveness":{"timeout_s":0.2}}"#,
    );
    let report = run_scenario(&mut app, &scenario).unwrap();
    let entry = report
        .violations
        .iter()
        .find(|v| v.rule == "frozen_world" || v.rule == "undrivable_game")
        .expect("expected a freeze violation");
    assert_eq!(
        entry.rule, "frozen_world",
        "a world never live after readiness must stay frozen_world, not be downgraded to undrivable_game"
    );
    assert_ne!(
        report.status,
        PlaytestStatus::Pass,
        "a frozen-from-frame-0 world with inputs attempted must not Pass; violations: {:?}",
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
        .remove_resource::<bevy_playtest::contract::TestApi>();

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
    let err = bevy_playtest::scenario::validate_scenario(&scenario);
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

/// le3.23 (RF27): same-value-write blindness — a system that assigns
/// the SAME VALUE every frame (no actual change) must still report
/// frozen_world. The old change-tick scan counted the WRITE as "alive"
/// even though the value never changed.
#[test]
fn same_value_write_fails_not_alive() {
    // A game with a Gameplay entity whose Transform is reset to the
    // SAME value every frame — no actual movement, just a write.
    #[derive(Clone, Default)]
    struct StaticGamePlugin;
    impl Plugin for StaticGamePlugin {
        fn build(&self, app: &mut App) {
            app.add_systems(Update, |mut q: Query<&mut Transform>| {
                for mut t in q.iter_mut() {
                    *t = Transform::IDENTITY; // Same value every frame
                }
            });
        }
    }
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        bevy::input::InputPlugin,
        bevy::transform::TransformPlugin,
        StaticGamePlugin,
        PlaytestPlugin,
    ));
    // IntentSurface so chaos bot attempts inputs (the game ignores them).
    app.insert_resource(IntentSurface::new(vec![SurfaceVariant::Move]));
    // Spawn a Gameplay entity that gets reset to Identity every frame.
    app.world_mut()
        .spawn((Gameplay, Name::new("Static"), Transform::default()));

    let scenario = scenario_json(
        r#"{"bot":{"type":"chaos","seed":1,"input_rate_hz":60},"duration_s":1.5,"invariants":[],"liveness":{"timeout_s":0.2}}"#,
    );
    let report = run_scenario(&mut app, &scenario).unwrap();
    let has_freeze = report.violations.iter().any(|v| v.rule == "frozen_world");
    assert!(
        has_freeze,
        "a world with same-value rewrites (no actual change) must report frozen_world; violations: {:?}",
        report.violations
    );
    assert_ne!(
        report.status,
        PlaytestStatus::Pass,
        "same-value writes must not keep a frozen world alive; violations: {:?}",
        report.violations
    );
}

/// le3.23 (RF27): reject NaN/negative liveness timeouts at load time.
#[test]
fn liveness_timeout_rejects_nan_negative() {
    let bad_nan: Result<Scenario, _> = serde_json::from_str(
        r#"{"bot":{"type":"chaos","seed":1},"duration_s":0.5,"invariants":[],"liveness":{"timeout_s":NaN}}"#,
    );
    assert!(bad_nan.is_err(), "NaN timeout must reject at deserialize");
    let bad_neg: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"chaos","seed":1},"duration_s":0.5,"invariants":[],"liveness":{"timeout_s":-1.0}}"#,
    ).unwrap();
    let err = bevy_playtest::scenario::validate_scenario(&bad_neg);
    assert!(
        err.is_err(),
        "negative timeout must reject at validate_scenario"
    );
}
