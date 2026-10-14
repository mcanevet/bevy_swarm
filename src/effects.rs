//! I5: Action-effect oracle (actuator-agnostic) — "did this input DO
//! anything?" plus per-action effect rates.
//!
//! Every audited action (intents, Z3 raw actions, Z5 widget activations)
//! records a `PendingEffect`. After `effect_window_frames` (default
//! tps/2) an effect = any gameplay-state change after the action's tick
//! (I4's `changed_since` helper). Rates feed `dead_verb` (an action key
//! with 0% effect over ≥10 samples) and Z1's vacuity guard.

use bevy::ecs::world::World;
use bevy::prelude::*;
use std::collections::HashMap;

use crate::state::{PlaytestState, Violations};

/// Minimum samples before an action key can be declared a dead verb.
pub const DEAD_VERB_MIN_SAMPLES: u64 = 10;

/// RF8: verb family for sibling-drivability grouping. Arrow keys form
/// one family, WASD letters another; everything else is individual.
pub fn verb_family(key: &str) -> String {
    // Strip the actuator prefix ("raw:key:KeyW", "intent:axis:x").
    let bare = key.rsplit(':').next().unwrap_or(key);
    if bare.starts_with("Arrow") {
        "arrow".to_string()
    } else if matches!(bare, "KeyW" | "KeyA" | "KeyS" | "KeyD") {
        "wasd".to_string()
    } else {
        bare.to_string()
    }
}

/// A recorded action awaiting its effect window to elapse.
#[derive(Clone, Debug)]
pub struct PendingEffect {
    /// Action key, e.g. "intent:move", "raw:key:ArrowLeft",
    /// "raw:click_entity", "widget:Button#main".
    pub action_key: String,
    /// Simulation frame at emission.
    pub frame: u64,
    /// World change tick at emission.
    pub tick: u32,
}

/// Per-key effect statistics embedded in the report.
#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
pub struct EffectRate {
    pub effective: u64,
    pub total: u64,
}

/// I5 oracle state: pending effects + accumulated rates.
#[derive(bevy::ecs::prelude::Resource, Default)]
pub struct ActionEffects {
    pub pending: Vec<PendingEffect>,
    pub rates: HashMap<String, EffectRate>,
    /// Change tick at the START of each frame (maintained by the
    /// oracle; actuators only know their frame).
    pub tick_by_frame: HashMap<u64, u32>,
    /// Keys explicitly declared by the game/scenario (scenario
    /// `bot.raw_surface` keys). Only declared keys (or keys whose
    /// verb-group siblings DO have effects) gate dead_verb at tier 0 —
    /// a guessed default-surface key with no effect is expected when
    /// the game simply doesn't bind it (RF8).
    pub declared_keys: std::collections::HashSet<String>,
}

impl ActionEffects {
    /// Record an emitted action (call from every actuator). The
    /// effect window anchors on the world change tick captured at
    /// this frame's start (see the oracle).
    pub fn record(&mut self, action_key: &str, frame: u64) {
        self.pending.push(PendingEffect {
            action_key: action_key.to_string(),
            frame,
            tick: 0,
        });
    }
}

