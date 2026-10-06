# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
