//! swarm-uz8: pass_through rule — detect missed colliders.
//!
//! Inference-based, genre-neutral: the rule fires only when SOME entity
//! pair in the run exhibited collision semantics (relative velocity
//! reversal while overlapping), but ANOTHER overlapping pair completed
//! a full traversal (peak penetration ≥ 60% of the smaller body, then
//! clean separation, no reversal). Games with no collision behavior
//! anywhere (crowd sims, particles, ghosts) get no verdict — that is
//! what keeps the rule genre-neutral.
//!
//! AABBs come from drawable components (Sprite custom_size with a
//! Transform scale fallback) — engine-level data, no game
//! annotations. Exclusions:
//! - entity despawn mid-overlap: pairs whose entities vanish are
//!   dropped (projectiles-on-impact, pickups)
//! - shallow grazes: corner clips and grazes never reach 60%
//!   penetration of the smaller body
//! - explicit opt-out: `collision_policy: off` silences the rule.
//!
//! Cadence: exclusive oracle at oracle cadence; per-pair trackers
//! live in PlaytestState; end-of-run finalize (driver) reports.

use bevy::ecs::entity::Entity;
use bevy::ecs::world::World;
use bevy::math::Vec2;

use crate::harness::{PlaytestState, Violations};
use crate::rules::Rule;

/// Per-pair overlap tracker.
#[derive(Clone, Debug, Default)]
pub(crate) struct PairTracker {
    /// Deepest penetration fraction (overlap extent / min body extent).
    pub(crate) max_penetration_frac: f32,
    /// Relative velocity along the dominant overlap axis at the
    /// previous check (sign flip while overlapping = collision
    /// response).
    pub(crate) prev_along: Option<f32>,
    /// Velocity reversal while overlapping (collision semantics).
    pub(crate) reversed: bool,
    /// The pair later separated after overlapping.
    pub(crate) separated: bool,
}

type Body = (Entity, Vec2, Vec2); // entity, center, half-extents

fn collect_bodies(world: &mut World) -> Vec<Body> {
    let mut out = Vec::new();
    let mut q = world.query_filtered::<(
        Entity,
        &bevy::transform::components::Transform,
        Option<&bevy::sprite::Sprite>,
    ), bevy::ecs::query::With<crate::harness::Gameplay>>();
    for (entity, transform, sprite) in q.iter(world) {
        let half = sprite
            .and_then(|s| s.custom_size)
            .map(|sz| sz / 2.0)
            .unwrap_or_else(|| Vec2::new(transform.scale.x, transform.scale.y).abs() / 2.0);
        if half.x <= 0.0 || half.y <= 0.0 {
            continue;
        }
        out.push((
            entity,
            Vec2::new(transform.translation.x, transform.translation.y),
            half,
        ));
    }
    out
}

/// Exclusive oracle: track pairwise overlaps at oracle cadence.
pub(crate) fn pass_through_oracle_system(world: &mut World) {
    {
        let scenario = world.resource::<crate::driver::ScenarioResource>();
        if scenario.0.collision_policy == crate::scenario::CollisionPolicy::Off {
            return;
        }
    }

    let mut bodies = collect_bodies(world);
    bodies.sort_by_key(|(e, _, _)| *e);
    let centers: std::collections::HashMap<Entity, Vec2> =
        bodies.iter().map(|(e, p, _)| (*e, *p)).collect();

    let prev_centers = {
        let state = world.resource::<PlaytestState>();
        state.pass_through_prev_centers.clone()
    };
    let mut state = world.resource_mut::<PlaytestState>();
    // Despawn exclusion: drop trackers for vanished entities.
    let live: std::collections::HashSet<Entity> = bodies.iter().map(|(e, _, _)| *e).collect();
    state
        .pass_through_pairs
        .retain(|(a, b), _| live.contains(a) && live.contains(b));

    for i in 0..bodies.len() {
        for j in (i + 1)..bodies.len() {
            let (e1, p1, h1) = bodies[i];
            let (e2, p2, h2) = bodies[j];
            let key = if e1 < e2 { (e1, e2) } else { (e2, e1) };
            let delta = p2 - p1;
            let rh = h1 + h2;
            let overlap_x = rh.x - delta.x.abs();
            let overlap_y = rh.y - delta.y.abs();
            let min_extent = h1.min(h2).max_element();
            let prev_rel = prev_centers
                .get(&e1)
                .zip(prev_centers.get(&e2))
                .map(|(a, b)| b - a);
            let tracker = state.pass_through_pairs.entry(key).or_default();

            if overlap_x <= 0.0 || overlap_y <= 0.0 {
                // Separated after overlap: together with deep peak
                // penetration and no reversal this is the
                // full-traversal signature (entered one side, exited
                // the other).
                tracker.separated = true;
                continue;
            }

            let penetration_frac = (overlap_x.max(overlap_y) / min_extent).clamp(0.0, 1.0);
            tracker.max_penetration_frac = tracker.max_penetration_frac.max(penetration_frac);

            // Relative velocity from consecutive pair configurations.
            if let Some(prev_rel) = prev_rel {
                let rel = p2 - p1;
                let along = if overlap_x < overlap_y { rel.x } else { rel.y };
                let prev_along = if overlap_x < overlap_y {
                    prev_rel.x
                } else {
                    prev_rel.y
                };
                let _ = prev_along;
                // Sign flip while overlapping = collision response
                // (approach, then bounce apart).
                if tracker
                    .prev_along
                    .is_some_and(|pa| pa * along < 0.0 && penetration_frac > 0.05)
                {
                    tracker.reversed = true;
                }
                tracker.prev_along = Some(along);
            }
        }
    }
    state.pass_through_prev_centers = centers;
    let _ = prev_centers;
}

/// End-of-run finalize: report pairs that fully traversed while the
/// run had collision semantics elsewhere.
pub(crate) fn finalize_pass_through(world: &mut World) {
    {
        let scenario = world.resource::<crate::driver::ScenarioResource>();
        if scenario.0.collision_policy == crate::scenario::CollisionPolicy::Off {
            return;
        }
    }
    let state = world.resource::<PlaytestState>();
    let frame = state.frame;
    let any_collision = state.pass_through_pairs.values().any(|t| t.reversed);
    if !any_collision {
        return;
    }
    let traversals: Vec<((Entity, Entity), f32)> = state
        .pass_through_pairs
        .iter()
        .filter(|(_, t)| t.separated && !t.reversed && t.max_penetration_frac > 0.6)
        .map(|(k, t)| (*k, t.max_penetration_frac))
        .collect();
    let mut violations = world.resource_mut::<Violations>();
    for ((a, b), frac) in traversals {
        violations.report(
            Rule::PassThrough,
            "collision",
            format!(
                "entity {a:?} fully traversed entity {b:?} (peak penetration {:.0}% of \
                 body size, no velocity reversal) while other pairs collided — a \
                 collider may be missing. Silence with collision_policy: off if these \
                 entities are intangible by design",
                frac * 100.0
            ),
            frame,
        );
    }
}
