//! Oracles: per-frame world checks and the custom-invariant engine.

use bevy::ecs::entity::Entity;
use bevy::ecs::query::{Changed, With};
use bevy::ecs::resource::Resource;
use bevy::ecs::system::{Query, Res, ResMut};
use bevy::ecs::world::World;
use bevy::picking::pointer::PointerId;
use bevy::prelude::{Name, Transform};

use crate::contract::{Gameplay, TestApi, TestApiResolve, TestFieldValue, UserIntent};
use crate::scenario::*;
use crate::state::*;

use crate::driver::ScenarioResource;

// ---------------------------------------------------------------------------
// Invariants
// ---------------------------------------------------------------------------

pub(crate) fn check_finite_transforms_system(
    q: Query<(Entity, &Transform), Changed<Transform>>,
    mut violations: ResMut<Violations>,
    state: Res<PlaytestState>,
) {
    for (entity, transform) in q.iter() {
        let t = transform.translation;
        if !t.x.is_finite() || !t.y.is_finite() || !t.z.is_finite() {
            violations.report(
                "nodes_finite",
                &format!("entity:{}", entity),
                format!("non-finite translation ({}, {}, {})", t.x, t.y, t.z),
                state.frame,
            );
        }
    }
}

/// Shared bounds predicate: x/y always; z only when a z bound is declared
/// (check z for 3D nodes — don't silently drop it).
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
    q_changed: Query<(&Name, &Transform), (With<Gameplay>, Changed<Transform>)>,
    q_all: Query<(&Name, &Transform), With<Gameplay>>,
    mut violations: ResMut<Violations>,
    state: Res<PlaytestState>,
    scenario: Res<ScenarioResource>,
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
                if out_of_bounds(transform.translation, inv) {
                    violations.report(
                        &inv.name,
                        name.as_ref(),
                        format!(
                            "out of bounds ({}, {}, {})",
                            transform.translation.x,
                            transform.translation.y,
                            transform.translation.z
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
                let matched_changed: Vec<(&Name, &Transform)> = q_changed
                    .iter()
                    .filter(|(n, _)| n.as_str() == target)
                    .collect();
                let matched_all: Vec<(&Name, &Transform)> =
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
                    if out_of_bounds(transform.translation, inv) {
                        violations.report(
                            &inv.name,
                            name.as_ref(),
                            format!(
                                "out of bounds ({}, {}, {})",
                                transform.translation.x,
                                transform.translation.y,
                                transform.translation.z
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
///   `Component:<TypePath>#count`      — count of entities with component
///   `Component:<TypePath>[N].<field>` — Nth entity's component field
///   `Component:<TypePath>.<field>`    — first entity's component field
///
/// Enums resolve to variant name as Text. Numeric leaf types → Numeric.
pub fn resolve_world_percept(world: &World, path: &str) -> Option<TestFieldValue> {
    use bevy::ecs::reflect::AppTypeRegistry;
    use bevy::reflect::GetPath;

    let registry = world.resource::<AppTypeRegistry>().0.read();

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
        // Field: "Component:<TypePath>.<field>" or "Component:<TypePath>[<i>].<field>"
        let (type_path, idx, field) = if let Some(bracket_end) = rest.find('[') {
            let idx_start = bracket_end + 1;
            let idx_end = rest[idx_start..].find(']')? + idx_start;
            let idx: usize = rest[idx_start..idx_end].parse().ok()?;
            let field = rest.get(idx_end + 1..)?.strip_prefix('.')?;
            (&rest[..bracket_end], idx, field)
        } else {
            let dot_idx = rest.find('.')?;
            (&rest[..dot_idx], 0usize, &rest[dot_idx + 1..])
        };

        let comp_id = comp_id_for(type_path)?;
        let reflect_component = registry
            .get_with_short_type_path(type_path)?
            .data::<bevy::ecs::reflect::ReflectComponent>()?;
        let mut seen = 0usize;
        for entity in world.iter_entities() {
            if entity.contains_id(comp_id) {
                if seen == idx {
                    let reflect = reflect_component.reflect(entity)?;
                    let val = reflect.reflect_path(field).ok()?;
                    return Some(value_from_reflect(val));
                }
                seen += 1;
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
    use bevy::ecs::reflect::AppTypeRegistry;

    let registry = world.resource::<AppTypeRegistry>().0.read();

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

fn resolve_test_api(api: &TestApi, path: &str) -> (Option<f64>, Option<String>) {
    match api.resolve(path) {
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
    let api = world.resource::<TestApi>().clone();

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
            let Some(count) = count_query_target(world, query) else {
                world.resource_mut::<Violations>().report(
                    &inv.name,
                    &format!("query({:?})", query),
                    "unresolved component type in query (typo?)".into(),
                    frame,
                );
                continue;
            };
            let check = inv.check.unwrap_or(crate::enums::CheckOp::Ge);
            let threshold = inv.value.as_ref().and_then(|v| v.as_f64());

            let holds_now = match threshold {
                Some(thr) => check.holds(count as f64, thr),
                None => false,
            };

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
        let (numeric, string_val) = match resolve_test_api(&api, path) {
            (Some(n), s) => (Some(n), s),
            (None, Some(s)) => (None, Some(s)),
            // TestApi miss → try world reflection percept (generic layer).
            (None, None) => match resolve_world_percept(world, path) {
                Some(TestFieldValue::Numeric(n)) => (Some(n), None),
                Some(TestFieldValue::Text(s)) => (None, Some(s)),
                None => (None, None),
            },
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

        // Equals on enum-ish (string) fields.
        if let (Some(sv), Some(serde_json::Value::String(expect))) = (&string_val, &inv.value) {
            let eq = sv == expect;
            // Eventually-mode: pass as soon as satisfied; report only at
            // deadline expiry if never satisfied.
            if let Some(deadline) = inv.eventually_s {
                let mut state_mut = world.resource_mut::<PlaytestState>();
                let entry = state_mut
                    .eventually_state
                    .entry(inv.name.clone())
                    .or_insert((deadline, None));
                if eq {
                    entry.1 = Some(entry.1.unwrap_or(frame)); // first satisfaction
                } else if elapsed_s >= deadline && entry.1.is_none() {
                    world.resource_mut::<Violations>().report(
                        &inv.name,
                        path,
                        format!(
                            "{} never equaled {} within {:.1}s (eventually deadline expired)",
                            path, expect, deadline
                        ),
                        frame,
                    );
                }
            } else if !eq {
                world.resource_mut::<Violations>().report(
                    &inv.name,
                    path,
                    format!("{} = {} (expected: {})", path, sv, expect),
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
            samples.push((frame, current));
            let window_ticks = tps.max(1);
            while samples.len() > 1 && frame - samples[0].0 > window_ticks {
                samples.remove(0);
            }
            let oldest = samples[0];
            let dt_ticks = frame - oldest.0;
            // Evaluate only once the window holds a full physics second —
            // shorter spans measure rates with warm-up skew.
            if dt_ticks >= window_ticks {
                let rate = (current - oldest.1).abs() / (dt_ticks as f64 / tps as f64);
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
                    // Text WHEN: string equality against the field value.
                    string_val
                        .as_deref()
                        .and_then(|sv| when_val.as_str().map(|wv| sv == wv))
                        .unwrap_or(false)
                }
            } else {
                true
            };
            if when_holds {
                let (req_num, req_str) = resolve_test_api(&api, req_path);
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
    state: Res<PlaytestState>,
    mut reader: bevy::ecs::message::MessageReader<UserIntent>,
    names: Query<&Name>,
    mut action_log: Option<ResMut<crate::contract::ActionLog>>,
) {
    for intent in reader.read() {
        let variant = match &intent {
            UserIntent::Move { .. } => "move",
            UserIntent::Choice { .. } => "choice",
            UserIntent::Axis { .. } => "axis",
            UserIntent::Select { .. } => "select",
            UserIntent::Wait => "wait",
        };
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
                match names.get(*target) {
                    Ok(n) => serde_json::json!({"target": n.as_str(), "entity": target.to_bits()}),
                    // No Name: keep the bits for diagnostics; the replay
                    // decoder treats a missing "target" as unreplayable
                    // (counted via unreplayable_actions).
                    Err(_) => serde_json::json!({"entity": target.to_bits()}),
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

// ---------------------------------------------------------------------------

/// Detects a "frozen world" — no Gameplay entity's Transform changed for
/// 2+ seconds while a real-time scenario runs. Turn-based games are
/// exempt: if any entity carries the `TurnBased` marker component,
/// waiting between turns is by design, not a soft-lock.
/// Zero contract: pure ECS change detection, O(changes) per tick.
pub(crate) fn frozen_world_oracle_system(
    q_changed: Query<(), (With<crate::contract::Gameplay>, Changed<Transform>)>,
    q_turn_based: Query<(), With<crate::contract::TurnBased>>,
    mut state: ResMut<PlaytestState>,
    mut violations: ResMut<Violations>,
) {
    // Turn-based exemption: legitimate long pauses between turns.
    if q_turn_based.iter().next().is_some() {
        return;
    }
    if q_changed.iter().next().is_some() {
        state.frozen_frames = 0;
        return;
    }
    state.frozen_frames += 1;
    let threshold = state.tps.saturating_mul(2);
    if state.frozen_frames == threshold {
        violations.report(
            "frozen_world",
            "world",
            format!(
                "no Gameplay entity changed for 2.0s ({} frames) — world may be stuck (soft-lock)",
                state.frozen_frames
            ),
            state.frame,
        );
    }
}

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
