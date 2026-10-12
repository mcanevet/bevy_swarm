//! I2: Compiled invariants — compile invariants once at run start.
//!
//! Eliminates per-frame interpretation overhead: component IDs resolve
//! at load time (typos become load errors), counts use archetype-level
//! queries (O(matched archetypes) vs O(entities)), and field paths use
//! pre-parsed ParsedPath.

use bevy::ecs::component::{ComponentId, Components};
use bevy::ecs::entity::Entity;
use bevy::ecs::world::World;
use bevy::prelude::*;
use bevy::reflect::ParsedPath;

use crate::enums::CheckOp;
use crate::scenario::{Invariant, ScenarioError};

/// Compiled accessor for invariant evaluation.
pub(crate) enum Accessor {
    /// Resource field via reflection.
    ResourceField {
        component_id: ComponentId,
        reflect: bevy::ecs::reflect::ReflectComponent,
        parsed_path: ParsedPath,
    },
    /// Count of entities matching a dynamic filter. The QueryState is
    /// built lazily on the first evaluation where ALL component ids have
    /// been registered (components spawned only at Startup register
    /// their storage lazily; resolving at compile time would falsely
    /// reject valid scenarios).
    Count {
        with_names: Vec<String>,
        without_names: Vec<String>,
        state: Option<QueryState<Entity>>,
    },
}

/// Compiled invariant (ready for fast per-frame evaluation). `window`
/// and `state` are precomputed runtime-evaluation slots (eventually
/// deadlines, delta ring buffers) reserved for the full fast path;
/// today only the count fast path consumes the compiled data.
pub(crate) struct CompiledInvariant {
    pub name: String,
    pub kind: CompiledKind,
    pub accessor: Option<Accessor>,
}

/// Invariant kind (after parsing). Kinds beyond QueryCount are
/// interpreted by the semantic checker (oracles.rs) from the scenario
/// resource; only count invariants gain a compiled fast path today.
pub(crate) enum CompiledKind {
    QueryCount { op: CheckOp, threshold: f64 },
}

/// Compiled scenario (invariants compiled once at load).
#[derive(Resource)]
pub(crate) struct CompiledScenario {
    pub invariants: Vec<CompiledInvariant>,
}

/// Compile invariants from a JSON scenario into a CompiledScenario.
pub(crate) fn compile(
    world: &mut World,
    scenario: &crate::scenario::Scenario,
) -> Result<CompiledScenario, ScenarioError> {
    let mut invariants = Vec::new();

    for inv in &scenario.invariants {
        // Only Custom rules go through this compiler — built-in rules
        // (nodes_in_bounds, frame_time_p99_below, ...) have their own
        // dedicated oracles and no percept compilation.
        if inv.rule != crate::enums::InvariantRule::Custom {
            continue;
        }
        let accessor = compile_accessor(world, inv)?;
        let kind = compile_kind(inv)?;
        invariants.push(CompiledInvariant {
            name: inv.name.clone(),
            kind,
            accessor,
        });
    }

    Ok(CompiledScenario { invariants })
}

