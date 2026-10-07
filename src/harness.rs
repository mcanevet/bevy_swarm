//! Headless in-process playtest harness for Bevy.
//!
//! Games consume this crate as a dependency instead of copying the
//! harness into src/. NO game-specific glue here.
//!
//! Targets Bevy 0.20 (exactly one minor version per bevy_swarm
//! release). Buffered events are `Message`s
//! (`app.add_message`, `MessageWriter`/`MessageReader` in
//! `bevy::ecs::message`); `Event` refers to the observer system.
//!
//! ## Module layout (re-exported here for convenience)
//!
//! - [`crate::scenario`] — scenario DTOs, defaults, load-time validation
//! - [`crate::state`] — runtime state resources (PlaytestState, Violations)
//! - [`crate::bots`] — chaos/replay/pursuit/synthetic bots
//! - [`crate::oracles`] — per-frame world checks and invariant evaluation
//! - [`crate::driver`] — PlaytestPlugin, run_scenario, PlaytestReport
//! - [`crate::minimize`] — ddmin crash minimization
//! - [`crate::branch`] — branch-matrix variant testing
//! - [`crate::diagnostics`] — contract diagnostics and calibration

pub use crate::branch::*;
pub use crate::diagnostics::*;
pub use crate::driver::*;
pub use crate::minimize::*;
pub use crate::oracles::*;
pub use crate::scenario::*;
pub use crate::state::*;

// Re-export for consumers that only depend on the harness module.
pub use crate::contract::Gameplay;
pub use crate::determinism::*;
