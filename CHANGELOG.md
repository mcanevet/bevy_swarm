# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- **le3.30 (RF40): public API curation** — `bevy_swarm::prelude` is the supported surface of 0.1 (PlaytestPlugin, run_scenario, autotest family + AutotestConfig/AutotestReport, Scenario/ScenarioBuilder, IntentSurface/SurfaceVariant/TestConventionsPlugin/UserIntent, PlaytestReport, PlaytestStatus, Rule, AgentPlugin+AgentConfig under the `agent` feature). All internal modules (branch, choice, determinism, entropy, golden, headless, harness, identity, liveness, minimize, oracles, robustness, sinks, state, sweep, typed, ...) are `#[doc(hidden)]`: reachable for harness fixtures/tests and power users, but excluded from the rendered docs and carrying no stability guarantee until the core/adapter split. A prelude-only doctest (docs/prelude-smoke.md) locks the quick-start pattern to the supported surface; examples/minimal.rs now uses the prelude.
- **dde**: pool-thread panics are now captured. Background work spawned on bevy_tasks pools (IoTaskPool, AsyncComputeTaskPool — common in real games) previously escaped all panic capture because routing was thread-local on the driver thread. Now a process-global run registry (`GLOBAL_RUNS`) attributes pool-thread events to the sole active run (thread-name-gated; multiple concurrent runs refuse ambiguous attribution instead of poisoning an innocent run — a real observed flake mode), panics on pool threads report target `pool_thread`, and a bounded 50ms grace at run end lets detached straggler work land before sinks drain. Known limitation (documented): stragglers from seed N panicking during seed N+1 attribute to the active run; task pools are process-global. New fixture `multithread_panic` (pure Bevy, plugin + binary) with regression coverage.
- **x4k**: liveness oracle false-negative fix at high TPS (tps > 300). The history-ring cap was computed with a hardcoded 60 fps, so the lookback window exceeded the ring and `nth()` was always None → `frozen_world`/`undrivable_game` could never fire. Fixed: cap now scales with `scenario.tps`. Regression tests added at tps=480 and tps=1000.
- **m2h.19 (Z18): autotest() integration** — `AutotestReport` rewritten to conventions §4 shape: integer `schema_version`, `contract_tier` derived from the per-run Z1 `CheckedScope` union (never hardcoded), `harness_mode: "in_process"`, `scope` union of per-run checked scopes, `budget` summary (renamed from `scope.seeds_run`), `regressions` (E3: persisted regressions replayed FIRST, each still_failing/now_passing), `gate` (T13: new/regressed/known/muted classification + verdict); per-run `RunSummary.violations` (rule/detail/fingerprint) for `detail_contains` expectations; `AutotestConfig.seeds` (explicit seed list) + `gate`; crate-root re-exports (`bevy_swarm::autotest/autotest_with/AutotestConfig/AutotestReport`); per-seed `seed-<n>.json` PlaytestReports written next to `report.json`; T13 issue-DB core (`src/issues.rs`: `IssueDb`, `classify`, `Gate::{NewOrRegressed,AnyOpen,None}`, `BEVY_SWARM_UPDATE=issues` writes); JUnit maps known/muted fingerprints to `<skipped>` with new/regressed staying failures; `docs/report-schema.json` gains `$defs.autotest_report` (+ validated in acceptance); scaffold:"none" expectation copies for all fixture `clean.json` + walker `dead_left_key`/`panic_on_edge`/`panic_on_load` (fixture-runner `--scaffold none|adapter` flag; scaffold:none path runs `bevy_swarm::autotest` over the plain game plugin, writing AutotestReport + per-seed reports to `--out`'s directory); Z4 discovery survives startup panics (probe update wrapped in `catch_unwind`, empty surface on panic so seeds still report the crash); headless factory adds `PlaytestPlugin` before the game plugin so startup-system panics are captured.
- **m2h.2 (Z1): tiered contract** — `ContractTier` (zero/observable/semantic) + `CheckedScope` (tier, active/inactive oracle families with reasons, actuators) on every `PlaytestReport` (schema-required, `report.scope`); registry-based tier detection; actuator selection (`bot.actuator`: auto/intents/raw/ui, validated at load, auto = intents when `IntentSurface` is non-empty, else raw); vacuity guard `harness_could_not_drive_game` (>= 20 actions with 0% effect, suppressed when no gameplay state is observable); `harness_could_not_drive_game` rule registered + documented; standalone tier-0 fixture (`fixture_lone`, no bevy_swarm dep, no adapter) with fixture × tier artifact tests; `expect_tier` acceptance expectation field; `contract_diagnostics` now checks `TestApiResolver` (resolver-based games detect as Semantic); README "Contract tiers" table.
- **m2h.5 (Z4): input-surface discovery** — `discover_surface()` differential probing (baseline + per-candidate probe runs, digest diff with harness-owned key filtering), static device pruning via `Schedule::systems_with_access`, group completion (arrow/WASD/d-pad/face groups), `RawSurface::from_discovered` with effect-breadth weights feeding chaos sampling, autotest integration (runs discovery when chaos has no declared surface, falls back to the FX4 default when nothing is observable), `discovered_surface` on `PlaytestReport`/`AutotestReport` + schema, `AutotestReport::summary()` surface line.
- **Z4 dependency fix**: A4 `canonical_state` now digests unnamed `Gameplay` entities via `StableId` fallback keys (tier-0 games without `Name` were previously invisible to digests, blinding differential probing).
- **I1 identity index + deterministic StableId**: observer-maintained IdentityIndex (O(1) Name/StableId lookups replace linear scans), StableId assigned on Gameplay insertion; replay Select accepts `{"stable_id": n}` for unnamed entities (removes B1 "name your Gameplay entities" limitation for replayability); `run_scenario` resets the index before ResetHooks; diagnostics report unnamed Gameplay entities.
- **Z2 headless App builder**: `headless_platform()` (DefaultPlugins configured with invisible window, DontExit) and `headless_app::<P>(game)` returning a fresh-App-per-call closure; bundled-DefaultPlugins conflicts surface actionable guidance instead of an opaque panic. Integration tests in `tests/headless.rs`.
- **Z11 golden differential testing**: `record_golden`/`check_golden` with per-frame digests + periodic snapshots, `Tolerance` for float comparisons, digest-only vs readable diffs via TestApi, `BEVY_SWARM_UPDATE=golden|all` approval workflow. Integration tests in `tests/golden.rs`.

### Fixed
- **le3.11 (RF7): error/log capture** — LogPlugin custom_layer integration (headless_app installs swarm capture layer via `LogPlugin::custom_layer`), stable sink fingerprints (digit-run normalization + count-suffix stripping), `bevy_suppressed` informational rule for Ignore-severity errors, drop-guarded handler install (handler moved after load-time validation). PR #124.
- **le3.10 (RF6): informational rules** — informational rules never gate run status, fps_floor uses wall-clock time, `Rule::is_informational()` derived from enum with parity test. PR #123.
- **le3.9 (RF5): expert-rule semantics** — text-WHEN rules fixed, J1 overall<0 iff fail corrected, text REQUIRE robustness ±1, deadline-capped observe_max. PR #122.
- **le3.8 (RF4): liveness** — softlock detection fixed (sticky-streak-only, timeout-scaled ring buffers; `IdlePhase` resource-based clean system demo). PR #121.
- **le3.7 (RF3): gate integrity** — per-case guard on many seeds, no fake/vacuous bugs, strict must_report matching, no stale binary (always rebuild fixture-runner). PR #120.
- **le3.6 (RF2): JUnit XML** — malformed XML fixed (illegal control chars, vacuous CDATA). PR #119.
- **le3.5 (RF1): auth guard** — token-guard every playtest method, no crash on missing resources, real screenshot sandbox. PR #118.
- **Qa8.16**: ambiguity detection hardened (merge-key redaction, hand-built minimal Apps structured rejection).
- **o0t**: Component-path validation changed (typos now reject at load for Component: paths).
- **ml5**: hardening (deterministic entropy, single-threaded executor fixes).
- **Sink fingerprint instability** — same error repeated 10 vs 1000 frames yielded different fingerprints; fixed by collapsing digit runs to single # and stripping (xN) suffix before fingerprinting.
- **Ignore-severity errors dropped** — Suppressed errors now surface as `bevy_suppressed` informational rule.
- **Half-installed handler after rejected run** — handler install moved after all load-time validation; rejected runs no longer leave a dangling handler.
- **Golden path traversal** — scenario names with `..` or `/` previously escaped the golden dir; now sanitized.
- **README doctest rot** — Usage snippet now compiles; env vars corrected; experimental features marked.
- **docs/conventions.md drift** — member list corrected, version pinned to 0.20.0, run ID format clarified, golden path fixed, env var table added.

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
- `PlaytestPlugin` now registers raw-input messages (`KeyboardInput`, `MouseButtonInput`, `MouseMotion`, `MouseWheel`, `WindowEvent`, gamepad events) so the raw input actuator works headless

### Removed
- **Obsolete docs** — `src/verify.rs` references removed from README (module tests inside `src/` are now documented).
