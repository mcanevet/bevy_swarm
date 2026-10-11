//! FX6 moderates pt3: aggregates in world percepts, digest coverage for
//! unnamed entities, audit-log replay detail comparison.

use bevy::prelude::*;
use bevy_swarm::oracles::resolve_world_percept;

#[derive(Component, Reflect, Default)]
#[reflect(Component)]
struct Health(f32);

#[derive(Component, Reflect, Default)]
#[reflect(Component)]
struct Unnamed;

#[derive(Resource, Reflect, Default)]
#[reflect(Resource)]
struct ScoreRes(f64);

fn app() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_plugins(bevy_swarm::driver::PlaytestPlugin);
    app.init_resource::<bevy_swarm::contract::TestApi>();
    app.insert_resource(bevy_swarm::contract::TestApi::default());
    app
}

/// #sum aggregate over a component field.
#[test]
fn fx6_aggregate_sum_over_component_field() {
    let mut a = app();
    let reg = a
        .world()
        .resource::<bevy::ecs::reflect::AppTypeRegistry>()
        .0
        .clone();
    reg.write().register::<Health>();
    reg.write().register::<ScoreRes>();

    let e1 = a.world_mut().spawn((Health(10.0), Name::new("A"))).id();
    let e2 = a.world_mut().spawn((Health(32.0), Name::new("B"))).id();
    let _ = (e1, e2);

    let v = resolve_world_percept(a.world(), "Component:Health.0#sum");
    assert_eq!(
        v,
        Some(bevy_swarm::contract::TestFieldValue::Numeric(42.0)),
        "#sum must aggregate the field over all entities with the component"
    );
}

/// #min/#max aggregates over a component field.
#[test]
fn fx6_aggregate_min_max_over_component_field() {
    let mut a = app();
    let reg = a
        .world()
        .resource::<bevy::ecs::reflect::AppTypeRegistry>()
        .0
        .clone();
    reg.write().register::<Health>();
    reg.write().register::<ScoreRes>();

    a.world_mut().spawn(Health(10.0));
    a.world_mut().spawn(Health(32.0));
    a.world_mut().spawn(Health(7.0));

    assert_eq!(
        resolve_world_percept(a.world(), "Component:Health.0#min"),
        Some(bevy_swarm::contract::TestFieldValue::Numeric(7.0))
    );
    assert_eq!(
        resolve_world_percept(a.world(), "Component:Health.0#max"),
        Some(bevy_swarm::contract::TestFieldValue::Numeric(32.0))
    );
}

/// Aggregates on empty component sets resolve to 0 / absent.
#[test]
fn fx6_aggregate_on_empty_set() {
    let a = app();
    let reg = a
        .world()
        .resource::<bevy::ecs::reflect::AppTypeRegistry>()
        .0
        .clone();
    reg.write().register::<Health>();

    assert_eq!(
        resolve_world_percept(a.world(), "Component:Health.0#sum"),
        Some(bevy_swarm::contract::TestFieldValue::Numeric(0.0)),
        "#sum over no entities must be 0, not None (query-count semantics)"
    );
    // min/max over an empty set is undefined — absent.
    assert_eq!(
        resolve_world_percept(a.world(), "Component:Health.0#min"),
        None
    );
}

/// A resource percept contributes to the canonical-state digest even
/// though it lives outside entity iteration.
#[test]
fn fx6_digest_covers_resource_percepts() {
    let mut a = app();
    let reg = a
        .world()
        .resource::<bevy::ecs::reflect::AppTypeRegistry>()
        .0
        .clone();
    reg.write().register::<ScoreRes>();
    a.insert_resource(ScoreRes(10.0));

    let d1 = bevy_swarm::determinism::canonical_state(a.world());
    a.insert_resource(ScoreRes(20.0));
    let d2 = bevy_swarm::determinism::canonical_state(a.world());

    assert_ne!(
        d1.get("Resource:ScoreRes"),
        d2.get("Resource:ScoreRes"),
        "changing a reflected resource must change the canonical state"
    );
}
