//! Oracles: per-frame world checks and the custom-invariant engine.

use bevy::ecs::entity::Entity;
use bevy::ecs::query::{Changed, With};
use bevy::ecs::resource::Resource;
use bevy::ecs::system::{Query, Res, ResMut};
use bevy::ecs::world::World;
use bevy::picking::pointer::PointerId;
use bevy::prelude::{Name, Transform};

use crate::contract::{Gameplay, TestFieldValue, UserIntent};
use crate::scenario::*;
use crate::state::*;
use bevy::transform::components::GlobalTransform;

use crate::driver::ScenarioResource;

// ---------------------------------------------------------------------------
// Invariants
// ---------------------------------------------------------------------------

// FX12 (H1): one-shot warning that an [idx] selector was used
// (unstable across spawns/despawns). Thread-local — resolve_world_percept
// only receives &World; cleared and surfaced per-run by the driver.
thread_local! {
    static WARNED_INDEX_SELECTOR: std::cell::RefCell<Option<String>> =
        const { std::cell::RefCell::new(None) };
}

/// Drain the idx-selector warning (driver calls this when building a report).
pub fn take_index_selector_warning() -> Option<String> {
    WARNED_INDEX_SELECTOR.with(|w| w.borrow_mut().take())
}

/// Detect when a visual Transform diverges from a paired logical
/// position component on the same entity. Games pair a component
/// named `...LogicalPosition` (a tuple struct holding a `Vec2`)
/// with their rendered Transform; this oracle finds pairs via
/// reflection, so games stay pure-Bevy with zero harness imports.
pub(crate) fn check_transform_desync_system(world: &mut World) {
    let frame = world.resource::<PlaytestState>().frame;
    let transform_id = match world
        .components()
        .get_valid_id(std::any::TypeId::of::<Transform>())
    {
        Some(id) => id,
        None => return,
    };
    let mut found: Vec<(String, String)> = Vec::new();
    {
        let registry = world
            .resource::<bevy::ecs::reflect::AppTypeRegistry>()
            .0
            .read();
        for (logical_id, info) in world.components().iter_registered() {
            if !info.name().ends_with("LogicalPosition") {
                continue;
            }
            let Some(type_id) = info.type_id() else {
                continue;
            };
            let Some(rc) = registry.get_type_data::<bevy::ecs::reflect::ReflectComponent>(type_id)
            else {
                continue;
            };
            for archetype in world.archetypes().iter() {
                if !archetype.contains(logical_id) || !archetype.contains(transform_id) {
                    continue;
                }
                for entity in archetype.entities() {
                    let Ok(er) = world.get_entity(entity.id()) else {
                        continue;
                    };
                    let Some(lr) = rc.reflect(er) else { continue };
                    // LogicalPosition is a tuple struct: field 0 is the Vec2.
                    let Ok(ts) = lr.reflect_ref().as_tuple_struct() else {
                        continue;
                    };
                    let Some(field0) = ts.field(0) else { continue };
                    let Some(v2) = field0.try_downcast_ref::<bevy::math::Vec2>() else {
                        continue;
                    };
                    let Some(tr) = er.get::<Transform>() else {
                        continue;
                    };
                    let rendered = bevy::math::Vec2::new(tr.translation.x, tr.translation.y);
                    let diff = (rendered - *v2).length();
                    if diff > 0.01 {
                        found.push((
                            "entity:transform".to_string(),
                            format!(
                                "rendered ({:.2},{:.2}) != logical ({:.2},{:.2}) — visual state diverged by {:.2}",
                                rendered.x, rendered.y, v2.x, v2.y, diff
                            ),
                        ));
                    }
                }
            }
        }
    }
    let mut v = world.resource_mut::<Violations>();
    for (target, detail) in found {
        v.report(crate::rules::TRANSFORM_DESYNC, &target, detail, frame);
    }
}

pub(crate) fn check_finite_transforms_system(
    q: Query<(Entity, &Transform), Changed<Transform>>,
    mut violations: ResMut<Violations>,
    state: Res<PlaytestState>,
    names: Query<&Name>,
) {
    for (entity, transform) in q.iter() {
        // FX7: stable target label — Name when present, else
        // "entity:<component>" (no raw entity ids in violation targets:
        // the same bug on another entity must produce ONE fingerprint).
        let label = match names.get(entity) {
            Ok(n) => format!("name:{}", strip_generated_suffix(n.as_str())),
            Err(_) => "entity:transform".to_string(),
        };
        let t = transform.translation;
        if !t.x.is_finite() || !t.y.is_finite() || !t.z.is_finite() {
            violations.report(
                crate::rules::FINITE_TRANSFORMS,
                &label,
                format!("non-finite translation ({}, {}, {})", t.x, t.y, t.z),
                state.frame,
            );
        }
        // Rotation/scale: NaN quaternions (normalising a zero vector)
        // and zero/NaN scale are classic bugs the translation-only
        // check missed.
        if !transform.rotation.is_finite() || !transform.scale.is_finite() {
            violations.report(
                crate::rules::FINITE_TRANSFORMS,
                &label,
                format!(
                    "non-finite rotation ({:?}) or scale ({:?})",
                    transform.rotation, transform.scale
                ),
                state.frame,
            );
        } else if (transform.rotation.length_squared() - 1.0).abs() > 1e-4 {
            // Separate rule: denormalized quats cost precision but are
            // not NaN — games may legitimately ignore this one.
            violations.report(
                "nodes_rotation_unnormalized",
                &label,
                format!("rotation not normalized: {:?}", transform.rotation),
                state.frame,
            );
        }
    }
}

/// Shared bounds predicate: x/y always; z only when a z bound is declared
/// (check z for 3D nodes — don't silently drop it).
/// Scale for bounds robustness: the largest configured half-width
/// (units cancel; default 1.0 for degenerate single-sided bounds).
fn default_bounds_scale(inv: &Invariant) -> f64 {
    [
        (inv.min_x, inv.max_x),
        (inv.min_y, inv.max_y),
        (inv.min_z, inv.max_z),
    ]
    .iter()
    .filter_map(|(lo, hi)| match (lo, hi) {
        (Some(lo), Some(hi)) => Some(((hi - lo) / 2.0).abs().max(1.0) as f64),
        _ => None,
    })
    .fold(1.0, f64::max)
}

