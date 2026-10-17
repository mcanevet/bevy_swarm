//! Data structures from discover.rs — DiscoveredSurface / Effect / config.
//! (only what PlaytestReport needs)

/// A differential effect: when a probe input diverged from baseline.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Effect {
    /// Frames between the probe input and the first digest
    /// divergence (1 = same frame; 0 = group-completion filler).
    pub first_divergence_frame_delta: u32,
    /// Top changed percept keys from the A4 snapshot diff
    /// (game-owned keys only).
    pub changed: Vec<String>,
}

/// The result of discovery.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct DiscoveredSurface {
    pub keys: Vec<(String, Effect)>,
    pub mouse_buttons: Vec<(String, Effect)>,
    pub mouse_motion: Option<Effect>,
    pub wheel: Option<Effect>,
    pub gamepad_buttons: Vec<(String, Effect)>,
    pub gamepad_axes: Vec<(String, Effect)>,
}
