//! Playtest harness for Bevy games implementing the testable-conventions contract.
//!
//! ## Usage in a game crate
//!
//! Add as a path dependency in your game's `Cargo.toml`:
//! ```toml
//! [dev-dependencies]
//! bevy_playtest = { path = "../.agents/skills/playtest/crates/bevy_playtest" }
//! ```
//!
//! Then in your game's `src/test_conventions.rs` (or wherever you define
//! the contract), implement the traits from `bevy_playtest::contract`:
//! - Define your `UserIntent` enum (or re-export the default one)
//! - Implement `TestApiResolve` on your `TestApi` resource
//! - Register `ResetHooks`, `IntentSurface`, and `Gameplay` marker
//!
//! In your test suite, import the harness:
//! ```rust,no_run
//! use bevy_playtest::harness::{run_scenario, validate_scenario};
//! use bevy_playtest::contract::{UserIntent, Gameplay, ResetHooks, IntentSurface};
//! ```
//!
//! The harness expects:
//! - A `UserIntent` message type (already provided by this crate's default)
//! - A `TestApi` resource that implements `TestApiResolve` (provided by your game)
//! - `ResetHooks` and `IntentSurface` resources (provided by this crate's defaults)
//!
//! ## Sync direction
//!
//! This crate is the canonical source. Fixes to the harness happen here;
//! games add it as a path dependency so they always get the latest version
//! without copy-paste drift.

#[cfg(feature = "agent")]
pub mod agent;
pub mod contract;
pub mod harness;
pub mod planner;

#[cfg(test)]
mod verify;
