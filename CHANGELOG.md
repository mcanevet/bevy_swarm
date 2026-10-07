# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed
- One comparison semantics everywhere: `CheckOp` is now `{lt, le, gt, ge, equals, ne}`
  (legacy wire spellings `below`/`above`/`eq` still parse as `le`/`ge`/`equals`).
  **below/above are inclusive at the boundary** everywhere, including the planner
  and expert-rule WHEN guards (previously strict there — use `lt`/`gt` for strict
  comparisons). `equals` now uses a relative tolerance instead of `f64::EPSILON`,
  so large integer counters compare correctly. The planner's goal `check` is a
  typed enum: typos fail at scenario parse time instead of silently making
  goals unreachable
- Declared MSRV 1.97 (Bevy 0.20.0-rc.2 dependency tree requirement), enforced in CI
- `minimize_crash` replaced by `minimize_failure`: shrinks any failure
  signature (crash OR invariant violation, matched by rule/target to
  prevent slippage to a different bug), trims the regression duration to
  the failure frame + 1s, and returns `Err(NotReproducible)` instead of a
  bogus "minimal" log when the full log does not reproduce the failure
- Removed dead `MinimizeOnCrash` resource and
  `PlaytestReport.minimized_actions` (never populated; `run_scenario`
  cannot rebuild the App — use `minimize_failure` with an app builder)

### Fixed
- Calibration off-by-one: archetype minimum invariants are now tight
  (`ge min` instead of `above min-1`)

### Added
- Initial (unreleased) version. Contract-first architecture: games
  implement `TestApi`, `UserIntent`, `ResetHooks`, `IntentSurface`,
  and `Gameplay` marker; the harness drives `App::update()` headlessly
- Bots: chaos (persona-weighted intent sampling), replay, pursuit,
  synthetic pointer, synthetic keyboard, planner
- Oracles: nodes-in-bounds, finite transforms, frame-time p99/floor/anomaly,
  frozen-world liveness, custom invariants over TestApi paths and
  query-targets, differential (no_decrease/no_increase), expert rules
  (`when` + `requires_*`), eventual-state assertions
- Crash minimization via Zeller ddmin over recorded action logs, with
  regression-scenario JSON emission
- Branch matrix testing across seeds/variants
- Contract diagnostics and calibration-based auto-invariant generation
- JSON scenario format with strict schema (`deny_unknown_fields`)
- Type-safe DSL enums (`BotType`, `CheckOp`, `InvariantRule`, `Persona`, …)
  with wire-format-compatible serialization

### Changed
- **Targets Bevy 0.20** (was 0.19): `bevy` pinned to the
  `v0.20.0-rc.2` on crates.io; when 0.20.0 final releases, switch to `bevy = "0.20"`
  it becomes `bevy = "0.20"`. Policy: each bevy_swarm release supports
  exactly one Bevy minor version.
