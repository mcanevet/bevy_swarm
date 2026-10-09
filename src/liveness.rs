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
        // FX12 (I4): liveness off must be LOUD — recorded in
        // report.warnings (surfaced by the driver) so a disabled
        // oracle is never mistaken for a green run.
        world.resource_mut::<PlaytestState>().warned_liveness_off = true;
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
    // FX1: the liveness WINDOW is the timeout itself — "nothing changed
    // for timeout_s" is tested directly against the frame-start tick
    // recorded timeout_frames ago. A component mutated only on odd
    // frames still lands inside the window (unit-tested).
    let window_frames = timeout_frames.max(quarter * 2);

    let now = now_tick(world);
    // FX1: window in FRAMES, not raw change ticks (the tick counter
    // advances per system run, dozens per frame, so `now - k` spans
    // microseconds). Compare against the frame-start tick recorded
    // `window_frames` frames ago.
    //
    // Default liveness scope: components on Gameplay entities. The
    // TestApi resource is deliberately EXCLUDED by default: games
    // commonly sync it every frame (write without value change), which
    // would mask a genuinely frozen world.
    //
    // Warmup: insufficient frame history → treat as LIVE (never flags
    // a young world frozen).
    let changed = {
        let state = world.resource::<PlaytestState>();
        match state
            .frame_start_ticks
            .iter()
            .rev()
            .nth(window_frames.saturating_sub(1) as usize)
        {
            // FX1: liveness scope includes game-owned RESOURCES (Z6
            // GameTypes prefixes) — fixtures like regression_pair keep
            // all state in resources; entity-component scanning alone
            // saw a permanently frozen world. TestApi stays EXCLUDED
            // (games sync it every frame; that masks real freezes).
            Some(&t) => {
                let since = Tick::new(t);
                gameplay_changed_since(world, since, now)
                    || game_resource_changed_since(world, since, now)
            }
            None => true,
        }
    };

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
        // Real-time: the FULL timeout window observed with zero liveness
        // → frozen. (changed=false above means nothing changed in the
        // last window_frames = timeout_s of frames.)
        if !changed && timeout_frames > 0 {
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
            // FX1: "alive after the intent" must mean state CHANGED
            // after the intent's frame STARTED (same-frame reactions
            // count — bots run in PreUpdate, games react in Update).
            // Comparing frame numbers counted a change that happened
            // EARLIER in the intent's own frame (false pass).
            let alive_after_intent = {
                let state = world.resource::<PlaytestState>();
                match frame_start_tick_of(state, last_intent) {
                    Some(t) => {
                        let since = Tick::new(t);
                        gameplay_changed_since(world, since, now)
                            || game_resource_changed_since(world, since, now)
                    }
                    // history evicted — fall back to the frame heuristic
                    None => state.last_alive_frame >= last_intent,
                }
            };
            // Accumulate a STUCK STREAK: consecutive no-reaction quanta.
            // A softlock that swallows every intent keeps the bot
            // retrying — idle_since_intent alone would reset on every
            // attempt and never reach the timeout. The streak counts
            // time since the LAST successful reaction to an intent.
            {
                let state = &mut world.resource_mut::<PlaytestState>();
                if alive_after_intent {
                    state.stuck_after_intent_quanta = 0;
                } else {
                    state.stuck_after_intent_quanta += quarter;
                }
            }
            let stuck_quanta = world.resource::<PlaytestState>().stuck_after_intent_quanta;
            if !alive_after_intent && stuck_quanta >= timeout_frames && idle_since_intent >= quarter
            {
                world.resource_mut::<Violations>().report(
                    "stuck_after_intent",
                    "world",
                    format!(
                        "no gameplay-relevant state changed for {:.1}s ({} frames) after the intent at frame {} — the game may be ignoring input or soft-locked in a turn",
                        liveness.timeout_s,
                        stuck_quanta,
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

/// The change tick snapshotted at the START of frame `target`
/// (bots emit in PreUpdate; a game reacting to that intent changes
/// state strictly after this tick).
fn frame_start_tick_of(state: &PlaytestState, target: u64) -> Option<u32> {
    let len = state.frame_start_ticks.len() as u64;
    let current = state.frame;
    if target == 0 || target > current || current - target >= len {
        return None;
    }
    let back = (current - target) as usize;
    state.frame_start_ticks.iter().rev().nth(back).copied()
}

/// True if any GAME-OWNED resource changed in `(since, now]` — Z6
/// GameTypes crate prefixes; engine resources (bevy_transform etc.)
/// churn every frame and would mask a frozen world.
pub fn game_resource_changed_since(world: &World, since: Tick, now: Tick) -> bool {
    // FX12 (I4): explicitly WATCHED resources (liveness.watch) count
    // as liveness signals regardless of GameTypes prefixes.
    let watched: Vec<String> = world
        .get_resource::<ScenarioResource>()
        .map(|s| s.0.liveness.watch.clone())
        .unwrap_or_default();
    if !watched.is_empty() {
        for (id, info, _) in world.iter_resources() {
            let name = info.name().to_string();
            if watched.iter().any(|w| name.contains(w.as_str())) {
                if let Some(ticks) = world.get_resource_change_ticks_by_id(id) {
                    if ticks.is_changed(since, now) {
                        return true;
                    }
                }
            }
        }
    }
    let prefixes: Vec<String> = world
        .get_resource::<crate::game_types::GameTypes>()
        .map(|gt| gt.prefixes.clone())
        .unwrap_or_default();
    if prefixes.is_empty() {
        return false;
    }
    for (id, info, _) in world.iter_resources() {
        let name = info.name().to_string();
        if !prefixes.iter().any(|p| name.starts_with(p.as_str())) {
            continue;
        }
        if let Some(ticks) = world.get_resource_change_ticks_by_id(id) {
            if ticks.is_changed(since, now) {
                return true;
            }
        }
    }
    false
}

/// Convenience: full `changed_since(world, scope, tick)` used by I4/I5
/// EFFECTS (not liveness): gameplay components + game-owned resources.
/// TestApi is EXCLUDED (effect detection must not count harness writes).
pub fn changed_since(world: &World, since: Tick) -> bool {
    let now = now_tick(world);
    gameplay_changed_since(world, since, now) || game_resource_changed_since(world, since, now)
}