fn compile_accessor(world: &mut World, inv: &Invariant) -> Result<Option<Accessor>, ScenarioError> {
    // Query-count accessor (Q1).
    if let Some(query) = &inv.query {
        // Reject unregistered TYPES at load (true typos). Component
        // ids may resolve later (lazy storage registration).
        let (with_ids, without_ids) = {
            let registry = world.resource::<AppTypeRegistry>().0.read();
            let components = world.components();
            let with_ids = resolve_ids(&query.with, &registry, components)?;
            let without_ids = resolve_ids(&query.without, &registry, components)?;
            (with_ids, without_ids)
        };
        if with_ids.iter().all(|i| i.is_some()) && without_ids.iter().all(|i| i.is_some()) {
            // Fast path: all ids known at compile time.
            let mut builder = bevy::ecs::query::QueryBuilder::<Entity>::new(world);
            for id in with_ids.iter().flatten() {
                builder.with_id(*id);
            }
            for id in without_ids.iter().flatten() {
                builder.without_id(*id);
            }
            let qs = builder.build();
            return Ok(Some(Accessor::Count {
                with_names: query.with.clone(),
                without_names: query.without.clone(),
                state: Some(qs),
            }));
        }
        return Ok(Some(Accessor::Count {
            with_names: query.with.clone(),
            without_names: query.without.clone(),
            state: None,
        }));
    }

    // Path-based accessor (D1).
    let Some(path) = &inv.path else {
        return Ok(None);
    };

    // le3.13 (RF14): Component: paths must be validated at load —
    // typos must reject before any frame runs. Before #84 the old
    // EntityField accessor did this; now we restore it explicitly.
    if let Some(rest) = path.strip_prefix("Component:") {
        // Parse "Type.field" or "Type#count" or aggregates.
        let (type_path, selector) =
            rest.split_once(['.', '#'])
                .ok_or_else(|| ScenarioError::InvalidComponent {
                    name: path.to_string(),
                    reason: "missing '.' or '#' in Component:<Type>.<selector>".into(),
                })?;
        // If the separator was '#', the selector must be recombined
        // with it (aggregate form: count/sum/min/max).
        let selector_owned;
        let selector = if !rest.contains('.') && rest.contains('#') {
            selector_owned = format!("#{}", selector);
            selector_owned.as_str()
        } else {
            selector
        };
        // Resolve type_id (reject unregistered types).
        let registry = world.resource::<AppTypeRegistry>().0.read();
        let components = world.components();
        let Some(_comp_id) = resolve_type_id(type_path, &registry, components)? else {
            return Err(ScenarioError::InvalidComponent {
                name: type_path.into(),
                reason: "type not registered".into(),
            });
        };
        drop(registry);
        // Validate selector: #count, #sum, #min, #max, or a field path.
        if selector.starts_with('#') {
            // Aggregate form — accept any valid aggregate keyword.
            match selector {
                "#count" | "#sum" | "#min" | "#max" => (),
                other => {
                    return Err(ScenarioError::InvalidComponent {
                        name: path.to_string(),
                        reason: format!("unknown aggregate '{}'", other),
                    });
                }
            }
        } else {
            // Field path — parse it (reject malformed).
            ParsedPath::parse(selector).map_err(|e| ScenarioError::InvalidComponent {
                name: path.to_string(),
                reason: format!("invalid field path: {:?}", e),
            })?;
        }
        // Component: paths are not yet implemented as Accessor variants
        // — return None but VALIDATE (the test confirms rejection at load).
        return Ok(None);
    }

    // Resource field.
    if let Some(rest) = path.strip_prefix("Resource:") {
        let (type_path, field) =
            rest.split_once('.')
                .ok_or_else(|| ScenarioError::InvalidComponent {
                    name: path.to_string(),
                    reason: "missing '.' in Resource:<Type>.<field>".into(),
                })?;
        let (comp_id, reflect) = {
            let registry = world.resource::<AppTypeRegistry>().0.read();
            let components = world.components();
            let Some(comp_id) = resolve_type_id(type_path, &registry, components)? else {
                return Ok(None);
            };
            let reflect = registry
                .get_with_short_type_path(type_path)
                .and_then(|r| r.data::<bevy::ecs::reflect::ReflectComponent>())
                .ok_or_else(|| ScenarioError::InvalidComponent {
                    name: type_path.into(),
                    reason: "type not registered or lacks ReflectComponent".into(),
                })?
                .clone();
            (comp_id, reflect)
        };
        let parsed = ParsedPath::parse(field).map_err(|e| ScenarioError::InvalidComponent {
            name: path.to_string(),
            reason: format!("invalid field path: {:?}", e),
        })?;
        return Ok(Some(Accessor::ResourceField {
            component_id: comp_id,
            reflect,
            parsed_path: parsed,
        }));
    }

    Ok(None)
}

