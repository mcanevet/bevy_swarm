# Rule-Name Registry

All oracle rule names emitted in reports. Source of truth: `src/rules.rs`.
Expectation files (X2) reference only these names.

| Constant | Rule name | Kind |
|---|---|---|
| `FROZEN_WORLD` | `frozen_world` | determinism |
| `NODES_IN_BOUNDS` | `nodes_in_bounds` | determinism |
| `FINITE_TRANSFORMS` | `finite_transforms` | determinism |
| `FRAME_TIME_ANOMALY` | `frame_time_anomaly` | informational |
| `FRAME_TIME_P99_BELOW` | `frame_time_p99_below` | informational |
| `FPS_FLOOR` | `fps_floor` | informational |
| `METAMORPHIC_TICK_RATE` | `metamorphic_tick_rate` | determinism |
| `AGGREGATE_CONSERVATION` | `aggregate_conservation` | determinism |
| `NONDETERMINISTIC` | `nondeterministic` | determinism |
| `GOLDEN_DIVERGED` | `golden_diverged` | determinism |
| `DEAD_WIDGET` | `dead_widget` | informational |
| `DEAD_VERB` | `dead_verb` | informational |
| `PERF_BUDGET_EXCEEDED` | `perf_budget_exceeded` | informational |

Notes:
- `crash` is a report **status**, not a rule.
- Informational rules are excluded from digests, failure signatures,
  fingerprint gating and sweep pass/fail (see docs/conventions.md §8).
