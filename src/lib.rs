//! Playtest harness for Bevy games implementing the testable-conventions contract.
//!
//! ## Usage in a game crate
//!
//! Add as a dev-dependency in your game's `Cargo.toml`:
//! ```toml
//! [dev-dependencies]
//! bevy_playtest = "0.1"
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
// Z14: getrandom_backend cfg is set via RUSTFLAGS (a planned
// `cargo bevy-playtest run` wrapper will set it automatically).
#![allow(unknown_lints)]
#![allow(unexpected_cfgs)]
// RF41: docs.rs builds with --cfg docsrs; expose feature names on
// feature-gated items. (doc_auto_cfg was removed in Rust 1.92,
// merged into doc_cfg.)
#![cfg_attr(docsrs, feature(doc_cfg))]
// le3.12 (RF13): the README is part of the doctest suite — its
// code blocks compile in CI, so the Usage snippet can never rot.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct _ReadmeDoctests;

// le3.25 (RF29): the docs/reference-*.md snippets are part of the
// doctest suite — `rust,no_run` blocks are still COMPILED (only
// execution is skipped), so reference examples can never rot either.
#[cfg(doctest)]
#[doc = include_str!("../docs/reference-usage.md")]
struct _ReferenceUsageDoctests;

#[cfg(doctest)]
#[doc = include_str!("../docs/reference-golden.md")]
struct _ReferenceGoldenDoctests;

#[cfg(feature = "agent")]
pub mod agent;
pub mod autotest;
/// The curated prelude: everything a playtest needs in one glob import.
///
/// ```rust,no_run
/// use bevy_playtest::prelude::*;
/// ```
///
/// This is the SUPPORTED surface of bevy_playtest 0.1. Everything outside
/// `prelude` (and the specific modules named in the docs) is internal
/// machinery with no stability guarantee until the core/adapter split
/// lands.
pub mod prelude {
    #[cfg(feature = "agent")]
    pub use crate::agent::{AgentConfig, AgentPlugin};
    pub use crate::builder::ScenarioBuilder;
    pub use crate::contract::{IntentSurface, SurfaceVariant, TestConventionsPlugin, UserIntent};
    pub use crate::driver::{run_scenario, PlaytestPlugin, PlaytestReport};
    pub use crate::enums::PlaytestStatus;
    pub use crate::rules::Rule;
    pub use crate::scenario::{BotConfig, Scenario};
    pub use crate::{autotest, autotest_factory, autotest_with, AutotestConfig, AutotestReport};
}

#[cfg(doctest)]
/// le3.30: the prelude alone must suffice for the documented quick-start
/// pattern (tiny game + scenario + run).
#[doc = include_str!("../docs/prelude-smoke.md")]
struct _PreludeSmokeDoctests;

/// Z18.H: crate-root re-exports so the Z10-generated test can call
/// `bevy_playtest::autotest(GamePlugin, AutotestConfig::from_env())`.
pub use autotest::{autotest, autotest_factory, autotest_with, AutotestConfig, AutotestReport};
pub mod ambiguity;
#[doc(hidden)]
/// Internal machinery — no stability guarantee before the core/adapter split.
pub mod bots;
#[doc(hidden)]
/// Internal machinery — no stability guarantee before the core/adapter split.
pub mod branch;
pub mod builder;
#[doc(hidden)]
/// Internal machinery — no stability guarantee before the core/adapter split.
pub mod choice;
#[doc(hidden)]
/// Internal machinery — no stability guarantee before the core/adapter split.
pub mod compiled;
pub mod contract;
pub mod conventions;
#[doc(hidden)]
/// Internal machinery — no stability guarantee before the core/adapter split.
pub mod determinism;
#[doc(hidden)]
/// Internal machinery — no stability guarantee before the core/adapter split.
pub mod diagnostics;
#[doc(hidden)]
/// Internal machinery — no stability guarantee before the core/adapter split.
pub mod discover;
pub mod driver;
#[doc(hidden)]
/// Internal machinery — no stability guarantee before the core/adapter split.
pub mod effects;
#[doc(hidden)]
/// Internal machinery — no stability guarantee before the core/adapter split.
pub mod entropy;
pub mod enums;
#[doc(hidden)]
/// Internal machinery — no stability guarantee before the core/adapter split.
pub mod external;
#[doc(hidden)]
/// Internal machinery — no stability guarantee before the core/adapter split.
pub mod fingerprint;
#[doc(hidden)]
/// Internal machinery — no stability guarantee before the core/adapter split.
pub mod game_types;
pub mod golden;
#[doc(hidden)]
/// Internal machinery — no stability guarantee before the core/adapter split.
pub mod harness;
#[doc(hidden)]
/// Internal machinery — no stability guarantee before the core/adapter split.
pub mod headless;
#[doc(hidden)]
/// Internal machinery — no stability guarantee before the core/adapter split.
pub mod headless_strip;
#[doc(hidden)]
/// Internal machinery — no stability guarantee before the core/adapter split.
pub mod identity;
pub mod junit;
#[doc(hidden)]
/// Internal machinery — no stability guarantee before the core/adapter split.
pub mod liveness;
#[doc(hidden)]
/// Internal machinery — no stability guarantee before the core/adapter split.
pub mod minimize;
#[doc(hidden)]
/// Internal machinery — no stability guarantee before the core/adapter split.
pub mod oracles;
#[doc(hidden)]
/// Internal machinery — no stability guarantee before the core/adapter split.
pub mod pass_through;
#[doc(hidden)]
/// Internal machinery — no stability guarantee before the core/adapter split.
pub mod planner;
#[doc(hidden)]
/// Internal machinery — no stability guarantee before the core/adapter split.
pub mod raw_input;
#[doc(hidden)]
/// Internal machinery — no stability guarantee before the core/adapter split.
pub mod render_check;
#[doc(hidden)]
/// Internal machinery — no stability guarantee before the core/adapter split.
pub mod robustness;
pub mod rules;
pub mod scenario;
#[doc(hidden)]
/// Internal machinery — no stability guarantee before the core/adapter split.
pub mod sinks;
#[doc(hidden)]
/// Internal machinery — no stability guarantee before the core/adapter split.
pub mod smoke;
#[doc(hidden)]
/// Internal machinery — no stability guarantee before the core/adapter split.
pub mod state;
#[doc(hidden)]
/// Internal machinery — no stability guarantee before the core/adapter split.
pub mod sweep;
#[doc(hidden)]
/// Internal machinery — no stability guarantee before the core/adapter split.
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
