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
                if with_ids.iter().all(|i| i.is_some()) && without_ids.iter().all(|i| i.is_some()) {
                    let mut builder = bevy::ecs::query::QueryBuilder::<Entity>::new(world);
                    for id in with_ids.iter().flatten() {
                        builder.with_id(*id);
                    }
                    for id in without_ids.iter().flatten() {
                        builder.without_id(*id);
                    }
                    *state = Some(builder.build());
                } else {
                    // FX6 I2: a registered-but-never-spawned component
                    // has a valid component id but no storage yet —
                    // `get_valid_id` returns None until first spawn
                    // triggers registration. Since load-time checks
                    // rejected true typos, treat fully-registered
                    // TYPES with unresolved storage as a valid count
                    // of 0 (nothing matches).
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
