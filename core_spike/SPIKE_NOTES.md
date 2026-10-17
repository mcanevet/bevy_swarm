# Spike: bevy-free `bevy_playtest_core` — empirical results

Validated on branch `spike/agnostic-core`. The crate compiles bevy-free
(deps: serde, serde_json, schemars + quick-xml dev) and passes all 56
carried-over unit tests + 1 doctest.

## Per-file outcome table

| File (orig LOC) | Outcome | Edits | LOC moved |
|---|---|---|---|
| external.rs (508) | LIGHT-EDIT | 3 import-path rewrites (`crate::{contract,driver,state}` → `crate::core_state`) | 508 |
| fingerprint.rs (432) | CLEAN (byte-identical copy) | none | 432 |
| junit.rs (320) | LIGHT-EDIT | 1 import rewrite; +1 dev-dep (quick-xml) for its unit tests | 320 |
| rules.rs (399) | LIGHT-EDIT | test-only: `crate::state::` → `crate::core_state::` (3 lines) + docs/rules.md copied for its doctest | 399 |
| smoke.rs (259) | LIGHT-EDIT | 4 path rewrites (`driver/state/contract` → `core_state`) | 259 |
| issues.rs (262) | CLEAN | none | 262 |
| enums.rs (239) | CLEAN (byte-identical) | none | 239 |
| conventions.rs (343) | CLEAN (byte-identical) | none | 343 |
| entropy.rs (291) | CLEAN (byte-identical) | none | 291 |
| builder.rs (576) | LIGHT-EDIT | 1 doctest-import rename; +9-line forwarding shim for `validate_scenario` (removed planner fn) | ~570 |
| choice.rs (333) | LIGHT-EDIT | removed `use bevy::prelude::*` + `#[derive(Resource)]` on ChoiceStream (2 lines) | 333 |
| robustness.rs (242) | LIGHT-EDIT | removed `use bevy::prelude::*` + `#[derive(Resource)]` on RobustnessTracker (2 lines) | 242 |
| scenario.rs (1085) | LIGHT-EDIT | parse_key_code/KEY_LETTERS/KEY_DIGITS swapped bevy KeyCode → local `key::Key` enum (~50 lines); `crate::contract::` → `core_state::` (5 uses) | ~1035 |
| state.rs (664) | SPLIT (light) | data half → `core_state.rs`: ViolationEntry, Violations (full incl. snapshot fingerprinting), Metrics, Coverage, SystemCoverage + new hand-written `PlaytestReport` (driver data half). Dropped: `#[derive(Resource)]`, PlaytestState engine bookkeeping, all World/Schedules probing fns. INFORMATIONAL_RULES kept as a Rule const (parity test passes). | ~440 |
| driver.rs (2069) | SPLIT (light) | PlaytestReport + crashed_empty/scenario_error only (~110 LOC, rewritten into core_state.rs); the entire run loop stays behind | ~110 |
| effects.rs (285) | SPLIT (light) | data half only: DEAR_VERB_MIN_SAMPLES, verb_family, PendingEffect, EffectRate, ActionEffects (dropped `#[derive(Resource)]`). The World oracle system stays behind | ~80 |
| contract.rs (473) | SPLIT (light) | DSL half only: ContractTier, CheckedScope(+union), ActionEntry/ActionSource, TestFieldValue, TestApiResolve trait (~230 LOC rewritten). Behind: UserIntent (bevy Message/Vec2/Entity), all Resource derives, TestConventionsPlugin, World-hook containers | ~230 |
| diagnostics.rs (760) | SPLIT (heavy) | report-formatting half only: ContractReport (trimmed), CalibrationSnapshot/Options, generate_invariants_from_calibration (~150 LOC). Behind: all tier detection, calibrate_world (App-driving), AppTypeRegistry reflection walks | ~150 |
| branch.rs (343) | SPLIT (light) | BranchOutcome/BranchMatrixReport data half (~65 LOC). Behind: the matrix RUNNER (builds bevy App per variant) | ~65 |
| discover.rs (598) | SPLIT (light) | Effect + DiscoveredSurface structs only (~35 LOC). Behind: all differential probing over bevy App | ~35 |
| planner.rs (677) | PARTIAL-PROBE | GoalNode, goal_tree_needs_resolver, PlannerStack (FULL stack machine incl. pop_success/check_postcondition), TestFieldValue/Resolve moved (~290 LOC). The 2 World fns stubbed with `todo!()` as planned | ~290 |

**Total bevy-free LOC: ~5,713** of 23,068 (~25%).

Clean (byte-identical): fingerprint, issues, enums, conventions, entropy
(~1,567 LOC). Light edits: 13 files (~4,000 LOC). Abandoned entirely:
driver run-loop, oracles, bots, agent, raw_input, sweep, minimize,
golden, compiled, headless*, identity, liveness, sinks, typed,
game_types, harness, autotest, render_check — every one is
World/System/App-coupled by design (systems, resources, exclusive
world access, BRP).

## Blockers catalogued (for anything left behind)

1. `bevy::ecs::world::World` / `&mut World` exclusive systems (driver,
   planner_bot_system, effects oracle, oracles, identity).
2. `#[derive(Resource)]` / `#[derive(Component)]` / `Plugin` / `Message`
   (contract hooks, state resources, conventions plugin).
3. Bevy reflection (`Reflect`, `AppTypeRegistry`,
   `ReflectComponent`) — diagnostics tier detection, TestApi
   dynamic-field fallback.
4. `bevy::input::keyboard::KeyCode`, `Vec2`, `Entity` in intent types.
5. Schedule introspection (`Schedules`, last_run ticks) for
   SystemCoverage snapshotting.
6. `bevy::app::App` driving (calibration, branch matrix runner, BRP
   client assumptions in external.rs probe half).

## Verification

- `cargo build --manifest-path core_spike/Cargo.toml` — zero errors, zero warnings
- `cargo test --manifest-path core_spike/Cargo.toml` — 56 passed + 1 doctest
