# Rule Catalog

GENERATED from `src/rules.rs` (`Rule` enum) — do not edit by hand.
Regenerate with `BEVY_SWARM_UPDATE=schema cargo test --test docs_generation`.
Source of truth: `bevy_swarm::rules::Rule`.
Expectation files (X2) reference only these names.

## Index

| Rule name | Kind |
|---|---|
| `frozen_world` | major |
| `undrivable_game` | informational |
| `harness_could_not_drive_game` | major |
| `bevy_error` | major |
| `bevy_warning` | major |
| `log_error` | major |
| `log_warn` | major |
| `panic` | major |
| `planner_goal_unfinished` | major |
| `bevy_suppressed` | informational |
| `nodes_in_bounds` | major |
| `finite_transforms` | major |
| `frame_time_anomaly` | informational |
| `frame_time_p99_below` | informational |
| `fps_floor` | informational |
| `metamorphic_tick_rate` | major |
| `aggregate_conservation` | major |
| `nondeterministic` | major |
| `golden_diverged` | major |
| `dead_widget` | informational |
| `dead_verb` | major |
| `unused_input` | informational |
| `transform_desync` | major |
| `perf_budget_exceeded` | informational |
| `stuck_after_intent` | major |
| `planner_select_target_missing` | major |
| `reset_hook_panicked` | major |
| `cheat_hook_panicked` | major |
| `nodes_rotation_unnormalized` | major |
| `readiness_timeout` | major |
| `planner_postcondition` | major |
| `planner_goal_timeout` | major |
| `planner_bot_config` | major |
| `planner_any_exhausted` | major |
| `oracle_error` | major |
| `chaos_bot_config` | major |
| `custom_bot_config` | major |
| `pursuit_bot_config` | major |
| `replay_target_missing` | major |
| `synthetic_pointer_config` | major |
| `synthetic_keyboard_config` | major |
| `raw_key_invalid` | major |
| `raw_gamepad_button_invalid` | major |
| `raw_gamepad_axis_invalid` | major |
| `raw_gamepad_disabled` | major |
| `raw_click_target_offscreen` | major |
| `pointer_not_actionable` | major |
| `chaos_select_unnamed` | major |
| `entropy_unattributed_draws` | informational |
| `scenario_error` | major |

## Reference

### `frozen_world`

**Kind:** major (fails the run) — Entities must keep changing: a world that stops updating while the game is running is a soft-lock.

**Detects:** Per-frame gameplay-entity digest unchanged for N consecutive frames while not paused.

**False-positive modes:** Legitimately static scenes (menus, cutscenes) — scope with the Gameplay marker or pause hooks.

### `undrivable_game`

**Kind:** informational (reported, never gates run status) — The bot's actions had no observable effect; the game may not be drivable by the configured input.

**Detects:** Post-run analysis: zero percept changes across the entire action log.

**False-positive modes:** Games whose state only changes via time-based schedules with no input dependence.

### `harness_could_not_drive_game`

**Kind:** major (fails the run) — The harness could not drive the game: enough actions were injected but none had any effect.

**Detects:** End-of-run vacuity check: >= 20 audited actions and an overall effect rate of exactly 0.

**False-positive modes:** Menus needing clicks the monkey cannot find; input gated behind state; broken headless build.

### `bevy_error`

**Kind:** major (fails the run) — Bevy reported an error (bevy_log/LogPlugin error stream).

**Detects:** ERROR-level record routed through the injected Bevy log subscriber.

**False-positive modes:** Upstream Bevy errors that are benign in headless mode (asset loader warnings, GPU absence).

### `bevy_warning`

**Kind:** major (fails the run) — Bevy reported a warning (bevy_log/LogPlugin warn stream).

**Detects:** WARN-level record routed through the injected Bevy log subscriber.

**False-positive modes:** Warnings that only apply to rendering backends absent in headless runs.

### `log_error`

**Kind:** major (fails the run) — Game code logged at error level (tracing subscriber).

**Detects:** ERROR-level tracing event captured by the harness sink.

