# Rule-Name Registry

All oracle rule names emitted in reports. Source of truth: `src/rules.rs`.
Expectation files (X2) reference only these names.

| Rule name | Kind |
|---|---|
| `frozen_world` | major |
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

Notes:
- `crash` is a report **status**, not a rule.
- Informational rules are excluded from digests, failure signatures,
  fingerprint gating and sweep pass/fail (see docs/conventions.md §8).