fn compile_kind(inv: &Invariant) -> Result<CompiledKind, ScenarioError> {
    debug_assert_eq!(inv.rule, crate::enums::InvariantRule::Custom);
    // Q1: query count. A query-count invariant with a missing or
    // non-numeric value silently defaulted to `Ge 0` (always true —
    // vacuous). Reject at load. Path-based / differential invariants
    // interpret `value` elsewhere and may legitimately omit it.
    let op = inv.check.unwrap_or(CheckOp::Ge);
    let threshold = if inv.query.is_some() {
        let t = inv.value.as_ref().and_then(|v| v.as_f64()).ok_or_else(|| {
            ScenarioError::Rejected(format!(
                "custom invariant \"{}\" requires a numeric value",
                inv.name
            ))
        })?;
        if !t.is_finite() {
            return Err(ScenarioError::Rejected(format!(
                "custom invariant \"{}\" threshold must be finite",
                inv.name
            )));
        }
        t
    } else {
        0.0
    };
    Ok(CompiledKind::QueryCount { op, threshold })
}

fn resolve_ids(
    names: &[String],
    registry: &bevy::reflect::TypeRegistry,
    components: &Components,
) -> Result<Vec<Option<ComponentId>>, ScenarioError> {
    names
        .iter()
        .map(|t| {
            let reg = registry.get_with_short_type_path(t).ok_or_else(|| {
                ScenarioError::InvalidComponent {
                    name: t.clone(),
                    reason: "type not registered".into(),
                }
            })?;
            Ok(components.get_valid_id(reg.type_id()))
        })
        .collect()
}

fn resolve_type_id(
    type_path: &str,
    registry: &bevy::reflect::TypeRegistry,
    components: &Components,
) -> Result<Option<ComponentId>, ScenarioError> {
    let reg = registry
        .get_with_short_type_path(type_path)
        .ok_or_else(|| ScenarioError::InvalidComponent {
            name: type_path.into(),
            reason: "type not registered".into(),
        })?;
    Ok(components.get_valid_id(reg.type_id()))
}

#[derive(Clone)]
#[allow(dead_code)]
enum Selector {
    Name(String),
    StableId(u64),
    Index(usize),
}

