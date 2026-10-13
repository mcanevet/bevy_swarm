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
// Z14: getrandom_backend cfg is set via RUSTFLAGS (a planned
// `cargo swarm run` wrapper will set it automatically).
#![allow(unknown_lints)]
#![allow(unexpected_cfgs)]
// le3.12 (RF13): the README is part of the doctest suite — its
// code blocks compile in CI, so the Usage snippet can never rot.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct _ReadmeDoctests;

#[cfg(feature = "agent")]
pub mod agent;
pub mod autotest;
/// Z18.H: crate-root re-exports so the Z10-generated test can call
/// `bevy_swarm::autotest(GamePlugin, AutotestConfig::from_env())`.
pub use autotest::{autotest, autotest_factory, autotest_with, AutotestConfig, AutotestReport};
pub mod bots;
pub mod branch;
pub mod choice;
pub mod compiled;
pub mod contract;
pub mod conventions;
pub mod determinism;
pub mod diagnostics;
pub mod discover;
pub mod driver;
pub mod effects;
pub mod entropy;
pub mod enums;
pub mod external;
pub mod fingerprint;
pub mod game_types;
pub mod golden;
pub mod harness;
pub mod headless;
pub mod identity;
pub mod junit;
pub mod liveness;
pub mod minimize;
pub mod oracles;
pub mod planner;
pub mod raw_input;
pub mod robustness;
pub mod rules;
pub mod scenario;
pub mod sinks;
pub mod smoke;
pub mod state;
pub mod sweep;
pub mod typed;

#[cfg(test)]
mod verify {
    mod action_log;
    mod app_config;
    mod audit;
    mod branching;
    mod check_ops;
    mod ddmin;
    pub(crate) mod fixtures;
    mod harness;
    mod invariants;
    mod loader;
    mod minimize;
    mod oracle_frames;
    mod persona;
    mod planner;
    mod replay;
    mod scenario;
    mod test_api_ext;
    mod validation;
}
pub mod issues;
