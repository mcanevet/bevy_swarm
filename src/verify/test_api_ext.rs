use crate::contract::*;
use crate::enums::PlaytestStatus;
use crate::harness::*;
use bevy::prelude::*;

#[allow(unused_imports)]
use super::fixtures::*;

// ---------------------------------------------------------------------------
// D1: resolve_path + optional type-erased TestApi resolver
// ---------------------------------------------------------------------------

/// A game-defined TestApi type entirely distinct from the crate's.
#[derive(Resource, Default)]
struct InventoryApi {
    gold: i64,
    gems: i64,
}
impl crate::contract::TestApiResolve for InventoryApi {
    fn resolve(&self, path: &str) -> Option<crate::contract::TestFieldValue> {
        match path.strip_prefix("TestApi.")? {
            "gold" => Some(crate::contract::TestFieldValue::Numeric(self.gold as f64)),
            "gems" => Some(crate::contract::TestFieldValue::Numeric(self.gems as f64)),
            _ => None,
        }
    }
}

#[test]
fn custom_test_api_type_works() {
    // A game with its OWN TestApi resource type: inventory resolver
    // registered via register_test_api; invariant on TestApi.gold works
    // WITHOUT the crate's TestApi resource.
    let mut app = build_app_no_scoring();
    app.init_resource::<InventoryApi>()
        .register_test_api::<InventoryApi>();
    app.add_systems(
        bevy::app::Update,
        |mut api: bevy::ecs::system::ResMut<InventoryApi>| {
            api.gold += 1; // grows past any ceiling -> violation
        },
    );
    let scen: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"chaos","seed":1},"duration_s":0.3,"invariants":[{"name":"gold_cap","rule":"custom","path":"TestApi.gold","check":"above","value":3}]}"#,
    )
    .unwrap();
    let rep = run_scenario(&mut app, &scen).unwrap();
    assert!(
        rep.violations.iter().any(|v| v.rule == "gold_cap"),
        "custom resolver not used: {:#?}",
        rep.violations
    );
}

#[test]
fn testapi_path_without_resolver_is_load_error() {
    // No resolver, no crate TestApi resource, but the scenario uses a
    // TestApi.* path -> structured load error with guidance.
    let mut app = bevy::app::App::new();
    app.add_plugins((bevy::MinimalPlugins, crate::harness::PlaytestPlugin));
    // NOTE: no TestConventionsPlugin -> no default resolver.
    let scen: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"chaos","seed":1},"duration_s":0.2,"invariants":[{"name":"cap","rule":"custom","path":"TestApi.score","check":"above","value":3}]}"#,
    )
    .unwrap();
    match run_scenario(&mut app, &scen) {
        Err(crate::harness::ScenarioError::ContractMissing(msg)) => {
            assert!(msg.contains("resolver"), "{}", msg);
        }
        other => panic!("expected ContractMissing, got {:?}", other.map(|_| ())),
    }
}

#[test]
fn requires_path_uses_percepts() {
    // requires_path now resolves through resolve_path: world percepts
    // (Resource:) work for expert-rule REQUIRE clauses without any
    // TestApi resolver at all.
    #[derive(Resource, Reflect, Default)]
    #[reflect(Resource)]
    struct Policy {
        health: f64,
    }
    let mut app = build_app_no_scoring();
    app.init_resource::<Policy>();
    {
        use bevy::ecs::reflect::AppTypeRegistry;
        let registry = app.world_mut().resource_mut::<AppTypeRegistry>();
        registry.0.write().register::<Policy>();
    }
    // Fixtures have the default TestApi (resolver present via
    // TestConventionsPlugin); this run proves percepts ALSO work for
    // requires_path: health=10 violates requires below 5.
    app.world_mut().resource_mut::<Policy>().health = 10.0;
    let scen: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"chaos","seed":1},"duration_s":0.3,"invariants":[{"name":"heal_policy","rule":"custom","path":"TestApi.score","check":"below","value":9999,"requires_path":"Resource:Policy.health","requires_check":"below","requires_value":5}]}"#,
    )
    .unwrap();
    let rep = run_scenario(&mut app, &scen).unwrap();
    assert!(
        rep.violations.iter().any(|v| v.rule == "heal_policy"),
        "requires_path did not use percepts: {:#?}",
        rep.violations
    );
}

