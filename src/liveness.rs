//! I4: Change-tick liveness — generalised frozen-world detection.
//!
//! Liveness = "gameplay-relevant state changed", measured by Bevy's
//! change ticks, not only Transforms. Replaces the blanket TurnBased
//! exemption: turn-based games are checked `after_intent` instead.

use bevy::ecs::change_detection::Tick;
use bevy::ecs::world::World;
use bevy::prelude::*;

use crate::contract::Gameplay;
use crate::driver::ScenarioResource;
use crate::harness::{PlaytestState, Violations};

/// True if any component of any Gameplay entity changed in the window
/// `(since, now]`. Scans entities, so callers run it at liveness
/// cadence (every tps/4 frames), stopping at the first change.
pub fn gameplay_changed_since(world: &World, since: Tick, now: Tick) -> bool {
    let Some(gameplay) = world
        .components()
        .get_valid_id(std::any::TypeId::of::<Gameplay>())
    else {
        return false;
    };
    for arch in world
        .archetypes()
        .iter()
        .filter(|a| a.contains(gameplay) && !a.is_empty())
    {
        for e in arch.entities() {
            let er = world.get_entity(e.id()).ok().unwrap();
            for cid in arch.components() {
                if let Some(t) = er.get_change_ticks_by_id(*cid) {
                    if t.is_changed(since, now) {
                        return true;
                    }
                }
            }
        }
    }
    false
}

/// True if a resource changed in the window `(since, now]` (generic
/// form; the I4/I5 shared `changed_since(world, scope, tick)` helper).
pub fn resource_changed_since<R: Resource>(world: &World, since: Tick, now: Tick) -> bool {
    world
        .get_resource_change_ticks::<R>()
        .is_some_and(|t| t.is_changed(since, now))
}

/// Snapshot of the world's current change tick.
pub fn now_tick(world: &World) -> Tick {
    world.read_change_tick()
}

/// The liveness oracle (I4). Real-time: violation `frozen_world` if no
/// liveness signal for `liveness.timeout_s` (default 2s). Turn-based
/// (marker present or mode `after_intent`): idle waiting is fine, but
/// after an intent some liveness signal must occur within timeout_s,
/// else `stuck_after_intent`. Mode `off` disables the oracle (recorded
/// in report warnings by the driver).
pub(crate) fn liveness_oracle_system(world: &mut World) {
    let Some(scenario_res) = world.get_resource::<ScenarioResource>() else {
        return;
    };
    let liveness = scenario_res.0.liveness.clone();
    if matches!(liveness.mode, crate::scenario::LivenessMode::Off) {
        return;
    }
    let (frame, tps, last_intent_frame) = {
        let state = world.resource::<PlaytestState>();
        (state.frame, state.tps, state.last_intent_frame)
    };
    let timeout_frames = (liveness.timeout_s * tps as f32) as u64;
    let turn_based = matches!(liveness.mode, crate::scenario::LivenessMode::AfterIntent)
        || world
            .query_filtered::<(), With<crate::contract::TurnBased>>()
            .iter(world)
            .next()
            .is_some();

    // Evaluate the liveness signal at tps/4 cadence (quarter-second).
    let quarter = (tps / 4).max(1);
    if !frame.is_multiple_of(quarter) {
        return;
    }

    let now = now_tick(world);
    // Liveness window: everything since the previous liveness check.
    let since = Tick::new(now.get().saturating_sub((quarter * 2) as u32));
    // Default liveness scope: components on Gameplay entities. The
    // TestApi resource is deliberately EXCLUDED by default: games
    // commonly sync it every frame (write without value change), which
    // would mask a genuinely frozen world.
    let changed = gameplay_changed_since(world, since, now);

    {
        let state = &mut world.resource_mut::<PlaytestState>();
        if changed {
            state.frozen_frames = 0;
            state.last_alive_frame = frame;
        } else {
            state.frozen_frames += quarter;
        }
    }

    if !turn_based {
        // Real-time: contiguous dead time crosses the threshold → frozen.
        if state_frozen(world) >= timeout_frames && timeout_frames > 0 {
            let frozen = state_frozen(world);
            world.resource_mut::<Violations>().report(
                "frozen_world",
                "world",
                format!(
                    "no gameplay-relevant state changed for {:.1}s ({} frames) — world may be stuck (soft-lock)",
                    liveness.timeout_s,
                    frozen
                ),
                frame,
            );
        }
    } else {
        // Turn-based: only require liveness AFTER an intent was consumed.
        if let Some(last_intent) = last_intent_frame {
            let idle_since_intent = frame.saturating_sub(last_intent);
            let alive_after_intent = {
                let state = world.resource::<PlaytestState>();
                state.last_alive_frame >= last_intent
            };
            if !alive_after_intent && idle_since_intent >= timeout_frames {
                world.resource_mut::<Violations>().report(
                    "stuck_after_intent",
                    "world",
                    format!(
                        "no gameplay-relevant state changed for {:.1}s ({} frames) after the intent at frame {} — the game may be ignoring input or soft-locked in a turn",
                        liveness.timeout_s,
                        idle_since_intent,
                        last_intent
                    ),
                    frame,
                );
            }
        }
    }
}

fn state_frozen(world: &World) -> u64 {
    world.resource::<PlaytestState>().frozen_frames
}

/// Convenience: full `changed_since(world, scope, tick)` used by I4/I5.
/// Scope: gameplay components + the TestApi resource (game-owned
/// resources arrive with Z6).
pub fn changed_since(world: &World, since: Tick) -> bool {
    let now = now_tick(world);
    gameplay_changed_since(world, since, now)
        || resource_changed_since::<crate::contract::TestApi>(world, since, now)
}
