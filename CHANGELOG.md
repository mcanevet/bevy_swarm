# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- New `raw_input` module (Z3): `RawActionQueue`, `ActiveKeyHolds`, `ActiveMouseHolds`, `VirtualGamepad` resources
- `RawAction` enum variants: `Key`, `MouseButton`, `Click`, `MouseMove`, `Cursor`, `Wheel`, `GamepadButton`, `GamepadAxis`, `ClickEntity`, `Wait`
- Systems `raw_input_preupdate_system` (keyboard/gamepad in PreUpdate) and `raw_input_update_system` (mouse/cursor/wheel in Update), wired into `PlaytestPlugin`
- Optional `gamepad` crate feature (`bevy/gamepad`): lazily spawned virtual gamepad emitting `GamepadConnectionEvent`, `RawGamepadButtonChangedEvent`, `RawGamepadAxisChangedEvent`
- Integration tests in `tests/raw_input.rs`
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