/// J1: signed margin of a position against the invariant's bounds —
/// min over checked axes of min(v - lo, hi - v). Positive = inside.
pub(crate) fn bounds_margin(t: bevy::math::Vec3, inv: &Invariant) -> f64 {
    let axes = [(t.x, inv.min_x, inv.max_x), (t.y, inv.min_y, inv.max_y)];
    let mut m = f64::INFINITY;
    for (v, lo, hi) in axes {
        if let (Some(lo), Some(hi)) = (lo, hi) {
            m = m.min((v - lo).min(hi - v) as f64);
        } else if let Some(lo) = lo {
            m = m.min((v - lo) as f64);
        } else if let Some(hi) = hi {
            m = m.min((hi - v) as f64);
        }
    }
    if inv.min_z.is_some() || inv.max_z.is_some() {
        let lo = inv.min_z.unwrap_or(f32::MIN);
        let hi = inv.max_z.unwrap_or(f32::MAX);
        m = m.min((t.z - lo).min(hi - t.z) as f64);
    }
    m
}

fn out_of_bounds(t: bevy::math::Vec3, inv: &Invariant) -> bool {
    let (min_x, max_x, min_y, max_y) = (
        inv.min_x.unwrap_or(-10000.0),
        inv.max_x.unwrap_or(10000.0),
        inv.min_y.unwrap_or(-10000.0),
        inv.max_y.unwrap_or(10000.0),
    );
    let xy_ok = t.x >= min_x && t.x <= max_x && t.y >= min_y && t.y <= max_y;
    let z_ok = if inv.min_z.is_some() || inv.max_z.is_some() {
        let lo = inv.min_z.unwrap_or(f32::MIN);
        let hi = inv.max_z.unwrap_or(f32::MAX);
        t.z >= lo && t.z <= hi
    } else {
        true
    };
    !(xy_ok && z_ok)
}

/// Bounds checking — two-tier: existence/aliveness of explicit targets is
/// checked EVERY tick (a despawned target is a violation the moment it
/// goes missing — `no_null_refs` semantics), but the numeric bounds
/// comparison only runs on `Changed<Transform>` (O(changes), not
/// O(all entities × ticks)). ECS change detection is what makes the
/// per-tick poll affordable.
#[allow(clippy::type_complexity)]
pub(crate) fn check_bounds_gameplay_system(
    q_changed: Query<(&Name, &GlobalTransform), (With<Gameplay>, Changed<GlobalTransform>)>,
    q_all: Query<(&Name, &GlobalTransform), With<Gameplay>>,
    mut violations: ResMut<Violations>,
    state: Res<PlaytestState>,
    scenario: Res<ScenarioResource>,
    mut robustness: ResMut<crate::robustness::RobustnessTracker>,
) {
    for inv in &scenario.0.invariants {
        if inv.rule != crate::enums::InvariantRule::NodesInBounds {
            continue;
        }
        if let Some(after) = inv.after_s {
            if state.elapsed_s() < after {
                continue;
            }
        }
        if let Some(before) = inv.before_s {
            if state.elapsed_s() >= before {
                continue;
            }
        }
        if inv.targets.is_empty() {
            // Default: gameplay entities only — structural/UI at origin
            // are legitimately positioned and would flood reports.
            // Only re-check entities whose Transform changed this tick.
            for (name, transform) in q_changed.iter() {
                let margin = bounds_margin(transform.translation(), inv);
                robustness.observe(
                    &format!("{}/{}", inv.name, name.as_ref()),
                    state.frame,
                    margin,
                    inv.scale.unwrap_or_else(|| default_bounds_scale(inv)),
                );
                if out_of_bounds(transform.translation(), inv) {
                    violations.report(
                        &inv.name,
                        name.as_ref(),
                        format!(
                            "out of bounds ({}, {}, {})",
                            transform.translation().x,
                            transform.translation().y,
                            transform.translation().z
                        ),
                        state.frame,
                    );
                }
            }
        } else {
            // Explicit targets: existence is part of the contract —
            // a vanished target checks ZERO entities and would pass green.
            // Existence + bounds both checked here; existence every tick,
            // bounds only on change.
            for target in &inv.targets {
                let matched_changed: Vec<(&Name, &GlobalTransform)> = q_changed
                    .iter()
                    .filter(|(n, _)| n.as_str() == target)
                    .collect();
                let matched_all: Vec<(&Name, &GlobalTransform)> =
                    q_all.iter().filter(|(n, _)| n.as_str() == target).collect();
                if matched_all.is_empty() {
                    violations.report(
                        &inv.name,
                        target,
                        "target not found — expected entity missing from the scene (missing gameplay object?)"
                            .to_string(),
                        state.frame,
                    );
                }
                for (name, transform) in matched_changed {
                    let margin = bounds_margin(transform.translation(), inv);
                    robustness.observe(
                        &format!("{}/{}", inv.name, name.as_ref()),
                        state.frame,
                        margin,
                        inv.scale.unwrap_or_else(|| default_bounds_scale(inv)),
                    );
                    if out_of_bounds(transform.translation(), inv) {
                        violations.report(
                            &inv.name,
                            name.as_ref(),
                            format!(
                                "out of bounds ({}, {}, {})",
                                transform.translation().x,
                                transform.translation().y,
                                transform.translation().z
                            ),
                            state.frame,
                        );
                    }
                }
            }
        }
    }
}
// ---------------------------------------------------------------------------
// Custom invariants — resolve TestApi paths (the contract read surface)
// ---------------------------------------------------------------------------

