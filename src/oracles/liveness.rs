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

/// True if any NON-REFLECTED game component on a Gameplay entity
/// changed in `(since, now]` (RF43 moderate fallback). Digest-visible
/// (reflected, non-engine) components are EXCLUDED — for those,
/// value semantics rule (same-value re-writes are NOT liveness).
/// Non-reflected game components are invisible to the digest, so a
/// tick change is the best available signal.
pub fn non_reflected_gameplay_changed_since(world: &World, since: Tick, now: Tick) -> bool {
    // RF43 moderate: components invisible to the value digest (plain
    // #[derive(Component)] game types, never registered in the type
    // registry) fall back to change-tick semantics. Identified by
    // COMPONENT NAME: not bevy/bevy_playtest (those are engine/harness —
    // e.g. GlobalTransform is re-propagated every frame) and not
    // present in the reflect registry (those follow value semantics
    // via the digest; re-written-same-value must stay frozen).
    let reflected: std::collections::HashSet<bevy::ecs::component::ComponentId> = {
        let mut set = std::collections::HashSet::new();
        if let Some(registry_arc) = world.get_resource::<bevy::ecs::reflect::AppTypeRegistry>() {
            let registry = registry_arc.0.read();
            for type_reg in registry.iter() {
                if type_reg
                    .data::<bevy::ecs::reflect::ReflectComponent>()
                    .is_none()
                {
                    continue;
                }
                let Some(tid) = registry
                    .get_with_short_type_path(type_reg.type_info().type_path_table().short_path())
                else {
                    continue;
                };
                if let Some(cid) = world.components().get_valid_id(tid.type_id()) {
                    set.insert(cid);
                }
            }
        }
        set
    };
    let is_game_component =
        |cid: &bevy::ecs::component::ComponentId| match world.components().get_info(*cid) {
            Some(info) => {
                let name = info.name();
                !(name.starts_with("bevy_")
                    || name.starts_with("bevy::")
                    || name.starts_with("bevy_playtest"))
                    && !reflected.contains(cid)
            }
            None => false,
        };
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
            let Ok(er) = world.get_entity(e.id()) else {
                continue;
            };
            for cid in arch.components() {
                if !is_game_component(cid) {
                    continue;
                }
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
    // le3.23 (RF27): VALUE-based liveness. The old change-tick scan
    // counted same-value RE-writes as "alive" (a system assigning an
    // unchanged transform every frame kept a frozen world live —
    // same-value-write blindness). Compare the canonical state digest
    // instead: only actual VALUE changes reset the freeze timer.
    // Change ticks remain the cheap pre-filter: if nothing was even
    // WRITTEN in the window, the world is trivially unchanged.
    let (written, (), anchor_t) = {
        let state = world.resource::<PlaytestState>();
        // Anchor on the tick from `window_frames` ago; if history is
        // shorter than the window (young run / long timeout), anchor on
        // the OLDEST recorded frame instead — a partial window beats
        // no evaluation at all (skipping entirely left short runs
        // unevaluated and last_alive_frame permanently 0).
        let anchor = state
            .frame_start_ticks
            .iter()
            .rev()
            .nth(window_frames.saturating_sub(1) as usize)
            .or_else(|| state.frame_start_ticks.front());
        match anchor {
            Some(&t) => {
                let since = Tick::new(t);
                (
                    gameplay_changed_since(world, since, now)
                        || game_resource_changed_since(world, since, now),
                    (),
                    t,
                )
            }
            // No history at all (first frames) → treat as LIVE
            // (never flags a young world frozen).
            None => return,
        }
    };
    let changed = if !written {
        false
    } else {
        // Something was written — but was the VALUE actually different?
        // If there's no observable state (no Gameplay entities, no
        // GameTypes), fall back to change-tick semantics to avoid
        // false positives on games with no tracked state.
        let has_observed = world.iter_entities().any(|e| {
            e.contains::<crate::contract::Gameplay>()
                || e.contains::<crate::game_types::InferredGameplay>()
        }) || world
            .get_resource::<crate::game_types::GameTypes>()
            .is_some();
        if !has_observed {
            true
        } else {
            // Census rows excluded: archetype-topology churn (harness
            // spawning observers mid-run) is not game state and would
            // falsely mark a frozen world alive.
            let digest = {
                let rows = crate::determinism::canonical_state(world);
                let mut m = std::collections::BTreeMap::new();
                for (k, v) in rows {
                    if !k.starts_with("census.") {
                        m.insert(k, v);
                    }
                }
                format!("{:?}", m)
            };
            let state = world.resource::<PlaytestState>();
            if digest != state.liveness_digest {
                let first_observation = state.liveness_digest.is_empty();
                world.resource_mut::<PlaytestState>().liveness_digest = digest;
                !first_observation
            } else {
                // Digest unchanged — fall back to change-tick semantics
                // for NON-REFLECTED gameplay components only (RF43
                // moderate). A plain #[derive(Component)] mutated every
                // frame is invisible to the digest but must count as
                // alive. Reflected components stay under value
                // semantics (same-value re-writes are not liveness).
                non_reflected_gameplay_changed_since(world, Tick::new(anchor_t), now)
            }
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
        // last window_frames = timeout_s of frames.) Fire only when the
        // frame history actually COVERS the full window AND the
        // accumulated freeze count reaches timeout_frames — a single
        // quiet quarter in an otherwise live run must not fire.
        let full_window = {
            let state = world.resource::<PlaytestState>();
            state.frame_start_ticks.len() as u64 >= window_frames
        };
        let state = world.resource::<PlaytestState>();
        if !changed && timeout_frames > 0 && full_window && state.frozen_frames >= timeout_frames {
            // RF8: if the actuator produced ZERO effective inputs, the
            // game is undrivable, not necessarily buggy. Downgrade to
            // informational.
            let effect_rate = crate::effects::overall_effect_rate(world).unwrap_or(1.0);
            // le3.22 (RF26): downgrade to undrivable_game only when the world
            // WAS live after readiness (last_alive_frame > 0). A world that
            // never responds from frame 0 is a pure soft-lock → FrozenWorld.
            // Exception: a world with NO observable gameplay state at all
            // (no Gameplay entities, no GameTypes) can't meaningfully be
            // "stuck" — there is nothing to freeze. Zero-effect inputs on
            // such a world are undrivable-game territory (RF8).
            let was_live_after_readiness = world.resource::<PlaytestState>().last_alive_frame > 0;
            // Observable = actual digest-visible state, not merely
            // registered types: the harness auto-registers GameTypes
            // prefixes even for a game with no state at all. A world
            // whose canonical digest holds nothing beyond the census
            // has nothing to freeze.
            let digest_rows = crate::determinism::canonical_state(world)
                .keys()
                .filter(|k| !k.starts_with("census."))
                .count();
            let has_observable = digest_rows > 0;
            let rule = if effect_rate == 0.0 && (was_live_after_readiness || !has_observable) {
                crate::rules::Rule::UndrivableGame
            } else {
                crate::rules::Rule::FrozenWorld
            };
            let frozen = state_frozen(world);
            world.resource_mut::<Violations>().report(
                rule,
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
            // (le3.8: idle_since_intent intentionally unused — see below)
            // FX1: "alive after the intent" must mean state CHANGED
            // after the intent's frame STARTED (same-frame reactions
            // count — bots run in PreUpdate, games react in Update).
            // Comparing frame numbers counted a change that happened
            // EARLIER in the intent's own frame (false pass).
            let (alive_after_intent, history_evicted) = {
                let state = world.resource::<PlaytestState>();
                match frame_start_tick_of(state, last_intent) {
                    Some(t) => {
                        let since = Tick::new(t);
                        (
                            gameplay_changed_since(world, since, now)
                                || game_resource_changed_since(world, since, now),
                            false,
                        )
                    }
                    // History evicted — fall back to the weaker frame
                    // heuristic. x4k: this should now be unreachable in
                    // steady state (the ring cap scales with scenario
                    // tps), but stay loud if it ever regresses: the
                    // fallback only approximates liveness.
                    None => (state.last_alive_frame >= last_intent, true),
                }
            };
            if history_evicted {
                let mut state = world.resource_mut::<PlaytestState>();
                state.warned_liveness_history_evicted = true;
            }
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
            // le3.8 (RF4): remove idle_since_intent >= quarter check.
            // A bot that retries every 15 frames defeats the old guard
            // — the softlock is invisible because idle_since_intent
            // never accumulates. The STICKY STREAK (stuck_quanta) is
            // the correct measure: if no reaction occurs for timeout_s
            // despite intents arriving, the game is softlocked.
            let stuck_quanta = world.resource::<PlaytestState>().stuck_after_intent_quanta;
            if !alive_after_intent && stuck_quanta >= timeout_frames {
                world.resource_mut::<Violations>().report(
                    crate::rules::Rule::StuckAfterIntent,
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
