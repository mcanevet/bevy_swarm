//! Headless in-process playtest harness for Bevy.
//!
//! Games consume this crate as a dependency instead of copying the
//! harness into src/. NO game-specific glue here.
//!
//! Requires Bevy 0.19 or later. Buffered events are `Message`s
//! (`app.add_message`, `MessageWriter`/`MessageReader` in
//! `bevy::ecs::message`); `Event` refers to the observer system.
//!
//! ## Module layout (re-exported here for convenience)
//!
//! - [`scenario`] — scenario DTOs, defaults, load-time validation
//! - [`state`] — runtime state resources (PlaytestState, Violations)
//! - [`bots`] — chaos/replay/pursuit/synthetic bots
//! - [`oracles`] — per-frame world checks and invariant evaluation
//! - [`driver`] — PlaytestPlugin, run_scenario, PlaytestReport
//! - [`minimize`] — ddmin crash minimization
//! - [`branch`] — branch-matrix variant testing
//! - [`diagnostics`] — contract diagnostics and calibration

pub use crate::branch::*;
pub use crate::diagnostics::*;
pub use crate::driver::*;
pub use crate::minimize::*;
pub use crate::oracles::*;
pub use crate::scenario::*;
pub use crate::state::*;

// Re-export for consumers that only depend on the harness module.
pub use crate::contract::Gameplay;