**False-positive modes:** Game logs transient recoverable errors by design (retry logic).

### `log_warn`

**Kind:** major (fails the run) — Game code logged at warn level (tracing subscriber).

**Detects:** WARN-level tracing event captured by the harness sink.

**False-positive modes:** Verbose games log warnings for normal operating conditions.

### `panic`

**Kind:** major (fails the run) — A system panicked during the run (caught by the panic hook).

**Detects:** std::panic hook fires during app.update() or setup.

**False-positive modes:** Panic in unwinding recovery wrappers that the game catches elsewhere.

### `planner_goal_unfinished`

**Kind:** major (fails the run) — A planner goal tree was abandoned without achieving its goal.

**Detects:** Planner exhausts the tree with goals still unachieved at duration end.

**False-positive modes:** Goals achievable only via paths not exposed in the declared intents.

### `bevy_suppressed`

**Kind:** informational (reported, never gates run status) — A known-benign Bevy warning was suppressed by the harness's filter list.

**Detects:** A log record matching the known-benign suppression list.

**False-positive modes:** By design: only known-benign records are suppressed; check the list when new warnings appear.

### `nodes_in_bounds`

**Kind:** major (fails the run) — Gameplay entity positions stay inside declared play-space bounds.

**Detects:** Any Gameplay entity leaves [min,max] per axis of the nodes_in_bounds invariant.

**False-positive modes:** Bounds declared tighter than real level extents (off-screen-but-valid movement).

### `finite_transforms`

**Kind:** major (fails the run) — Every entity's Transform holds finite values (no NaN/inf).

**Detects:** Any Translation/Rotation/Scale component is NaN or infinite.

**False-positive modes:** Placeholder transforms at spawn before initialization.

### `frame_time_anomaly`

**Kind:** informational (reported, never gates run status) — Frame time deviated abnormally from the run's Welford mean (smoothed, transient spikes ignored).

**Detects:** Frame time exceeds the Welford running mean by k sigma (transient tolerated).

**False-positive modes:** First-frame compile spikes; the smoothed detector mostly ignores them, but heavy headless scheduling can still trip it.

### `frame_time_p99_below`

**Kind:** informational (reported, never gates run status) — Rolling per-frame p99 stayed below a configured floor — suspiciously fast, likely a frozen loop.

**Detects:** Rolling p99 frame time < configured floor for the window.

**False-positive modes:** Trivially light scenes on fast machines.

### `fps_floor`

**Kind:** informational (reported, never gates run status) — Measured FPS fell below a configured floor.

**Detects:** Smoothed FPS < configured floor.

**False-positive modes:** Debug builds with heavy logging; headless on busy CI runners.

### `metamorphic_tick_rate`

**Kind:** major (fails the run) — Behavior differs between tick rates — a determinism/timing bug surfaced by metamorphic comparison.

**Detects:** Digest mismatch between the base run and a re-run at a different tps.

**False-positive modes:** Games legitimately sensitive to tick rate (animation blending).

### `aggregate_conservation`

**Kind:** major (fails the run) — A declared conserved aggregate (total HP, currency, mass) changed outside allowed flows.

**Detects:** Sum/derivative of the tracked aggregate violates the declared conservation law.

**False-positive modes:** Aggregates with legit external flows (level-up grants) — declare the flows.

### `nondeterministic`

**Kind:** major (fails the run) — Same seed produced different per-frame state digests across repeated runs.

**Detects:** Two runs with identical seed produce different digests at some frame.

**False-positive modes:** Float non-associativity across thread counts — use single_threaded mode.

### `golden_diverged`

**Kind:** major (fails the run) — The run's state trajectory diverged from the recorded golden trajectory beyond tolerance.

**Detects:** State digest at frame k differs from the golden file beyond tolerance.

**False-positive modes:** Legitimate behavior changes after intentional edits — regenerate goldens.

### `dead_widget`

**Kind:** informational (reported, never gates run status) — A UI widget never reacted to any interaction during the whole run.

**Detects:** A picked UI node receives no effective interaction over the whole run.