// ---------------------------------------------------------------------------
// H1: GlobalTransform, full NaN check, plugin finish/cleanup, name addressing
// ---------------------------------------------------------------------------

#[test]
fn bounds_use_global_transform() {
    // A CHILD at local (1,0,0) under a parent moved to x=20_000: the
    // child's WORLD position is out of bounds but its LOCAL Transform
    // looks fine. The oracle must flag the child (previously silently
    // passed with the local-only check).
    let scen: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"chaos","seed":3},"duration_s":0.3,"invariants":[
            {"name":"in_bounds","rule":"nodes_in_bounds",
             "min_x":-100,"max_x":100,"min_y":-100,"max_y":100}
        ],"setup":{}}"#,
    )
    .unwrap();
    let mut app = build_app_no_scoring();
    app.add_systems(
        bevy::app::Update,
        |mut commands: bevy::ecs::system::Commands| {
            static SPAWNED: std::sync::atomic::AtomicBool =
                std::sync::atomic::AtomicBool::new(false);
            if !SPAWNED.swap(true, std::sync::atomic::Ordering::SeqCst) {
                let child = commands
                    .spawn((
                        bevy::prelude::Name::new("Turret"),
                        crate::harness::Gameplay,
                        bevy::prelude::Transform::from_xyz(1.0, 0.0, 0.0),
                    ))
                    .id();
                commands
                    .spawn((
                        bevy::prelude::Name::new("Tank"),
                        crate::harness::Gameplay,
                        bevy::prelude::Transform::from_xyz(20_000.0, 0.0, 0.0),
                    ))
                    .add_child(child);
            }
        },
    );
    let rep = run_scenario(&mut app, &scen).unwrap();
    assert!(
        rep.violations.iter().any(|v| v.rule == "in_bounds"),
        "child out of bounds via parent not detected: {:?}",
        rep.violations
    );
}

#[test]
fn nan_rotation_is_detected() {
    // NaN quaternion (normalising a zero vector): the finite check must
    // catch rotation, not only translation. Separate rule for
    // denormalized-but-finite quats.
    let scen: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"chaos","seed":3},"duration_s":0.3,"invariants":[],"setup":{}}"#,
    )
    .unwrap();
    let mut app = build_app_no_scoring();
    let rep = run_scenario(&mut app, &scen).unwrap();
    // sanity: control run passes
    assert_eq!(rep.status, PlaytestStatus::Pass);

    let mut app2 = build_app_no_scoring();
    app2.add_systems(
        bevy::app::Update,
        |mut q: bevy::ecs::system::Query<
            '_,
            '_,
            &mut bevy::prelude::Transform,
            With<crate::harness::Gameplay>,
        >| {
            for mut t in &mut q {
                t.rotation = bevy::math::Quat::from_xyzw(f32::NAN, 0.0, 0.0, 1.0);
            }
        },
    );
    let rep2 = run_scenario(&mut app2, &scen).unwrap();
    assert!(
        rep2.violations
            .iter()
            .any(|v| v.rule == "finite_transforms" && v.detail.contains("rotation")),
        "NaN rotation not detected: {:?}",
        rep2.violations
    );
}

