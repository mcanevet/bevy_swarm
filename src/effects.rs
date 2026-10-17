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

/// swarm-6i8: hint that the world's recent change activity looks
/// purely ambient (all recently-changed digest keys were already
/// changing in an action-free baseline window). Used to quiet
/// unused_input noise in attract-mode worlds where no input is bound.
pub fn ambient_only_hint(world: &World) -> bool {
    let Some(state) = world.get_resource::<PlaytestState>() else {
        return false;
    };
    ambient_only_from_snaps(&state.effect_window_snapshots)
}

/// Pure snapshot-based ambient check — callable after PlaytestState
/// has left the World (the vacuity guard runs at report-finalize
/// time, past the remove_resource).
pub fn ambient_only_from_snaps(
    snaps: &[(u64, u32, std::collections::BTreeMap<String, String>)],
) -> bool {
    if snaps.len() < 3 {
        return false;
    }
    // Baseline pair: the two OLDEST snapshots AFTER the digest map
    // reaches its steady state (early snapshots can predate Startup
    // spawns — an empty r0 vs populated r1 would look like wholesale
    // change). Steady state = first index whose map has the same
    // length as the LAST snapshot's map.
    let steady_len = snaps.last().map(|(_, _, m)| m.len()).unwrap_or(0);
    let steady_start = snaps
        .iter()
        .position(|(_, _, m)| m.len() == steady_len)
        .unwrap_or(0);
    if steady_start + 1 >= snaps.len() {
        return false;
    }
    let r0 = &snaps[steady_start].2;
    let r1 = &snaps[steady_start + 1].2;
    let ambient: std::collections::HashSet<String> = r0
        .iter()
        .filter(|(k, v)| r1.get(*k) != Some(v))
        .map(|(k, _)| k.clone())
        .collect();
    if ambient.is_empty() {
        // A world with digest rows but ZERO change in the baseline gap
        // is a STATIC world — vacuity/dead-verb oracles own it. But a
        // world with NO digest rows in its steady state is purely
        // cosmetic (no Gameplay entities, no reflected resources): the
        // effect attribution is structurally blind, not vacuous —
        // treat as ambient-only to quiet unused_input/vacuity noise.
        return steady_len == 0;
    }
    // Every LATER change must be confined to ambient keys. New rows
    // appearing (spawns) or disappearing (despawns) make the world
    // non-ambient-quiet.
    let bad: std::collections::BTreeSet<String> = snaps[steady_start + 1..]
        .windows(2)
        .flat_map(|w| {
            w[0].2
                .iter()
                .filter(|(k, v)| w[1].2.get(*k) != Some(*v) && !ambient.contains(*k))
                .map(|(k, _)| k.clone())
                .collect::<Vec<_>>()
        })
        .collect();
    bad.is_empty()
}

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
    // FX1: snapshot rows for value-based effect attribution.
    // swarm-6i8: keep per-key rows (not flattened digests) so ambient
    // motion can be subtracted.
    let frame_rows: HashMap<u64, std::collections::BTreeMap<String, String>> = {
        let state = world.resource::<PlaytestState>();
        state
            .effect_window_snapshots
            .iter()
            .map(|&(f, _, ref d)| (f, d.clone()))
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
    let now_rows: std::collections::BTreeMap<String, String> = {
        let rows = crate::determinism::canonical_state(world);
        let mut m = std::collections::BTreeMap::new();
        for (k, v) in rows {
            if !k.starts_with("census.") {
                m.insert(k, v);
            }
        }
        m
    };
    // swarm-6i8: ambient-only hint, computed ONCE per oracle pass
    // from the snapshot rows.
    let world_ambient_only = {
        let state = world.resource::<PlaytestState>();
        ambient_only_from_snaps(&state.effect_window_snapshots)
    };
    // swarm-6i8 (RF43 remainder): AMBIENT MOTION SUBTRACTION. Some
    // fixtures legitimately animate without input (attract modes,
    // elapsed-time-scaled values like spawner's IdlePhase sine and
    // regression_pair's Damage). Whole-digest comparison counted that
    // ambient churn as the effect of whatever intent preceded it,
    // masking dead verbs. Fix: for each pending action, identify the
    // keys that were ALREADY changing during the action-free baseline
    // window (the `window` frames immediately before the action —
    // guaranteed action-free by FX1's window <= interval/2 cap) and
    // exclude them: an action is effective iff it produced change
    // BEYOND the ambient dynamics. Intent-causality, not raw change.
    fn ambient_keys(
        frames: &HashMap<u64, std::collections::BTreeMap<String, String>>,
        action_frame: u64,
        window: u64,
    ) -> std::collections::HashSet<String> {
        let mut ambient = std::collections::HashSet::new();
        if window < 2 {
            return ambient;
        }
        let start = action_frame.saturating_sub(window);
        let baseline: Vec<&std::collections::BTreeMap<String, String>> = (start..action_frame)
            .filter_map(|f| frames.get(&f))
            .collect();
        if baseline.len() < 2 {
            return ambient;
        }
        for w in baseline.windows(2) {
            for (k, v) in w[0].iter() {
                if w[1].get(k) != Some(v) {
                    ambient.insert(k.clone());
                }
            }
        }
        ambient
    }
    for p in take_pending.drain(..) {
        if frame.saturating_sub(p.frame) < window {
            still_pending.push(p);
            continue;
        }
        let Some(rows_at_action) = frame_rows.get(&p.frame) else {
            still_pending.push(p);
            continue;
        };
        // Effective = some NON-ambient key changed (or a brand-new key
        // appeared) between the action frame and now.
        let ambient = ambient_keys(&frame_rows, p.frame, window);
        let effective = now_rows
            .iter()
            .any(|(k, v)| rows_at_action.get(k) != Some(v) && !ambient.contains(k))
            || rows_at_action.keys().any(|k| {
                // Disappeared rows also count (despawns) unless ambient.
                !now_rows.contains_key(k) && !ambient.contains(k)
            });
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
                // The world must have been alive at least ONCE (a
                // never-alive world belongs to the liveness oracle's
                // undrivable_game diagnosis) and recently on the
                // liveness timescale.
                let live = state.last_alive_frame > 0
                    && frame.saturating_sub(state.last_alive_frame) <= timeout_frames;
                // swarm-6i8: liveness alone is not enough when the ONLY
                // thing keeping the world live is AMBIENT motion (an
                // attract-mode heartbeat watched by liveness config —
                // spawner's IdlePhase). In such a world the entire
                // default input surface is unbound by design; reporting
                // every key as unused_input is noise, not signal. If ANY
                // audited action anywhere in the run demonstrated a
                // non-ambient effect, the world genuinely responds to
                // input and dead/unused verbs deserve reporting.
                let any_real_effect = eff_snapshot.0.values().any(|r| r.effective > 0);
                live && (any_real_effect
                    || (!declared_keys.is_empty() && declared_keys.contains(&key))
                    || !world_ambient_only)
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