**False-positive modes:** Decorative widgets that correctly never react.

### `dead_verb`

**Kind:** major (fails the run) — A declared IntentSurface verb was never observed to trigger any game effect.

**Detects:** No percept delta is attributable to a verb despite it being emitted.

**False-positive modes:** Verb effects invisible to the observable percepts (internal state only).

### `unused_input`

**Kind:** informational (reported, never gates run status) — A raw input in the declared surface was never observed to change any percept.

**Detects:** Differential probing shows zero effect breadth for a raw input.

**False-positive modes:** Input consumed deep in subsystems not visible via percepts.

### `transform_desync`

**Kind:** major (fails the run) — Transform and GlobalTransform disagreed beyond a tolerance (missing transform propagation).

**Detects:** GlobalTransform recomputation disagrees with stored Transform beyond epsilon.

**False-positive modes:** Manually-set GlobalTransforms (cached by design, e.g. UI layout).

### `perf_budget_exceeded`

**Kind:** informational (reported, never gates run status) — A performance budget (frame time, memory) exceeded its baseline.

**Detects:** Measured metric above its recorded baseline budget.

**False-positive modes:** Machine noise — always compare against same-platform baselines.

### `stuck_after_intent`

**Kind:** major (fails the run) — After an intent fired, no liveness signal followed (soft-lock with input).

**Detects:** No liveness signal within the timeout following a fired intent.

**False-positive modes:** Long deliberate animations before visible reaction; widen timeout_s.

### `planner_select_target_missing`

**Kind:** major (fails the run) — A planner 'select' referenced a target Name/StableId not found in the world.

**Detects:** Select resolution finds no Name/StableId match at fire time.

**False-positive modes:** Entities named differently at runtime (dynamic names).

### `reset_hook_panicked`

**Kind:** major (fails the run) — The game's ResetHooks handler panicked.

**Detects:** Panic caught while invoking ResetHooks.

**False-positive modes:** Reset hooks assuming resources only present after full startup.

### `cheat_hook_panicked`

**Kind:** major (fails the run) — The game's CheatHooks handler panicked.

**Detects:** Panic caught while invoking CheatHooks.

**False-positive modes:** Cheat hooks referencing dev-only entities.

### `nodes_rotation_unnormalized`

**Kind:** major (fails the run) — A rotation quaternion drifted from unit length (accumulated float error).

**Detects:** |quat.length() - 1| > epsilon on a Gameplay entity.

**False-positive modes:** None known; renormalization is cheap and should hold.

### `readiness_timeout`

**Kind:** major (fails the run) — The game never reached a ready state within the configured startup budget.

**Detects:** Readiness predicate still false after readiness budget elapses.

**False-positive modes:** Slow asset loading on cold CI caches — raise the budget.

### `planner_postcondition`

**Kind:** major (fails the run) — A planner goal marked achieved failed its postcondition shortly after.

**Detects:** Postcondition predicate fails within the stability window after achievement.

**False-positive modes:** Postcondition windows shorter than latency in the game's response.

### `planner_goal_timeout`

**Kind:** major (fails the run) — A planner goal exceeded its max_s deadline without achieving.

**Detects:** max_s elapses on a Seq/Repeat/Any node without child achievement.

**False-positive modes:** max_s tuned for faster play than the game allows.

### `planner_bot_config`

**Kind:** major (fails the run) — The planner bot's scenario configuration is invalid or incomplete.

**Detects:** Planner bot loaded with missing goals, bad paths, or unknown predicates.

**False-positive modes:** None known — config errors should fail loudly.

### `planner_any_exhausted`

**Kind:** major (fails the run) — A planner Any-combinator exhausted all children without success.

**Detects:** All Any children timed out or failed.

**False-positive modes:** All alternatives genuinely requiring sequential play order.

### `oracle_error`

**Kind:** major (fails the run) — An oracle itself errored while evaluating (infrastructure failure, not a game bug).

**Detects:** Oracle system returned Err / panicked internally.

**False-positive modes:** Oracle infrastructure bugs — report them.

### `chaos_bot_config`