#[test]
fn unnormalized_rotation_is_separate_rule() {
    // Denormalized (finite) quat → nodes_rotation_unnormalized, NOT nodes_finite.
    let scen: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"chaos","seed":3},"duration_s":0.3,"invariants":[],"setup":{}}"#,
    )
    .unwrap();
    let mut app = build_app_no_scoring();
    app.add_systems(
        bevy::app::Update,
        |mut q: bevy::ecs::system::Query<
            '_,
            '_,
            &mut bevy::prelude::Transform,
            With<crate::harness::Gameplay>,
        >| {
            for mut t in &mut q {
                t.rotation = bevy::math::Quat::from_xyzw(0.5, 0.0, 0.0, 0.5);
            }
        },
    );
    let rep = run_scenario(&mut app, &scen).unwrap();
    assert!(
        rep.violations
            .iter()
            .any(|v| v.rule == "nodes_rotation_unnormalized"),
        "denormalized rotation not flagged: {:?}",
        rep.violations
    );
    assert!(
        !rep.violations
            .iter()
            .any(|v| v.rule == "finite_transforms" && v.detail.contains("rotation")),
        "denormalized rotation must not be a nodes_finite hit"
    );
}

#[test]
fn plugin_finish_runs_before_scenarios() {
    // A plugin whose finish() inserts a resource a game system reads:
    // run_scenario must finish plugin building or the system panics.
    #[derive(bevy::prelude::Resource)]
    struct LateResource;
    struct LatePlugin;
    impl bevy::app::Plugin for LatePlugin {
        fn build(&self, _app: &mut App) {}
        fn finish(&self, app: &mut App) {
            app.insert_resource(LateResource);
        }
    }
    let mut app = build_app_no_scoring();
    app.add_plugins(LatePlugin);
    app.add_systems(
        bevy::app::Update,
        |_late: bevy::ecs::system::Res<LateResource>| {},
    );
    let scen: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"chaos","seed":3},"duration_s":0.2,"invariants":[],"setup":{}}"#,
    )
    .unwrap();
    let rep = run_scenario(&mut app, &scen).unwrap();
    assert_eq!(rep.status, PlaytestStatus::Pass);
}

#[test]
fn percept_name_addressing_survives_despawns() {
    // Two entities with a component; despawn the first: {Name}
    // addressing keeps reading the survivor, positional [idx] does not.
    #[derive(bevy::prelude::Component, bevy::prelude::Reflect, Default)]
    #[reflect(Component)]
    struct Health(f64);

    #[derive(bevy::prelude::Resource, Default)]
    struct FirstEntity(Option<bevy::ecs::entity::Entity>);
    let mut app = build_app_no_scoring();
    app.init_resource::<FirstEntity>();
    app.add_systems(
        bevy::app::Update,
        |mut commands: bevy::ecs::system::Commands,
         mut first: bevy::ecs::system::ResMut<FirstEntity>| {
            static SPAWNED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let n = SPAWNED.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            match n {
                0 => {
                    first.0 = Some(
                        commands
                            .spawn((
                                bevy::prelude::Name::new("First"),
                                crate::harness::Gameplay,
                                Health(1.0),
                            ))
                            .id(),
                    );
                    commands.spawn((
                        bevy::prelude::Name::new("Second"),
                        crate::harness::Gameplay,
                        Health(2.0),
                    ));
                }
                5 => {
                    // Despawn First (frame 5): index-based reads flip.
                    if let Some(e) = first.0.take() {
                        commands.entity(e).despawn();
                    }
                }
                _ => {}
            }
        },
    );
    {
        use bevy::ecs::reflect::AppTypeRegistry;
        let registry = app.world_mut().resource_mut::<AppTypeRegistry>();
        registry.0.write().register::<Health>();
    }
    // Run enough updates for the spawn AND the frame-5 despawn of First.
    for _ in 0..8 {
        app.update();
    }
    // Name addressing reads Second regardless of spawn/despawn order.
    let v = crate::oracles::resolve_world_percept(app.world(), "Component:Health{Second}.0");
    assert_eq!(
        v,
        Some(crate::contract::TestFieldValue::Numeric(2.0)),
        "{{Name}} addressing lost the survivor"
    );
}
