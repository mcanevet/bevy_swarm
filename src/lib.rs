//! Playtest harness for Bevy games implementing the testable-conventions contract.
//!
//! ## Usage in a game crate
//!
//! Add as a dev-dependency in your game's `Cargo.toml`:
//! ```toml
//! [dev-dependencies]
//! bevy_swarm = "0.1"
//! ```
//!
//! Then in your game's `src/test_conventions.rs` (or wherever you define
//! the contract), implement the traits from `bevy_swarm::contract`:
//! - Define your `UserIntent` enum (or re-export the default one)
//! - Implement `TestApiResolve` on your `TestApi` resource
//! - Register `ResetHooks`, `IntentSurface`, and `Gameplay` marker
//!
//! In your test suite, import the harness:
//! ```rust,no_run
//! use bevy_swarm::harness::{run_scenario, validate_scenario};
//! use bevy_swarm::contract::{UserIntent, Gameplay, ResetHooks, IntentSurface};
//! ```
//!
//! The harness expects:
//! - A `UserIntent` message type (already provided by this crate's default)
//! - A `TestApi` resource that implements `TestApiResolve` (provided by your game)
//! - `ResetHooks` and `IntentSurface` resources (provided by this crate's defaults)
//!
// Z14: getrandom_backend cfg is set via RUSTFLAGS by cargo swarm run.
#![allow(unknown_lints)]
#![allow(unexpected_cfgs)]
#[cfg(feature = "agent")]
pub mod agent;
pub mod autotest;
pub mod bots;
pub mod branch;
pub mod choice;
pub mod compiled;
pub mod contract;
pub mod conventions;
pub mod determinism;
pub mod diagnostics;
pub mod driver;
pub mod effects;
pub mod entropy;
pub mod enums;
pub mod fingerprint;
pub mod game_types;
pub mod golden;
pub mod harness;
pub mod headless;
pub mod identity;
pub mod liveness;
pub mod minimize;
pub mod oracles;
pub mod planner;
pub mod raw_input;
pub mod robustness;
pub mod rules;
pub mod scenario;
pub mod sinks;
pub mod state;
pub mod sweep;
pub mod typed;

#[cfg(test)]
mod verify;
