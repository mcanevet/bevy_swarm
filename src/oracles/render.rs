//! RF38: rendering detection (headless-safe, pure ECS checks).
//!
//! Headless runs strip the render family — a game that renders nothing
//! still passes every other oracle. These checks catch the common
//! structural causes with pure ECS queries (no renderer needed):
//!
//! - `no_active_camera` (Major): after readiness, drawable entities
//!   exist (Sprite, Text2d, UI Node) but zero active `Camera`
//!   components — the window would be blank.
//! - `nothing_on_screen` (informational): drawable entities exist and
//!   a camera is active, but NONE ever projected inside the camera's
//!   viewport across every check — everything may be off-screen.
//!   Approximation: no occlusion, no alpha, no render layers.
//!
//! Both work when CameraPlugin/SpritePlugin are stripped: components
//! are queried directly (they exist as data even without the
//! plugin's systems). Honesty is preserved elsewhere: every headless
//! run lists "rendering" in `scope.unchecked` with reason
//! "headless: pixels not verified" — these rules narrow the blind
//! spot, they do not close it (pixel verification is swarm-1tw, v0.3).
//!
//! Cadence: the structural scan runs every liveness quarter (early
//! camera transforms are unreliable headless before the first check,
//! so a single early verdict can false-positive). `no_active_camera`
//! fires only if NO active camera existed at ANY check; the
//! nothing-on-screen verdict accumulates across checks and is
//! finalized at end of run (driver) so transiently off-screen sprites
//! don't flag.

use bevy::ecs::world::World;
use bevy::math::Vec2;
use bevy::prelude::*;

use crate::harness::{PlaytestState, Violations};
use crate::rules::Rule;

/// Fallback headless viewport size when the camera has no explicit
/// viewport (headless windows have a default logical resolution).
const HEADLESS_VIEWPORT: Vec2 = Vec2::new(1280.0, 720.0);

/// Structural rendering check. Runs at liveness cadence after
/// readiness (see PlaytestSet::Oracles). Updates the accumulators in
/// PlaytestState; the end-of-run finalize (driver) turns them into
/// violations.
pub(crate) fn render_check_oracle_system(world: &mut World) {
    // Runs EVERY frame from frame 1: the on-screen verdict must
    // observe the whole trajectory — a player that starts on-screen
    // then wanders off-screen proves the game renders. (No settle
    // delay: cameras are static data here; projection math needs no
    // render-loop initialization.)

    // Drawable component types, resolved to ComponentIds when the
    // types are registered in this app (feature-dependent).
    let drawables: Vec<bevy::ecs::component::ComponentId> = {
        let registry = world.components();
        let mut out = Vec::new();
        for tid in [
            std::any::TypeId::of::<bevy::sprite::Sprite>(),
            std::any::TypeId::of::<bevy::sprite::Text2d>(),
            std::any::TypeId::of::<bevy::ui::Node>(),
            #[cfg(feature = "mesh2d")]
            std::any::TypeId::of::<bevy::sprite::Mesh2d>(),
        ] {
            if let Some(cid) = registry.get_valid_id(tid) {
                out.push(cid);
            }
        }
        out
    };

    // Active cameras + drawable transforms, collected in one pass over
    // the archetypes (borrows released before touching PlaytestState).
    let cam_cid = world
        .components()
        .get_valid_id(std::any::TypeId::of::<bevy::camera::Camera>());
    let mut drawable_entities: Vec<GlobalTransform> = Vec::new();
    let mut cameras: Vec<(bevy::camera::Camera, GlobalTransform)> = Vec::new();
    for arch in world.archetypes().iter() {
        let arch_drawable = drawables.iter().any(|cid| arch.contains(*cid));
        let arch_camera = cam_cid.is_some_and(|cid| arch.contains(cid));
        if !arch_drawable && !arch_camera {
            continue;
        }
        for e in arch.entities() {
            let Some(er) = world.get_entity(e.id()).ok() else {
                continue;
            };
            if arch_drawable {
                let gt = er.get::<GlobalTransform>().copied();
                drawable_entities.push(gt.unwrap_or_default());
            }
            if arch_camera {
                if let Some(cam) = er.get::<bevy::camera::Camera>() {
                    if cam.is_active {
                        let gt = er.get::<GlobalTransform>().copied().unwrap_or_default();
                        cameras.push((cam.clone(), gt));
                    }
                }
            }
        }
    }

    // Update the accumulators.
    {
        let mut state = world.resource_mut::<PlaytestState>();
        if drawable_entities.is_empty() {
            // No drawables: a physics/logic-only app — nothing to
            // check (also silences the end-of-run reports).
            state.rendering_has_drawables = false;
            return;
        }
        state.rendering_has_drawables = true;
        if cameras.is_empty() {
            // No active camera WITH drawables present at this check.
            // The end-of-run verdict fires only if NO check ever saw
            // one (a camera spawned late is fine).
            state.rendering_seen_active_camera = false;
            return;
        }
        state.rendering_seen_active_camera = true;
    }

    // Off-screen check: remember whether ANY drawable projected
    // inside ANY camera's viewport at ANY check. A FAILED projection
    // is UNKNOWN (apps built without WindowPlugin have no viewport to
    // derive), never evidence of "off-screen".
    for (camera, cam_gt) in &cameras {
        let size = camera
            .viewport
            .as_ref()
            .map(|v| v.physical_size.as_vec2())
            .unwrap_or(HEADLESS_VIEWPORT);
        for gt in &drawable_entities {
            if let Ok(p) = camera.world_to_viewport(cam_gt, gt.translation()) {
                world
                    .resource_mut::<PlaytestState>()
                    .rendering_saw_projection = true;
                if p.x >= 0.0 && p.x < size.x && p.y >= 0.0 && p.y < size.y {
                    world
                        .resource_mut::<PlaytestState>()
                        .rendering_seen_drawable_inside = true;
                    return;
                }
            }
        }
    }
}

/// End-of-run finalize: turn the accumulated rendering-check state
/// into violations. Called from the driver after the run loop, with
/// the world still alive (camera count could have changed on the very
/// last frames — the accumulated verdicts are authoritative).
pub(crate) fn finalize_render_checks(world: &mut World) {
    let state = world.resource::<PlaytestState>();
    if !state.rendering_has_drawables {
        return;
    }
    let frame = state.frame;
    if !state.rendering_seen_active_camera {
        let mut violations = world.resource_mut::<Violations>();
        violations.report(
            Rule::NoActiveCamera,
            "rendering",
            "drawable entities present but no active camera — the game renders nothing \
             (a blank window). Headless runs cannot catch all rendering bugs: see \
             scope.unchecked 'rendering'"
                .to_string(),
            frame,
        );
        return;
    }
    if !state.rendering_seen_drawable_inside && state.rendering_saw_projection {
        let mut violations = world.resource_mut::<Violations>();
        violations.report(
            Rule::NothingOnScreen,
            "rendering",
            "drawable entities and an active camera, but none projected inside the \
             viewport — everything may be off-screen. Approximation: no occlusion, \
             alpha, or render layers (pixels are not verified headless)"
                .to_string(),
            frame,
        );
    }
}
