//! Data half of effects.rs — action-effect statistics and the
//! effect-window helpers. The World-probing oracle system stays behind.

use std::collections::HashMap;

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
/// (Engine-binding dropped: was a `#[derive(Resource)]`.)
#[derive(Default)]
pub struct ActionEffects {
    pub pending: Vec<PendingEffect>,
    pub rates: HashMap<String, EffectRate>,
    /// Change tick at the START of each frame (maintained by the
    /// oracle; actuators only know their frame).
    pub tick_by_frame: HashMap<u64, u32>,
    /// Keys explicitly declared by the game/scenario.
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
