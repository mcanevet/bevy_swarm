# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- **I5: Action-effect oracle (actuator-agnostic) + per-action effect rates** — `src/effects.rs`: every audited action (intents via the audit log, Z3 raw actions) records a `PendingEffect`; after `effect_window_frames` (default tps/2) an effect = any gameplay-state change since the action's tick (I4 `changed_since`). Report gains `action_effect_rate: {key: {effective, total}}`. Rule `dead_verb` (Major): an action key with 0% effect over ≥10 samples. `overall_effect_rate()` feeds the Z1 vacuity guard.

### Added
- **Z6: Gameplay inference from the game's crate path** — `src/game_types.rs`: `GameTypes` (crate-prefix classification, `from_plugin::<P>()`, `.with_crate(...)`), `infer_gameplay_archetypes`, and sticky tagging that inserts the real `Gameplay` + `InferredGameplay` markers on entities whose archetype has ≥1 game-owned component (cameras/lights/windows/UI/observers excluded). Explicit `Gameplay` markers disable inference. Wired via `headless_app` (GameTypes insert) and an incremental `First`-set system. Tests in `tests/game_inference.rs` + lib tests.

### Added
- **I3: Typed Rust API** — oracles, goal predicates and bot policies as plain Bevy systems: `PlaytestAppExt::{add_oracle, add_goal_predicate, add_bot_policy}` registering into a `TypedRegistry`; `run_typed_oracles` in `PlaytestSet::Oracles`; scenario `oracles` selection (None = all, unknown names reject at load); planner `{"kind":"predicate","name":...}` goals; `BotType::Custom` + `bot.policy`. Tests in `tests/typed.rs` incl. minimization of typed-oracle failures. JSON DSL unchanged.

### Added
- **I2: Compiled invariants** — load-time component resolution (typos reject before any frame runs), archetype-level query counts via QueryBuilder + matched_archetypes (O(archetypes) vs O(entities)), pre-parsed ParsedPath for reflect fields, lazy component-id resolution for Startup-spawned components. New `src/compiled.rs` with `CompiledScenario` resource, `Accessor::Count` fast path, and `CompiledKind::QueryCount`. Tests: `unknown_component_rejected_at_load`, `count_is_archetype_level`, `lazy_registered_component_count_works`. Delta windows converted to VecDeque (eliminating O(n) remove(0)).

### Added
- New `raw_input` module (Z3): `RawActionQueue`, `ActiveKeyHolds`, `ActiveMouseHolds`, `VirtualGamepad` resources
- `RawAction` enum variants: `Key`, `MouseButton`, `Click`, `MouseMove`, `Cursor`, `Wheel`, `GamepadButton`, `GamepadAxis`, `ClickEntity`, `Wait`
- Systems `raw_input_preupdate_system` (keyboard/gamepad in PreUpdate) and `raw_input_update_system` (mouse/cursor/wheel in Update), wired into `PlaytestPlugin`
- Optional `gamepad` crate feature (`bevy/gamepad`): lazily spawned virtual gamepad emitting `GamepadConnectionEvent`, `RawGamepadButtonChangedEvent`, `RawGamepadAxisChangedEvent`
- Integration tests in `tests/raw_input.rs`
- Z11 version-to-version differential testing (golden trajectories): record_golden/check_golden with per-frame digests + periodic snapshots, Tolerance for float comparisons, digest-only vs readable diffs via TestApi, BEVY_SWARM_UPDATE_GOLDEN approval workflow. Integration tests in tests/golden.rs
- Z18 autotest() integration: AutotestConfig (16 seeds × 20s chaos defaults, BEVY_SWARM_SEEDS/DURATION/THREADS env overrides), autotest(game_plugin, config) building on headless_app + run_matrix (parallel, fresh App per seed), AutotestReport {schema_version, contract_tier, harness_mode, scope, runs, failures, coverage, summary()} persisted to target/bevy_swarm/runs/autotest-<elapsed>/report.json. Integration tests in tests/autotest.rs
- I1 identity index + deterministic StableId: observer-maintained IdentityIndex (O(1) Name/StableId lookups replace linear scans), StableId assigned on Gameplay insertion; replay Select accepts {"stable_id": n} for unnamed entities (removes B1 "name your Gameplay entities" limitation for replayability); run_scenario resets the index before ResetHooks; diagnostics report unnamed Gameplay entities
- Z2 headless App builder: `headless_platform()` (DefaultPlugins configured with invisible window, DontExit) and `headless_app::<P>(game)` returning a fresh-App-per-call closure; bundled-DefaultPlugins conflicts surface actionable guidance instead of an opaque panic. Integration tests in `tests/headless.rs`

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