/// Effect scope: game-owned state by default (Z6 Gameplay components +
/// TestApi resource via I4's helper). Optional narrowing arrives with
/// scenario "effects": {"scope": [...]}.
/// The I5 oracle: retire pending effects whose window elapsed, count
/// effective/total per key, report `dead_verb` for keys with 0%
/// effectiveness over >= DEAD_VERB_MIN_SAMPLES samples (Major).
pub(crate) fn action_effect_oracle_system(world: &mut World) {
    let (frame, tps, liveness_timeout_s) = {
        let state = world.resource::<PlaytestState>();
        let timeout_s = world
            .resource::<crate::driver::ScenarioResource>()
            .0
            .liveness
            .timeout_s;
        (state.frame, state.tps, timeout_s)
    };
    // FX1: anchor each action's effect window on the change tick
    // snapshotted at the START of the action's frame (before bots and
    // game systems ran) — recorded by tick_counter_system into
    // PlaytestState. The previous late-snapshot (taken in Last, AFTER
    // the game reacted) counted the reaction itself as pre-action
    // state, marking every action ineffective.
    // FX1: snapshot digests for value-based effect attribution.
    let frame_digests: HashMap<u64, (u32, String)> = {
        let state = world.resource::<PlaytestState>();
        state
            .effect_window_snapshots
            .iter()
            .map(|&(f, t, ref d)| (f, (t, d.clone())))
            .collect()
    };

    // FX1: reaction window. Attribution must not span MULTIPLE actions:
    // at the bot's input rate a 30-frame (tps/2) window overlaps the
    // next several intents, letting a live neighbor mask a dead verb
    // (walker/bug_dead_left_key scored 10/16 effective). Cap the window
    // below the bot's inter-action interval: reactions in a frame-based
    // game land within a couple frames; the default is the tighter of
    // tps/10 and half the inter-fire interval.
    let rate = world
        .resource::<crate::driver::ScenarioResource>()
        .0
        .bot
        .input_rate_hz
        .max(1) as f32;
    let interval = (tps as f32 / rate).max(1.0);
    let window = ((tps as f32 / 10.0).min(interval / 2.0)).floor().max(1.0) as u64;
    if window == 0 {
        return;
    }
    let now_tick = world.read_change_tick().get();
    let _ = now_tick;
    let mut take_pending = {
        let mut eff = world
            .get_resource_mut::<ActionEffects>()
            .expect("ActionEffects initialized by plugin");
        std::mem::take(&mut eff.pending)
    };
    let mut still_pending = Vec::new();
    let mut new_rates: Vec<(String, bool)> = Vec::new();
    // Value-based effect attribution: an action is effective iff the
    // canonical digest changed between its frame start and retirement
    // time (current frame). This aligns with le3.23's value-based
    // liveness and prevents same-value writes (ambient counters,
    // idempotent rewrites) from masking dead verbs.
    let now_digest = format!("{:?}", crate::determinism::canonical_state(world));
    for p in take_pending.drain(..) {
        if frame.saturating_sub(p.frame) < window {
            still_pending.push(p);
            continue;
        }
        let Some((_tick, digest_at_action)) = frame_digests.get(&p.frame) else {
            still_pending.push(p);
            continue;
        };
        let effective = now_digest != *digest_at_action;
        new_rates.push((p.action_key.clone(), effective));
    }
    world
        .get_resource_mut::<ActionEffects>()
        .expect("ActionEffects initialized by plugin")
        .pending = still_pending;

    // RF8: snapshot rates (plus declared keys) before the mutable
    // update loop, for sibling/family liveness and decl decision.
    let eff_snapshot = {
        let eff = world.resource::<ActionEffects>();
        (eff.rates.clone(), eff.declared_keys.clone())
    };
    let (family_live, declared_keys) = {
        let mut live: std::collections::HashSet<String> = eff_snapshot
            .0
            .iter()
            .filter(|(_, r)| r.effective > 0)
            .map(|(k, _)| verb_family(k))
            .collect();
        // Declared keys belong to declared families; treat them as
        // live-by-contract (the game claims to bind them).
        for k in &eff_snapshot.1 {
            live.insert(verb_family(k));
        }
        (live, eff_snapshot.1)
    };

    for (key, effective) in new_rates {
        let (total, _effective_count, report_now) = {
            let mut eff = world
                .get_resource_mut::<ActionEffects>()
                .expect("ActionEffects");
            let rate = eff.rates.entry(key.clone()).or_default();
            rate.total += 1;
            if effective {
                rate.effective += 1;
            }
            (
                rate.total,
                rate.effective,
                !effective && rate.total >= DEAD_VERB_MIN_SAMPLES && rate.effective == 0,
            )
        };
        if report_now {
            // FX1: dead_verb is Major only when the world was otherwise
            // LIVE (something else responded all along). In a frozen
            // world the liveness oracle owns the diagnosis; flagging
            // every verb too just doubles the noise. Scale the live
            // horizon to the liveness timeout (not the tiny effect
            // attribution window): a verb is dead while the rest of the
            // game keeps responding on the liveness timescale.
            let world_live = {
                let state = world.resource::<PlaytestState>();
                let timeout_frames = (liveness_timeout_s * state.tps as f32) as u64;
                frame.saturating_sub(state.last_alive_frame) <= timeout_frames.max(window * 4)
            };
            // RF8: at tier 0 the default surface is a GUESS. An
            // ineffective key only gates when the game/scenario
            // DECLARED it (raw_surface / IntentSurface) or when a
            // sibling of the same verb family demonstrably works
            // (e.g. WASD moves but one arrow is dead). Otherwise an
            // unbound default key is expected — report it as
            // informational unused_input, not dead_verb.
            // Intent-actuator keys come from the game's own
            // IntentSurface declaration — declared by construction.
            // Raw keys are only declared via scenario.bot.raw_surface.
            let declared = declared_keys.contains(&key) || !key.starts_with("raw:");
            let family = verb_family(&key);
            let sibling_live = family_live.contains(&family);
            if world_live && (declared || sibling_live) {
                world.resource_mut::<Violations>().report(
                    crate::rules::Rule::DeadVerb,
                    key.as_str(),
                    format!(
                        "action '{}' produced no gameplay-state change in {} samples — the game may not implement it",
                        key, total
                    ),
                    frame,
                );
            } else if world_live {
                world.resource_mut::<Violations>().report(
                    crate::rules::Rule::UnusedInput,
                    key.as_str(),
                    format!(
                        "input '{}' produced no gameplay-state change in {} samples (not declared by the game — tier-0 default surface)",
                        key, total
                    ),
                    frame,
                );
            }
        }
    }
}

/// Report-facing rates snapshot (folded into the report by the driver).
pub fn effect_rates(world: &World) -> HashMap<String, EffectRate> {
    world
        .get_resource::<ActionEffects>()
        .map(|e| e.rates.clone())
        .unwrap_or_default()
}

/// Vacuity rate for Z1's harness_could_not_drive_game: fraction of ALL
/// audited actions that had zero effect.
pub fn overall_effect_rate(world: &World) -> Option<f64> {
    let rates = effect_rates(world);
    let total: u64 = rates.values().map(|r| r.total).sum();
    let effective: u64 = rates.values().map(|r| r.effective).sum();
    if total == 0 {
        None
    } else {
        Some(effective as f64 / total as f64)
    }
}