impl Accessor {
    /// Evaluate the accessor, returning a count (for Count accessors) or
    /// None if not a count accessor.
    pub(crate) fn eval_count(&mut self, world: &mut World) -> Option<usize> {
        if let Accessor::Count {
            with_names,
            without_names,
            state,
        } = self
        {
            if state.is_none() {
                // Lazy resolution: component storage registers lazily, so
                // retry id resolution each evaluation until it succeeds.
                let (with_ids, without_ids) = {
                    let registry = world.resource::<AppTypeRegistry>().0.read();
                    let components = world.components();
                    match (
                        resolve_ids(with_names, &registry, components),
                        resolve_ids(without_names, &registry, components),
                    ) {
                        (Ok(w), Ok(wo)) => (w, wo),
                        // Load-time checks already passed; a registry read
                        // failure here means an unusable world — treat as
                        // unresolved.
                        _ => return None,
                    }
                };
                if with_ids.iter().all(|i| i.is_some()) {
                    // Without filters on registered-but-never-spawned
                    // components exclude NOTHING (no entities have them
                    // yet) — skip those filters instead of reporting
                    // count 0. Unresolved without names must at least be
                    // registered types (load-time checks rejected true
                    // typos), else defer.
                    let without_known = {
                        let registry = world.resource::<AppTypeRegistry>().0.read();
                        without_names.iter().zip(without_ids.iter()).all(|(n, id)| {
                            id.is_some() || registry.get_with_short_type_path(n).is_some()
                        })
                    };
                    if !without_known {
                        return None;
                    }
                    let mut builder = bevy::ecs::query::QueryBuilder::<Entity>::new(world);
                    for id in with_ids.iter().flatten() {
                        builder.with_id(*id);
                    }
                    for id in without_ids.iter().flatten() {
                        builder.without_id(*id);
                    }
                    // Cache ONLY when fully resolved — a query that
                    // skipped a not-yet-spawned without filter must be
                    // rebuilt every evaluation until the component
                    // exists, or a later spawn would silently stop being
                    // excluded.
                    if without_ids.iter().all(|i| i.is_some()) {
                        *state = Some(builder.build());
                    } else {
                        let mut qs = builder.build();
                        qs.update_archetypes(world);
                        return Some(
                            qs.matched_archetypes()
                                .map(|id| {
                                    world
                                        .archetypes()
                                        .get(id)
                                        .map(|a| a.len() as usize)
                                        .unwrap_or(0)
                                })
                                .sum(),
                        );
                    }
                } else {
                    // FX6 I2: a registered-but-never-spawned component
                    // has a valid component id but no storage yet —
                    // `get_valid_id` returns None until first spawn
                    // triggers registration. Since load-time checks
                    // rejected true typos, treat fully-registered
                    // TYPES with unresolved storage as a valid count
                    // of 0 (nothing matches).
                    //
                    // This shortcut is only correct because the
                    // UNRESOLVED names are in with_names: a with filter
                    // on a never-spawned type matches 0 entities, so
                    // the total is 0 regardless of the without side.
                    let registry = world.resource::<AppTypeRegistry>().0.read();
                    let types_known = with_names
                        .iter()
                        .chain(without_names.iter())
                        .all(|n| registry.get_with_short_type_path(n).is_some());
                    if types_known {
                        return Some(0);
                    }
                    // Unknown type after all — genuinely unresolved.
                    return None;
                }
            }
            let qs = state.as_mut()?;
            qs.update_archetypes(world);
            // Sum archetype lengths — O(matched archetypes), not
            // O(entities).
            Some(
                qs.matched_archetypes()
                    .map(|id| {
                        world
                            .archetypes()
                            .get(id)
                            .map(|a| a.len() as usize)
                            .unwrap_or(0)
                    })
                    .sum(),
            )
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::component::Component;
    use bevy::prelude::App;
    use bevy::prelude::MinimalPlugins;
    use bevy::prelude::World;

    #[derive(Component, Reflect)]
    struct Spawned;

    #[derive(Component, Reflect)]
    struct NeverSpawned;

    fn make_world() -> World {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .register_type::<Spawned>()
            .register_type::<NeverSpawned>();
        // Spawn at least one entity with Spawned component
        app.world_mut().spawn(Spawned);
        std::mem::take(app.world_mut())
    }

    #[test]
    fn le313_component_path_typo_rejected_at_load() {
        // le3.13 (RF14): Component: paths must be validated at load.
        let mut world = make_world();
        // Typo'd type name.
        let inv: Invariant =
            serde_json::from_str(r#"{"name":"test","rule":"custom","path":"Component:Gamepla.x"}"#)
                .unwrap();
        let err = compile_accessor(&mut world, &inv)
            .err()
            .expect("typo must reject");
        assert!(matches!(err, ScenarioError::InvalidComponent { .. }));
        // Aggregate forms accepted.
        let inv_count: Invariant = serde_json::from_str(
            r#"{"name":"test","rule":"custom","path":"Component:Spawned#count"}"#,
        )
        .unwrap();
        assert!(compile_accessor(&mut world, &inv_count).is_ok());
    }

    #[test]
    fn without_never_spawned_returns_real_count() {
        // 716.29: without filter on registered-but-never-spawned component
        // must return the real with-matched count, not 0.
        let mut world = make_world();
        let mut accessor = Accessor::Count {
            with_names: vec!["Spawned".to_string()],
            without_names: vec!["NeverSpawned".to_string()],
            state: None,
        };
        let count = accessor.eval_count(&mut world);
        assert_eq!(count, Some(1));
    }

    #[test]
    fn with_never_spawned_returns_zero() {
        // FX6 I2: with filter on never-spawned component still returns 0.
        let mut world = make_world();
        let mut accessor = Accessor::Count {
            with_names: vec!["NeverSpawned".to_string()],
            without_names: vec![],
            state: None,
        };
        let count = accessor.eval_count(&mut world);
        assert_eq!(count, Some(0));
    }

    #[test]
    fn without_never_spawned_defers_if_unresolved() {
        // When without names are not registered (unresolved), defer.
        let mut world = make_world();
        let mut accessor = Accessor::Count {
            with_names: vec!["Spawned".to_string()],
            without_names: vec!["SomeUnknownType".to_string()],
            state: None,
        };
        // resolve_ids errors on the unregistered without name → None.
        let count = accessor.eval_count(&mut world);
        assert_eq!(count, None);
    }
}