/// Resolve a `TestApi.<field>` path by delegating to the GAME's
/// `TestApi::resolve` — the harness holds no field-name knowledge. When a
/// game extends `TestApi`, it extends its own `resolve` implementation
/// (see testable-conventions template). Numeric fields feed below/above +
/// windowed max_delta_per_sec; text fields feed equals.
/// World-level reflection percept resolution — generic state discovery
/// without game-side boilerplate (the percept layer).
///
/// Supported path forms:
///   `Resource:<TypePath>.<field>`     — Resource field (single-instance)
///   `Component:<TypePath>#count`          — count of entities with component
///   `Component:<TypePath>{<Name>}.<field>` — named entity's field (STABLE)
///   `Component:<TypePath>@<id>.<field>`   — entity-id-addressed field
///   `Component:<TypePath>[N].<field>`     — Nth of Entity-SORTED candidates
///     (unstable across despawns; prefer {Name})
///   `Component:<TypePath>.<field>`        — first Entity-sorted candidate
///
/// Enums resolve to variant name as Text. Numeric leaf types → Numeric.
/// Read an entity's Name without a pre-built query (World APIs are
/// enough — avoids init requirements inside &World contexts).
fn name_ref(entity: bevy::ecs::world::EntityRef<'_>) -> Option<&'_ str> {
    entity.get::<Name>().map(|n| n.as_str())
}

/// Component-percept entity selector (see resolve_world_percept).
enum Selector<'a> {
    Name(&'a str),
    StableId(u64),
    Index(usize, bool),
}

pub fn resolve_world_percept(world: &World, path: &str) -> Option<TestFieldValue> {
    use bevy::reflect::GetPath;

    let registry = world
        .resource::<bevy::ecs::reflect::AppTypeRegistry>()
        .0
        .read();

    // Helper: look up a registered type by short path and resolve its world
    // ComponentId (may not exist if never inserted as component/resource).
    let comp_id_for = |type_path: &str| -> Option<bevy::ecs::component::ComponentId> {
        let reg = registry.get_with_short_type_path(type_path)?;
        world.components().get_valid_id(reg.type_id())
    };

    // Resource: "Resource:<TypePath>.<field>" — resources live on singleton
    // entities; find it via resource_entities, then ReflectComponent.
    if let Some(rest) = path.strip_prefix("Resource:") {
        let dot_idx = rest.find('.')?;
        let type_path = &rest[..dot_idx];
        let field = &rest[dot_idx + 1..];
        let comp_id = comp_id_for(type_path)?;
        let reflect_component = registry
            .get_with_short_type_path(type_path)?
            .data::<bevy::ecs::reflect::ReflectComponent>()?;
        // Find the singleton entity holding this resource component.
        let resource_entity = world
            .resource_entities()
            .iter()
            .find(|(id, _)| *id == comp_id)
            .map(|(_, e)| e)?;
        let entity = world.get_entity(resource_entity).ok()?;
        let reflect = reflect_component.reflect(entity)?;
        let val = reflect.reflect_path(field).ok()?;
        return Some(value_from_reflect(val));
    }

    if let Some(rest) = path.strip_prefix("Component:") {
        // Count: "Component:<TypePath>#count"
        if let Some(type_path) = rest.strip_suffix("#count") {
            let comp_id = comp_id_for(type_path)?;
            let mut count = 0usize;
            for entity in world.iter_entities() {
                if entity.contains_id(comp_id) {
                    count += 1;
                }
            }
            return Some(TestFieldValue::Numeric(count as f64));
        }
        // Field addressing forms:
        //   "Component:<TypePath>{<Name>}.<field>" — STABLE: entity by Name
        //   "Component:<TypePath>@<stable_id>.<field>" — stable id (I1)
        //   "Component:<TypePath>[<i>].<field>" — index into Entity-sorted
        //     candidates (UNSTABLE across despawns; one-time warning)
        //   "Component:<TypePath>.<field>" — first of Entity-sorted candidates
        let (type_path, selector, field) = if let Some(brace) = rest.find('{') {
            // {Name} addressing
            let close = rest[brace..].find('}')? + brace;
            let name = &rest[brace + 1..close];
            let field = rest.get(close + 1..)?.strip_prefix('.')?;
            (&rest[..brace], Selector::Name(name), field)
        } else if let Some(at) = rest.find('@') {
            // @<stable_id> addressing (requires Name for now — I1 maps
            // stable ids; fall through to None when unmatched)
            let field = rest[at + 1..].split_once('.')?;
            let sid: u64 = rest[at + 1..rest.find('.').unwrap_or(rest.len())]
                .parse()
                .ok()?;
            (&rest[..at], Selector::StableId(sid), field.1)
        } else if let Some(bracket_end) = rest.find('[') {
            let idx_start = bracket_end + 1;
            let idx_end = rest[idx_start..].find(']')? + idx_start;
            let idx: usize = rest[idx_start..idx_end].parse().ok()?;
            let field = rest.get(idx_end + 1..)?.strip_prefix('.')?;
            (&rest[..bracket_end], Selector::Index(idx, true), field)
        } else {
            let dot_idx = rest.find('.')?;
            (
                &rest[..dot_idx],
                Selector::Index(0, false),
                &rest[dot_idx + 1..],
            )
        };

        let comp_id = comp_id_for(type_path)?;
        let reflect_component = registry
            .get_with_short_type_path(type_path)?
            .data::<bevy::ecs::reflect::ReflectComponent>()?;

        // Collect candidates SORTED by Entity — iter_entities() order
        // changes with spawns/despawns, so a positional index would
        // silently read different entities over time. Entity has a
        // stable total order.
        let mut candidates: Vec<bevy::ecs::entity::Entity> = world
            .iter_entities()
            .filter(|e| e.contains_id(comp_id))
            .map(|e| e.id())
            .collect();
        candidates.sort();

        match selector {
            Selector::Name(name) => {
                // Read the Name component directly via reflection-free
                // lookup: world.get() with the component's ReflectComponent.
                for entity in &candidates {
                    let matches_name = world
                        .get_entity(*entity)
                        .ok()
                        .and_then(|e| name_ref(e))
                        .is_some_and(|n| n == name);
                    if matches_name {
                        let Ok(entity_ref) = world.get_entity(*entity) else {
                            continue;
                        };
                        let reflect = reflect_component.reflect(entity_ref)?;
                        let val = reflect.reflect_path(field).ok()?;
                        return Some(value_from_reflect(val));
                    }
                }
            }
            Selector::StableId(sid) => {
                // FX6 I2: resolve @id via the identity index (works for
                // unnamed entities; stable across resets).
                if let Some(idx) = world.get_resource::<crate::identity::IdentityIndex>() {
                    if let Some(entity) = idx.by_stable(crate::identity::StableId(sid)) {
                        if let Ok(entity_ref) = world.get_entity(entity) {
                            let reflect = reflect_component.reflect(entity_ref)?;
                            let val = reflect.reflect_path(field).ok()?;
                            return Some(value_from_reflect(val));
                        }
                    }
                }
            }
            Selector::Index(idx, warn) => {
                // FX12 (H1): one-time warning that [idx] selectors are
                // unstable across despawns (prefer {Name} or @stable_id).
                if warn {
                    WARNED_INDEX_SELECTOR.with(|w| {
                        let mut w = w.borrow_mut();
                        if w.is_none() {
                            *w = Some(format!(
                                "[idx] selector on {}[{}] is unstable across spawns/despawns — prefer {{Name}} or @stable_id",
                                type_path, idx
                            ));
                        }
                    });
                }
                // Both [N] and the implicit first form use the sorted
                // candidate list; the caller-visible warning about index
                // instability is emitted where the report is built.
                if let Some(entity) = candidates.get(idx) {
                    if let Ok(entity_ref) = world.get_entity(*entity) {
                        let reflect = reflect_component.reflect(entity_ref)?;
                        let val = reflect.reflect_path(field).ok()?;
                        return Some(value_from_reflect(val));
                    }
                }
            }
        }
        return None;
    }

    None
}

/// Convert a reflected leaf value into a TestFieldValue. Numbers become
/// Numeric; enums become Text (variant name); strings become Text;
/// anything else falls back to the debug representation.
fn value_from_reflect(val: &dyn bevy::reflect::PartialReflect) -> TestFieldValue {
    use crate::contract::TestFieldValue;
    use bevy::reflect::ReflectKind;

    // Try numeric conversions
    if let Some(v) = val.try_downcast_ref::<i64>() {
        return TestFieldValue::Numeric(*v as f64);
    }
    if let Some(v) = val.try_downcast_ref::<i32>() {
        return TestFieldValue::Numeric(*v as f64);
    }
    if let Some(v) = val.try_downcast_ref::<u64>() {
        return TestFieldValue::Numeric(*v as f64);
    }
    if let Some(v) = val.try_downcast_ref::<u32>() {
        return TestFieldValue::Numeric(*v as f64);
    }
    if let Some(v) = val.try_downcast_ref::<f64>() {
        return TestFieldValue::Numeric(*v);
    }
    if let Some(v) = val.try_downcast_ref::<f32>() {
        return TestFieldValue::Numeric(*v as f64);
    }
    if let Some(v) = val.try_downcast_ref::<bool>() {
        return TestFieldValue::Numeric(if *v { 1.0 } else { 0.0 });
    }
    // Enum: stringify variant name
    if val.reflect_kind() == ReflectKind::Enum {
        if let bevy::reflect::ReflectRef::Enum(enum_val) = val.reflect_ref() {
            return TestFieldValue::Text(enum_val.variant_name().to_string());
        }
    }
    // String
    if let Some(s) = val.try_downcast_ref::<String>() {
        return TestFieldValue::Text(s.clone());
    }
    // Fallback: debug repr
    TestFieldValue::Text(format!("{val:?}"))
}

/// Count entities matching a QueryTarget (component filters). This is the
/// ECS-native analogue of Playwright's `toHaveCount` — query-shaped
/// assertions on entity archetypes. Uses reflection to resolve component
/// short type paths to ComponentIds, then iterates entities checking
/// presence/absence.
fn count_query_target(world: &World, target: &QueryTarget) -> Option<usize> {
    let registry = world
        .resource::<bevy::ecs::reflect::AppTypeRegistry>()
        .0
        .read();

    let resolve_ids = |names: &[String]| -> Option<Vec<bevy::ecs::component::ComponentId>> {
        names
            .iter()
            .map(|t| {
                let reg = registry.get_with_short_type_path(t)?;
                world.components().get_valid_id(reg.type_id())
            })
            .collect()
    };

    // Unresolvable component names → None (caller reports the typo).
    let with_ids = resolve_ids(&target.with)?;
    let without_ids = resolve_ids(&target.without)?;

    let mut count = 0;
    for entity in world.iter_entities() {
        if with_ids.iter().all(|id| entity.contains_id(*id))
            && without_ids.iter().all(|id| !entity.contains_id(*id))
        {
            count += 1;
        }
    }
    Some(count)
}

/// Resolve via the type-erased resolver (registered first, then world
/// percepts) — a custom TestApi type works without the crate's own
/// TestApi resource.
/// Evaluate a compiled reflect-field accessor (resource or entity
/// singleton) with a pre-parsed path (I2 fast path).
fn eval_reflect_field(
    world: &World,
    registry: &bevy::reflect::TypeRegistry,
    component_id: bevy::ecs::component::ComponentId,
    reflect: &bevy::ecs::reflect::ReflectComponent,
    parsed_path: &bevy::reflect::ParsedPath,
) -> Option<(Option<f64>, Option<String>)> {
    let _ = registry;
    let _ = registry;
    let resource_entity = world
        .resource_entities()
        .iter()
        .find(|(id, _)| *id == component_id)
        .map(|(_, e)| e)?;
    let entity = world.get_entity(resource_entity).ok()?;
    let reflect_val = reflect.reflect(entity)?;
    use bevy::reflect::GetPath;
    let val = reflect_val.reflect_path(parsed_path).ok()?;
    Some(match value_from_reflect(val) {
        crate::contract::TestFieldValue::Numeric(n) => (Some(n), None),
        crate::contract::TestFieldValue::Text(s) => (None, Some(s)),
    })
}

fn resolve_field(world: &World, path: &str) -> (Option<f64>, Option<String>) {
    match crate::contract::resolve_path(world, path) {
        Some(TestFieldValue::Numeric(n)) => (Some(n), None),
        Some(TestFieldValue::Text(s)) => (None, Some(s)),
        None => (None, None),
    }
}

/// Exclusive system: snapshot scenario/TestApi state immutably first, then
/// mutate violations/delta-windows (two-phase to satisfy the borrow checker).
pub(crate) fn check_custom_system(world: &mut World) {
    let scenario = world.resource::<ScenarioResource>().0.clone();
    let frame = world.resource::<PlaytestState>().frame;
    let elapsed_s = world.resource::<PlaytestState>().elapsed_s();
    let tps = world.resource::<PlaytestState>().tps;

    // J1: record this frame's signed robustness margin for the
    // invariant (threshold / query-count / differential / rate /
    // implication). Boolean verdict == sign of ρ (debug-checked).
    macro_rules! rho {
        ($name:expr, $rho:expr, $scale:expr) => {
            let __rho: f64 = $rho;
            if let Some(sc) = $scale {
                world
                    .resource_mut::<crate::robustness::RobustnessTracker>()
                    .observe($name, frame, __rho, sc);
            }
        };
    }

    for inv in &scenario.invariants {
        if inv.rule != crate::enums::InvariantRule::Custom {
            continue;
        }
        if let Some(after) = inv.after_s {
            if elapsed_s < after {
                continue;
            }
        }
        if let Some(before) = inv.before_s {
            if elapsed_s >= before {
                continue;
            }
        }

        // Query-target invariants: component-filtered entity counts.
        // ECS-native analogue of Playwright's `toHaveCount`.
        if let Some(query) = &inv.query {
            // I2: compiled QueryState — O(matched archetypes) counting,
            // load-time rejection of unknown component types.
            let count = {
                if world.contains_resource::<crate::compiled::CompiledScenario>() {
                    let mut result = None;
                    world.resource_scope(
                        |world,
                         mut cs: bevy::ecs::change_detection::Mut<
                            crate::compiled::CompiledScenario,
                        >| {
                            result = cs
                                .invariants
                                .iter_mut()
                                .find(|ci| ci.name == inv.name)
                                .and_then(|ci| ci.accessor.as_mut())
                                .and_then(|acc| acc.eval_count(world));
                        },
                    );
                    result
                } else {
                    count_query_target(world, query)
                }
            };
            let Some(count) = count else {
                world.resource_mut::<Violations>().report(
                    &inv.name,
                    &format!("query({:?})", query),
                    "unresolved component type in query (typo?)".into(),
                    frame,
                );
                continue;
            };
            // I2: use the compiled kind when available (pre-parsed op
            // + threshold); fall back to per-frame JSON interpretation.
            let (check, threshold) = {
                let cs = world.resource::<crate::compiled::CompiledScenario>();
                match cs
                    .invariants
                    .iter()
                    .find(|ci| ci.name == inv.name)
                    .map(|ci| &ci.kind)
                {
                    Some(crate::compiled::CompiledKind::QueryCount { op, threshold }) => {
                        (*op, Some(*threshold))
                    }
                    _ => (
                        inv.check.unwrap_or(crate::enums::CheckOp::Ge),
                        inv.value.as_ref().and_then(|v| v.as_f64()),
                    ),
                }
            };

            let holds_now = match threshold {
                Some(thr) => check.holds(count as f64, thr),
                None => false,
            };
            if let Some(thr) = threshold {
                let margin = crate::robustness::threshold_margin(check, count as f64, thr);
                debug_assert_eq!(
                    holds_now,
                    !(margin < 0.0
                        || (matches!(
                            check,
                            crate::enums::CheckOp::Lt
                                | crate::enums::CheckOp::Gt
                                | crate::enums::CheckOp::Ne
                        ) && margin == 0.0)),
                    "J1 drift: {check:?} count={count} thr={thr} rho={margin}"
                );
                rho!(
                    &inv.name,
                    margin,
                    inv.scale.or(Some(crate::robustness::default_scale(thr)))
                );
            }

            match inv.eventually_s {
                Some(deadline) => {
                    let mut state_mut = world.resource_mut::<PlaytestState>();
                    let entry = state_mut
                        .eventually_state
                        .entry(inv.name.clone())
                        .or_insert((deadline, None));
                    if holds_now {
                        if entry.1.is_none() {
                            entry.1 = Some(frame);
                        }
                    } else if elapsed_s >= deadline && entry.1.is_none() {
                        world.resource_mut::<Violations>().report(
                        &inv.name,
                        &format!("query({:?})", query),
                            format!(
                                "query count {} never {} {} within {:.1}s (eventually deadline expired)",
                                count, check, threshold.unwrap_or(0.0), deadline
                            ),
                            frame,
                        );
                    }
                }
                None => {
                    if !holds_now {
                        world.resource_mut::<Violations>().report(
                            &inv.name,
                            &format!("query({:?})", query),
                            format!(
                                "query count {} violates {} {}",
                                count,
                                check.symbol(),
                                threshold.unwrap_or(0.0)
                            ),
                            frame,
                        );
                    }
                }
            }
            continue;
        }

        let Some(path) = &inv.path else { continue };
        // I2: compiled accessor fast paths (Resource/Component reflect
        // fields with pre-parsed paths) when available; TestApi and
        // unresolved fall back to the resolver chain.
        let (numeric, string_val) =
            if world.contains_resource::<crate::compiled::CompiledScenario>() {
                let mut result = None;
                world.resource_scope(
                |world, cs: bevy::ecs::change_detection::Mut<crate::compiled::CompiledScenario>| {
                    let ci = cs.invariants.iter().find(|ci| ci.name == inv.name);
                    result = match ci.and_then(|ci| ci.accessor.as_ref()) {
                        Some(crate::compiled::Accessor::ResourceField {
                            component_id,
                            reflect,
                            parsed_path,
                        }) => {
                            let registry = world
                                .resource::<bevy::ecs::reflect::AppTypeRegistry>()
                                .0
                                .read();
                            eval_reflect_field(
                                world,
                                &registry,
                                *component_id,
                                reflect,
                                parsed_path,
                            )
                        }
                        _ => None,
                    };
                },
            );
                result.unwrap_or_else(|| resolve_field(world, path))
            } else {
                resolve_field(world, path)
            };
        world
            .resource_mut::<PlaytestState>()
            .coverage
            .test_api_paths_read
            .insert(path.clone());
        if numeric.is_none() && string_val.is_none() {
            // Warn-once for unresolved paths (per-cycle would recreate
            // log spam). Unknown TestApi field = contract breach.
            if !world
                .resource::<PlaytestState>()
                .warned_paths
                .contains(path)
            {
                world
                    .resource_mut::<PlaytestState>()
                    .warned_paths
                    .insert(path.clone());
                world.resource_mut::<Violations>().report(
                    &inv.name,
                    path,
                    "TestApi field not found — extend TestApi or fix the invariant path"
                        .to_string(),
                    frame,
                );
            }
            continue;
        }

        // FX5 C1: string comparison respects inv.check (Equals/Ne only).
        if let (Some(sv), Some(serde_json::Value::String(expect))) = (&string_val, &inv.value) {
            let check = inv.check.unwrap_or(crate::enums::CheckOp::Equals);
            let holds = check.holds_text(sv, expect).unwrap_or(false);
            // Eventually-mode: pass as soon as satisfied; report only at
            // deadline expiry if never satisfied.
            if let Some(deadline) = inv.eventually_s {
                let mut state_mut = world.resource_mut::<PlaytestState>();
                let entry = state_mut
                    .eventually_state
                    .entry(inv.name.clone())
                    .or_insert((deadline, None));
                if holds {
                    entry.1 = Some(entry.1.unwrap_or(frame)); // first satisfaction
                } else if elapsed_s >= deadline && entry.1.is_none() {
                    world.resource_mut::<Violations>().report(
                        &inv.name,
                        path,
                        format!(
                            "{} never {} {} within {:.1}s (eventually deadline expired)",
                            path, check, expect, deadline
                        ),
                        frame,
                    );
                }
            } else if !holds {
                world.resource_mut::<Violations>().report(
                    &inv.name,
                    path,
                    format!("{} = {} ({} {})", path, sv, check, expect),
                    frame,
                );
            }
            continue;
        }

        let Some(current) = numeric else { continue };
        let check = inv.check.unwrap_or(crate::enums::CheckOp::Le);
        let threshold = inv.value.as_ref().and_then(|v| v.as_f64());
        if let Some(thr) = threshold {
            // Shared predicate evaluation for both modes.
            let holds_now = check.holds(current, thr);
            // J1: per-frame margin (eventually-mode callers would track
            // best-so-far; the tracker keeps min over run which the
            // summary reports — the two views compose).
            let margin = crate::robustness::threshold_margin(check, current, thr);
            debug_assert_eq!(
                holds_now,
                !(margin < 0.0
                    || (matches!(
                        check,
                        crate::enums::CheckOp::Lt
                            | crate::enums::CheckOp::Gt
                            | crate::enums::CheckOp::Ne
                    ) && margin == 0.0)),
                "J1 drift: {check:?} v={current} thr={thr} rho={margin}"
            );
            rho!(
                &inv.name,
                margin,
                inv.scale.or(Some(crate::robustness::default_scale(thr)))
            );
            match inv.eventually_s {
                // Eventually-mode: satisfied on first hold; report only at
                // deadline expiry if never held. Semantics mirror
                // Playwright's expect().toBeVisible(timeout) / gdUnit's
                // await_func().wait_until(ms).
                Some(deadline) => {
                    let mut state_mut = world.resource_mut::<PlaytestState>();
                    let entry = state_mut
                        .eventually_state
                        .entry(inv.name.clone())
                        .or_insert((deadline, None));
                    if holds_now {
                        if entry.1.is_none() {
                            entry.1 = Some(frame);
                        }
                    } else if elapsed_s >= deadline && entry.1.is_none() {
                        world.resource_mut::<Violations>().report(
                            &inv.name,
                            path,
                            format!(
                                "{} = {} never {} {} within {:.1}s (eventually deadline expired)",
                                path, current, check, thr, deadline
                            ),
                            frame,
                        );
                    }
                }
                None => {
                    // Always-mode: strict per-frame check.
                    if !holds_now {
                        world.resource_mut::<Violations>().report(
                            &inv.name,
                            path,
                            format!("{} = {} (threshold: {})", path, current, thr),
                            frame,
                        );
                    }
                }
            }
        }

        // Windowed rate-of-change: rate over ~1s of samples, not per-tick.
        // A discrete +1 event is a window rate of ~1/sec — legal under a
        // ceiling >= 1. A re-firing handler climbs the whole window.
        if let Some(max_dps) = inv.max_delta_per_sec {
            let mut state_mut = world.resource_mut::<PlaytestState>();
            let samples = state_mut.delta_windows.entry(inv.name.clone()).or_default();
            samples.push_back((frame, current));
            let window_ticks = tps.max(1);
            while samples.len() > 1 && frame - samples[0].0 > window_ticks {
                samples.pop_front();
            }
            let oldest = samples[0];
            let dt_ticks = frame - oldest.0;
            // Evaluate only once the window holds a full physics second —
            // shorter spans measure rates with warm-up skew.
            if dt_ticks >= window_ticks {
                let rate = (current - oldest.1).abs() / (dt_ticks as f64 / tps as f64);
                rho!(
                    &inv.name,
                    max_dps - rate,
                    inv.scale
                        .or(Some(crate::robustness::default_scale(max_dps)))
                );
                if rate > max_dps {
                    world.resource_mut::<Violations>().report(
                        &inv.name,
                        path,
                        format!(
                            "delta rate {:.1}/sec exceeds max_delta_per_sec {} (value {} -> {} over {} ticks)",
                            rate, max_dps, oldest.1, current, dt_ticks
                        ),
                        frame,
                    );
                }
            }
        }

        // Differential invariants: no_decrease (value must never drop) or
        // no_increase (value must never rise). Requires at least one prior
        // sample to compare against.
        if let Some(diff) = &inv.differential {
            let mut state_mut = world.resource_mut::<PlaytestState>();
            let prev = state_mut.api_history.insert(path.clone(), current);
            if let Some(prev_val) = prev {
                let violated = if diff == &crate::enums::DifferentialOp::NoDecrease {
                    current < prev_val
                } else {
                    current > prev_val // no_increase
                };
                // J1: signed margin of this step (positive = compliant).
                let step_margin = if diff == &crate::enums::DifferentialOp::NoDecrease {
                    current - prev_val
                } else {
                    prev_val - current
                };
                rho!(
                    &inv.name,
                    step_margin,
                    inv.scale.or(Some(crate::robustness::default_scale(
                        prev_val.abs().max(current)
                    )))
                );
                if violated {
                    world.resource_mut::<Violations>().report(
                        &inv.name,
                        path,
                        format!(
                            "{}: {} changed from {} to {} (violates {})",
                            diff, path, prev_val, current, diff
                        ),
                        frame,
                    );
                }
            }
        }

        // Expert-rule oracle: WHEN `path` satisfies check vs value,
        // REQUIRE `requires_path` to satisfy requires_check vs
        // requires_value. Checks game POLICY compliance ("low HP ⇒ must
        // heal"), complementing state invariants.
        if let (Some(req_path), Some(req_check), Some(req_value)) =
            (&inv.requires_path, &inv.requires_check, &inv.requires_value)
        {
            // WHEN guard: only fire when the condition holds. A missing
            // `check` means an unconditional policy rule.
            let when_holds = if let (Some(check), Some(when_val)) = (&inv.check, &inv.value) {
                if let (Some(cur), Some(thr)) = (numeric, when_val.as_f64()) {
                    check.holds(cur, thr)
                } else {
                    // FX5 C1: Text WHEN respects the check op
                    // (Equals/Ne; other ops on strings are rejected at
                    // load, unwrap_or(false) is a defensive default).
                    string_val
                        .as_deref()
                        .and_then(|sv| when_val.as_str().map(|wv| check.holds_text(sv, wv)))
                        .flatten()
                        .unwrap_or(false)
                }
            } else {
                true
            };
            if when_holds {
                // requires_path resolves through the SAME entry point:
                // registered resolver first, then world percepts
                // (previously skipped percepts entirely).
                let (req_num, req_str) = resolve_field(world, req_path);
                // J1: implication robustness ¬W ∨ R = max(-ρ_W, ρ_R).
                // While WHEN holds (mask active), the reactive clause
                // dominates: record ρ_R when both clauses are numeric.
                // Anti-masking: WHEN false frames are NOT recorded
                // here (the guard clause ρ_W > 0 dominates then and is
                // tracked via the threshold branch above).
                if let Some(r_thr) = req_value.as_f64() {
                    if let Some(rc) = req_num {
                        // ρ_R: the reactive clause margin while masked.
                        let r_margin = crate::robustness::threshold_margin(*req_check, rc, r_thr);
                        rho!(
                            &inv.name,
                            r_margin,
                            inv.scale.or(Some(crate::robustness::default_scale(r_thr)))
                        );
                    }
                }
                world
                    .resource_mut::<PlaytestState>()
                    .coverage
                    .test_api_paths_read
                    .insert(req_path.clone());
                let violated = match (req_num, req_value.as_f64()) {
                    (Some(v), Some(thr)) => !req_check.holds(v, thr),
                    _ => match (&req_str, req_value.as_str()) {
                        (Some(s), Some(exp)) => !req_check.holds_text(s, exp).unwrap_or(true),
                        _ => false,
                    },
                };
                if violated {
                    let actual = req_num
                        .map(|n| n.to_string())
                        .or_else(|| req_str.clone())
                        .unwrap_or_else(|| "<unresolved>".into());
                    let when_desc = numeric
                        .map(|n| format!("{} = {}", path, n))
                        .or_else(|| string_val.clone().map(|s| format!("{} = {}", path, s)))
                        .unwrap_or_else(|| path.clone());
                    world.resource_mut::<Violations>().report(
                        &inv.name,
                        req_path,
                        format!(
                            "expert rule violated: when {}, {} = {} (required: {} {})",
                            when_desc, req_path, actual, req_check, req_value
                        ),
                        frame,
                    );
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Intent audit observer — optional, ECS-native (0.20 observer `Event`s)
// ---------------------------------------------------------------------------

/// : fired whenever a bot (or anything) writes a
/// `UserIntent`. Games/harness extensions can listen to audit intent
/// traffic without polling. The audit logger (below) counts every intent
/// by variant for coverage — ECS observers replace the per-bot manual
/// counting with a single centralized listener.
/// Centralized intent audit log: reads ALL UserIntent messages (fan-out —
/// buffered messages can have many readers) and records them in the action
/// log with full structured payloads (`intent:<variant>` + JSON details).
/// Single canonical AUDIT-LOG path: bots count coverage themselves (rich
/// variant names), but the audit trail — what makes crashes found by ANY
/// writer minimizable via ddmin — lives here and nowhere else.
pub(crate) fn intent_audit_log_system(
    mut state: ResMut<PlaytestState>,
    mut reader: bevy::ecs::message::MessageReader<UserIntent>,
    names: Query<&Name>,
    ids: Query<&crate::identity::StableId>,
    mut action_log: Option<ResMut<crate::contract::ActionLog>>,
    mut action_effects: ResMut<crate::effects::ActionEffects>,
) {
    for intent in reader.read() {
        // FX1: remember the last frame a NON-WAIT intent was consumed
        // (for the turn-based stuck_after_intent check). Wait is a
        // deliberate no-op and must not arm the stuck check.
        if !matches!(intent, UserIntent::Wait) {
            state.last_intent_frame = Some(state.frame);
        }
        let variant = match &intent {
            UserIntent::Move { .. } => "move",
            UserIntent::Choice { .. } => "choice",
            UserIntent::Axis { .. } => "axis",
            UserIntent::Select { .. } => "select",
            UserIntent::Wait => "wait",
        };
        // I5/FX1: every audited action records a pending effect.
        // Wait is a DELIBERATE no-op — the surface variant every
        // turn-based/idle strategy emits — so it never counts as a
        // dead verb.
        if !matches!(intent, UserIntent::Wait) {
            let key = match &intent {
                // Direction-quadrant keys: a LEFT-dropping move bug
                // (walker/bug_dead_left_key) must kill only the
                // leftward verbs, not all of "intent:move".
                UserIntent::Move { dir } => format!(
                    "intent:move:{}",
                    if dir.x < 0.0 {
                        "left"
                    } else if dir.x > 0.0 {
                        "right"
                    } else if dir.y < 0.0 {
                        "down"
                    } else {
                        "up"
                    }
                ),
                UserIntent::Choice { .. } => "intent:choice".to_string(),
                UserIntent::Axis { name, .. } => format!("intent:axis:{}", name),
                UserIntent::Select { .. } => "intent:select".to_string(),
                UserIntent::Wait => unreachable!(),
            };
            action_effects.record(&key, state.frame);
        }
        // Structured JSON via serde_json: no escaping bugs, lossless
        // float encoding (f32 -> f64 -> shortest repr round-trips
        // exactly, unlike the old {:.4} truncation).
        let details = match &intent {
            UserIntent::Move { dir } => {
                Some(serde_json::json!({"dir": [dir.x, dir.y]}).to_string())
            }
            UserIntent::Choice { index } => Some(serde_json::json!({"index": index}).to_string()),
            UserIntent::Axis { name, value } => {
                Some(serde_json::json!({"name": name, "value": value}).to_string())
            }
            UserIntent::Select { target } => Some(
                match (names.get(*target), ids.get(*target)) {
                    // Named: record both name and stable_id (I1).
                    (Ok(n), Ok(&sid)) => serde_json::json!({
                        "target": n.as_str(),
                        "stable_id": sid.0,
                        "entity": target.to_bits(),
                    }),
                    // Unnamed but tracked: stable_id alone is replayable
                    // (I1) — no longer counted as unreplayable.
                    (Err(_), Ok(&sid)) => serde_json::json!({
                        "stable_id": sid.0,
                        "entity": target.to_bits(),
                    }),
                    // Genuinely untracked (non-Gameplay): diagnostics only;
                    // the replay decoder treats a missing "target"/"stable_id"
                    // as unreplayable (counted via unreplayable_actions).
                    (Err(_), Err(_)) => serde_json::json!({"entity": target.to_bits()}),
                    (Ok(n), Err(_)) => {
                        serde_json::json!({"target": n.as_str(), "entity": target.to_bits()})
                    }
                }
                .to_string(),
            ),
            UserIntent::Wait => None,
        };
        if let Some(log) = action_log.as_mut() {
            log.entries.push(crate::contract::ActionEntry {
                frame: state.frame,
                elapsed_ms: state.elapsed_s() as f64 * 1000.0,
                source: crate::contract::ActionSource::BotScenario("intent-audit".into()),
                action: format!("intent:{}", variant),
                details,
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Execution-time oracle: per-frame timing anomaly
// detection via Welford's online algorithm. Flags frames slower than
// mean + 3σ after warm-up — catches infinite-loop-in-system,
// progressive degradation (resource leak), and stalls the static
// frame_time_p99_below threshold misses. Zero game knowledge: measures
// the update loop itself.
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Readiness gate (UE PrepareTest/IsReady analog)
// ---------------------------------------------------------------------------

/// Games with async setup (asset streaming, level generation, server
/// connect) insert this resource and flip it true when their world is
/// ready for the scenario to begin. Until ready, scenario duration does
/// not accrue — avoiding flaky frame-0 assumptions. Games that don't
/// insert it get immediate readiness (frame-0 behavior preserved).
#[derive(Resource, Default)]
pub struct GameReady(pub bool);

// ---------------------------------------------------------------------------
// Actionability gate for synthetic_pointer (Playwright-style, opt-in)
// ---------------------------------------------------------------------------

/// One frame after a pointer click completes, consult the picking backend's
/// `HoverMap` to verify the pointer was over SOME entity — proof the picking
/// wiring is alive and the click wasn't masked/occluded into oblivion.
pub(crate) fn synthetic_pointer_actionability_check_system(
    mut state: ResMut<PlaytestState>,
    mut violations: ResMut<Violations>,
    hover_map: Option<Res<bevy::picking::hover::HoverMap>>,
) {
    let frame = state.frame;
    let mut still_pending = std::collections::VecDeque::new();
    while let Some(g) = state.pending_actionability_checks.pop_front() {
        // This system runs one frame AFTER release_at (release_at == frame-1).
        if g.release_at + 1 != frame {
            still_pending.push_back(g);
            continue;
        }
        if let Some(hover) = hover_map.as_ref() {
            // HoverMap keys are PointerId; values are maps from window to hovered entities.
            let mouse_hovered = hover
                .get(&PointerId::Mouse)
                .and_then(|wm| wm.values().next());
            if mouse_hovered.is_none() {
                violations.report(
                    "pointer_not_actionable",
                    &g.target_name,
                    format!(
                        "click on '{}' completed but hover map shows no entity under cursor — picking backend may be dead or target occluded",
                        g.target_name
                    ),
                    frame,
                );
            }
        } else {
            // No HoverMap resource → picking backend not wired.
            violations.report(
                "pointer_not_actionable",
                &g.target_name,
                "no HoverMap resource — picking backend not active".to_string(),
                frame,
            );
        }
    }
    state.pending_actionability_checks = still_pending;
}

/// FX7: strip ONLY generated index suffixes from entity Names — a
/// final whitespace-separated token consisting solely of ASCII digits
/// ("Enemy 01", "Enemy 02" from `format!("Enemy {i}")` naming) is
/// removed so spawned instances share one violation target label.
/// Rule (documented): digits are stripped ONLY when they form the
/// entire last space-delimited token. Semantic digits ("Area51",
/// "Level2 boss", "Enemy") are preserved verbatim — "Enemy" and
/// "Enemy 1" both map to "Enemy" (a bare name matches its indexed
/// family), but "Enemy" is never conflated with "Enemy2boss".
pub(crate) fn strip_generated_suffix(name: &str) -> &str {
    match name.rfind(' ') {
        Some(sp)
            if !name[sp + 1..].is_empty() && name[sp + 1..].chars().all(|c| c.is_ascii_digit()) =>
        {
            &name[..sp]
        }
        _ => name,
    }
}

#[cfg(test)]
mod fx7_label_tests {
    use super::strip_generated_suffix;

    #[test]
    fn generated_index_suffixes_stripped() {
        assert_eq!(strip_generated_suffix("Enemy 01"), "Enemy");
        assert_eq!(strip_generated_suffix("Enemy 02"), "Enemy");
        assert_eq!(strip_generated_suffix("Patrol Point 3"), "Patrol Point");
    }

    #[test]
    fn semantic_digits_preserved() {
        assert_eq!(strip_generated_suffix("Enemy"), "Enemy");
        assert_eq!(strip_generated_suffix("Area51"), "Area51");
        assert_eq!(strip_generated_suffix("Level2 boss"), "Level2 boss");
        assert_eq!(strip_generated_suffix("Boss v2"), "Boss v2");
    }

    #[test]
    fn enemy_bare_and_indexed_share_label() {
        assert_eq!(
            strip_generated_suffix("Enemy"),
            strip_generated_suffix("Enemy 1")
        );
    }
}
