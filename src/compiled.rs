//! I2: Compiled invariants — compile invariants once at run start.
//!
//! Eliminates per-frame interpretation overhead: component IDs resolve
//! at load time (typos become load errors), counts use archetype-level
//! queries (O(matched archetypes) vs O(entities)), and field paths use
//! pre-parsed ParsedPath.

use bevy::ecs::component::{ComponentId, Components};
use bevy::ecs::entity::Entity;
use bevy::ecs::query::QueryState;
use bevy::ecs::reflect::AppTypeRegistry;
use bevy::ecs::world::World;
use bevy::prelude::*;
use bevy::reflect::ParsedPath;

use crate::enums::CheckOp;
use crate::scenario::{Invariant, ScenarioError};

/// Compiled accessor for invariant evaluation.
pub(crate) enum Accessor {
    /// Game TestApi resolver (opaque path; runtime-checked). The
    /// path is retained for diagnostics; resolution stays dynamic
    /// (opaque resolver, D1).
    TestApi(#[allow(dead_code)] String),
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
    /// Field of one entity, addressed stably. Reserved for full runtime
    /// wiring (selectors resolve per-frame today); the resolved
    /// component id / reflect handle / parsed path live here for the
    /// fast path once wired.
    EntityField {
        #[allow(dead_code)]
        component_id: ComponentId,
        #[allow(dead_code)]
        reflect: bevy::ecs::reflect::ReflectComponent,
        #[allow(dead_code)]
        parsed_path: ParsedPath,
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
    #[allow(dead_code)]
    pub window: Window,
    #[allow(dead_code)]
    pub state: InvariantState,
}

/// Invariant kind (after parsing). Kinds beyond QueryCount are
/// interpreted by the semantic checker (oracles.rs) from the scenario
/// resource; only count invariants gain a compiled fast path today.
pub(crate) enum CompiledKind {
    QueryCount { op: CheckOp, threshold: f64 },
}

/// Time windows for eventual/delta checks.
#[allow(dead_code)]
pub(crate) struct Window {
    pub after_ticks: u64,
    pub before_ticks: Option<u64>,
    pub deadline_ticks: Option<u64>,
}

/// Runtime state for an invariant.
#[derive(Clone, Default)]
#[allow(dead_code)]
pub(crate) struct InvariantState {
    pub eventually_satisfied: Option<u64>,
    pub delta_ring: std::collections::VecDeque<(u64, f64)>,
    pub last_value: Option<f64>,
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
        let window = Window {
            after_ticks: ticks(inv.after_s, scenario.tps as u64).unwrap_or(0),
            before_ticks: ticks(inv.before_s, scenario.tps as u64),
            deadline_ticks: ticks(inv.eventually_s, scenario.tps as u64),
        };
        invariants.push(CompiledInvariant {
            name: inv.name.clone(),
            kind,
            accessor,
            window,
            state: InvariantState::default(),
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

    // TestApi resolver (opaque; runtime-checked).
    if path.starts_with("TestApi.") {
        return Ok(Some(Accessor::TestApi(path.clone())));
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

    // Entity field (Component:T{<Name>}.<field>, @<stable_id>, [idx]).
    if let Some(rest) = path.strip_prefix("Component:") {
        // Parse selector and field.
        let (type_path, _selector, field) = parse_component_selector(rest)?;
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
        return Ok(Some(Accessor::EntityField {
            component_id: comp_id,
            reflect,
            parsed_path: parsed,
        }));
    }

    Ok(None)
}

fn compile_kind(inv: &Invariant) -> Result<CompiledKind, ScenarioError> {
    debug_assert_eq!(inv.rule, crate::enums::InvariantRule::Custom);
    // Q1: query count.
    let op = inv.check.unwrap_or(CheckOp::Ge);
    let threshold = inv.value.as_ref().and_then(|v| v.as_f64()).unwrap_or(0.0);
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

fn parse_component_selector(rest: &str) -> Result<(&str, Selector, &str), ScenarioError> {
    // Forms: "Type{<Name>}.<field>", "Type@<stable_id>.<field>", "Type[<i>].<field>", "Type.<field>"
    if let Some(brace) = rest.find('{') {
        let close = rest[brace..]
            .find('}')
            .ok_or_else(|| ScenarioError::InvalidComponent {
                name: rest.into(),
                reason: "missing '}'".into(),
            })?
            + brace;
        let name = &rest[brace + 1..close];
        let field = rest
            .get(close + 1..)
            .and_then(|s| s.strip_prefix('.'))
            .ok_or_else(|| ScenarioError::InvalidComponent {
                name: rest.into(),
                reason: "missing '.' after {Name}".into(),
            })?;
        Ok((&rest[..brace], Selector::Name(name.to_string()), field))
    } else if let Some(at) = rest.find('@') {
        let dot = rest[at + 1..]
            .find('.')
            .ok_or_else(|| ScenarioError::InvalidComponent {
                name: rest.into(),
                reason: "missing '.' after @<stable_id>".into(),
            })?;
        let sid: u64 =
            rest[at + 1..at + 1 + dot]
                .parse()
                .map_err(|_| ScenarioError::InvalidComponent {
                    name: rest.into(),
                    reason: "invalid stable_id".into(),
                })?;
        let field = &rest[at + 1 + dot + 1..];
        Ok((&rest[..at], Selector::StableId(sid), field))
    } else if let Some(bracket) = rest.find('[') {
        let close = rest[bracket..]
            .find(']')
            .ok_or_else(|| ScenarioError::InvalidComponent {
                name: rest.into(),
                reason: "missing ']'".into(),
            })?
            + bracket;
        let idx: usize =
            rest[bracket + 1..close]
                .parse()
                .map_err(|_| ScenarioError::InvalidComponent {
                    name: rest.into(),
                    reason: "invalid index".into(),
                })?;
        let field = rest
            .get(close + 1..)
            .and_then(|s| s.strip_prefix('.'))
            .ok_or_else(|| ScenarioError::InvalidComponent {
                name: rest.into(),
                reason: "missing '.' after [idx]".into(),
            })?;
        Ok((&rest[..bracket], Selector::Index(idx), field))
    } else {
        let dot = rest
            .find('.')
            .ok_or_else(|| ScenarioError::InvalidComponent {
                name: rest.into(),
                reason: "missing '.<field>'".into(),
            })?;
        Ok((&rest[..dot], Selector::Index(0), &rest[dot + 1..]))
    }
}

#[derive(Clone)]
#[allow(dead_code)]
enum Selector {
    Name(String),
    StableId(u64),
    Index(usize),
}

fn ticks(seconds: Option<f32>, tps: u64) -> Option<u64> {
    seconds.map(|s| (s * tps as f32) as u64)
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
                    // Still unresolved (unknown component in this world —
                    // distinct from a typo rejected at load). Return None;
                    // the caller reports "unresolved" like the old path.
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

    #[test]
    fn parse_selector_works() {
        let (tp, sel, fld) = parse_component_selector("Hero{Player}.health").unwrap();
        assert_eq!(tp, "Hero");
        assert!(matches!(sel, Selector::Name(s) if s == "Player"));
        assert_eq!(fld, "health");

        let (tp, sel, fld) = parse_component_selector("Health@42.current").unwrap();
        assert_eq!(tp, "Health");
        assert!(matches!(sel, Selector::StableId(42)));
        assert_eq!(fld, "current");

        let (tp, sel, fld) = parse_component_selector("Pos[2].x").unwrap();
        assert_eq!(tp, "Pos");
        assert!(matches!(sel, Selector::Index(2)));
        assert_eq!(fld, "x");

        let (tp, sel, fld) = parse_component_selector("Speed.velocity").unwrap();
        assert_eq!(tp, "Speed");
        assert!(matches!(sel, Selector::Index(0)));
        assert_eq!(fld, "velocity");
    }
}