**Kind:** major (fails the run) — The chaos bot's scenario configuration is invalid or incomplete.

**Detects:** Chaos bot with an empty surface, missing seed, or unsupported combination.

**False-positive modes:** None known — config errors should fail loudly.

### `custom_bot_config`

**Kind:** major (fails the run) — A custom named bot policy's configuration is invalid or unknown.

**Detects:** Named policy not registered via add_bot_policy.

**False-positive modes:** Policy registered after scenario load in a different app instance.

### `pursuit_bot_config`

**Kind:** major (fails the run) — The pursuit bot's scenario configuration is invalid or incomplete.

**Detects:** Pursuit bot with unresolved agent/target names.

**False-positive modes:** Names differing between record and replay scenes.

### `replay_target_missing`

**Kind:** major (fails the run) — A replay step referenced a target entity that no longer exists.

**Detects:** Replay step target Name/StableId absent from live world.

**False-positive modes:** Replay recorded on a different scene/build.

### `synthetic_pointer_config`

**Kind:** major (fails the run) — The synthetic_pointer bot's configuration is invalid (e.g. missing picking backend).

**Detects:** synthetic_pointer without a picking backend or camera.

**False-positive modes:** Games relying on custom picking backends the harness cannot probe.

### `synthetic_keyboard_config`

**Kind:** major (fails the run) — The synthetic_keyboard bot's configuration is invalid.

**Detects:** synthetic_keyboard with unparseable key names.

**False-positive modes:** None known.

### `raw_key_invalid`

**Kind:** major (fails the run) — A raw-input key name did not parse to a KeyCode.

**Detects:** parse_key_code returns None for a RawAction::Key.

**False-positive modes:** Key aliases not in the common subset map — extend parse_key_code.

### `raw_gamepad_button_invalid`

**Kind:** major (fails the run) — A raw-input gamepad button name is unknown.

**Detects:** Gamepad button string not a known Gilrs button.

**False-positive modes:** Vendor-specific button naming.

### `raw_gamepad_axis_invalid`

**Kind:** major (fails the run) — A raw-input gamepad axis name is unknown.

**Detects:** Gamepad axis string not a known axis.

**False-positive modes:** Vendor-specific axis naming.

### `raw_gamepad_disabled`

**Kind:** major (fails the run) — Raw gamepad input was requested but the gamepad feature is off.

**Detects:** RawAction::Gamepad* while the gamepad feature is disabled.

**False-positive modes:** Forgot the gamepad cargo feature.

### `raw_click_target_offscreen`

**Kind:** major (fails the run) — A raw click targeted a position outside the camera viewport.

**Detects:** Click pos fails inside the computed camera viewport rect.

**False-positive modes:** Letterboxed viewports where logical pixels differ from camera coords.

### `pointer_not_actionable`

**Kind:** major (fails the run) — A synthetic click completed but the picking map showed nothing receivable (masked/dead backend).

**Detects:** require_actionable set and picking hover map empty after the gesture.

**False-positive modes:** Masked-but-functioning overlays; drop require_actionable if intentional.

### `chaos_select_unnamed`

**Kind:** major (fails the run) — The chaos bot tried to select among entities but they carry no Name.

**Detects:** Chaos Select drawn while candidate entities lack Name components.

**False-positive modes:** Gameplay entities without Name by design — use StableId targeting.

### `entropy_unattributed_draws`

**Kind:** informational (reported, never gates run status) — getrandom draws occurred outside a run's attribution window.

**Detects:** getrandom call outside an attributed run window.

**False-positive modes:** Background threads drawing entropy concurrently with runs.

### `scenario_error`

**Kind:** major (fails the run) — The scenario itself failed (rejected config, unloadable state) rather than the game misbehaving.

**Detects:** Load-time or runtime ScenarioError paths.

**False-positive modes:** None known — the error text names the rejected field.

Notes:
- `crash` is a report **status**, not a rule.
- Informational rules are excluded from digests, failure signatures,
fingerprint gating and sweep pass/fail (see docs/conventions.md §8).
