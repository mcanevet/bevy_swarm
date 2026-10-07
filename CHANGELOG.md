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
- Bounds/frozen-world/pursuit oracles read GLOBAL Transform (children of
  moved parents were checked in parent-relative space: false passes and
  wrong steering)
- Finite check covers rotation and scale (NaN quats, NaN/zero scale),
  plus a separate `nodes_rotation_unnormalized` rule for denormalized
  quaternions
- run_scenario/calibrate_world finish plugin building (Plugin::finish
  hooks now run — App::update alone never called finish/cleanup)
- ViolationEntry gains  (detail at most recent occurrence);
   stays anchored to the FIRST occurrence (reproduction anchor)
- System coverage excludes harness-owned systems (bevy_swarm::) to avoid
  inflated fraction() and spurious unexecuted gaps; new
  SystemCoverage::game_only() drops bevy_ engine systems too
- Aggressive persona no longer hangs on Wait-only surfaces (draws from
  filtered candidate set; empty → yields Wait)
- Planner pursuit sequences pace by input_rate_hz and start at emit[0]
  when a primitive activates (previously indexed by absolute frame)
- Coverage keys are variant-level (move/choice/axis/select/wait) for
  all bots; detailed values kept in violation context only
- Deleted dead PersonaConfig/persona_of
- Readiness gate now runs INSIDE the panic boundary: a game that panics
  while loading yields status crash (previously could hang or pass).
  Bots and oracles no longer run before `GameReady(true)`; when the gate
  opens, frame counters reset so `after_s`, replay frames and `duration_s`
  count from readiness. New `PlaytestReport.pre_ready_frames`
- `run_scenario` on the same App twice no longer leaks violations/action
  log across runs (fresh Violations/ActionLog per run)
- Bot dispatch via run conditions instead of per-system early returns
- `calibrate_world_opts` with `CalibrationOptions {duration_s, tps, seed,
  simulated_time}`: inserts PlaytestState if missing, applies simulated
  time, captures panics, samples generic numeric percepts (not just
  TestApi.score), errors clearly without PlaytestPlugin
- Calibration off-by-one: archetype minimum invariants are now tight
  (`ge min` instead of `above min-1`)

### Added
- `sweep_seeds` + `SweepConfig`/`SweepReport` (E3): seed sweeps deduped by
  fingerprint (lowest seed wins), early-stop at max_failures
- Persisted regressions: `RegressionRecord` (versioned, one file per
  fingerprint: regression_<fp>.json), `write_regression`,
  `load_regressions` (record + legacy bare-Scenario forms),
  `run_regressions_and_sweep`
- Fingerprinting (T1): stable failure identity across seeds/frames/entities;
  `Normalizer` strips entity IDs, frame numbers, floats, hex addresses,
  seeds, paths; `Fingerprint` computed at report snapshot time (scheme 1)
- `ViolationEntry.fingerprint` + `fingerprint_scheme` fields;
  informational rules exempt from fingerprint gating
- `ScenarioRunner` trait + `InProcess` runner + `run_matrix` (parallel:
  one fresh scoped thread per scenario, max_parallel cap, input-order
  output); `run_branch_matrix` is now `run_matrix(.., 1)` semantics
- `Component:<Type>{<Name>}.<field>` percept addressing — stable across
  despawns; `[N]` indices now sort candidates by Entity (documented as
  unstable; prefer {Name}); stable-id addressing reserved for I1
- `resolve_path(world, path)` — one path-resolution entry point: the
  registered type-erased resolver (if any) first, then world percepts
  (Resource:/Component: grammar)
- `TestApiResolver` resource + `RegisterTestApi::register_test_api::<A>()`
  — games register a custom TestApi type; last call wins
- TestApi is now OPTIONAL (zero-contract): only scenarios referencing
  TestApi.* paths (or the planner bot) fail at load time with guidance
  when no resolver is registered
- `requires_path` (expert REQUIRE) resolves through percepts too
- planner bot resolves goals through resolve_path (custom TestApi types
  work for pursuit/goal conditions)
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
