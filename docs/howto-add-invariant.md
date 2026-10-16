# How-to: Add an Invariant

*Goal-oriented — you know a property your game must hold and want the
harness to check it during playtests.*

Invariants live in the scenario, expressed in the JSON DSL (or the
equivalent [`ScenarioBuilder`](../src/builder.rs) Rust API). Everything
validates offline:

```sh
cargo bevy-swarm validate my-scenario.json
```

## Threshold invariant (TestApi path)

The workhorse: compare a numeric/string path against a threshold.

```json
{ "name": "hp_floor", "rule": "custom",
  "path": "TestApi.hp", "check": "ge", "value": 10 }
```

Checks: `lt` `le` `gt` `ge` `equals` `ne` (numeric or string).
Checks apply per-frame by default — **always-mode**.

## Eventually (deadline) invariant

Condition must hold at least once within `eventually_s` seconds —
"the menu reaches 'options'" style assertions:

```json
{ "name": "reaches_options", "rule": "custom",
  "path": "TestApi.menu_phase", "check": "equals", "value": "options",
  "eventually_s": 8 }
```

## Time-windowed invariant

Only check between `after_s` and `before_s`:

```json
{ "name": "no_spawn_before_grace", "rule": "custom",
  "path": "TestApi.enemy_count", "check": "equals", "value": 0,
  "after_s": 0.0, "before_s": 5.0 }
```

## Bounds invariant (positions)

```json
{ "name": "player_in_arena", "rule": "nodes_in_bounds",
  "targets": ["Player"],
  "min_x": -500.0, "max_x": 500.0,
  "min_z": -500.0, "max_z": 500.0 }
```

`targets` names entities by `Name`; empty = all `Gameplay` entities.

## Entity-count invariant (query)

The ECS analogue of Playwright's `toHaveCount`:

```json
{ "name": "enemy_cap", "rule": "custom",
  "query": { "with": ["EnemyTag"], "without": ["DeadTag"] },
  "check": "le", "value": 200 }
```

## Differential invariant (never rises/drops)

Per-frame comparison to the PREVIOUS value — conservation and monotonic
properties:

```json
{ "name": "score_monotonic", "rule": "custom",
  "path": "TestApi.score", "check": "ge", "value": 0,
  "differential": "no_decrease" }
```

## Rate-limit invariant

Bound how FAST a value may change:

```json
{ "name": "energy_drain", "rule": "custom",
  "path": "TestApi.energy", "check": "ge", "value": 0,
  "max_delta_per_sec": 25.0 }
```

## Expert rule (WHEN → REQUIRE)

Policy compliance: "low HP implies healing active":

```json
{ "name": "low_hp_heals", "rule": "custom",
  "path": "TestApi.player_hp", "check": "le", "value": 20,
  "requires_path": "TestApi.healing_active",
  "requires_check": "equals", "requires_value": true }
```

## Performance floors

```json
{ "name": "fps_playable", "rule": "fps_floor", "value": 50 }
```

## Tuning knobs

- **`scale`** — normalize robustness units (default
  `max(|threshold|, 1.0)`); matters only for the ρ report
- **Severity** — informational rules (`fps_floor`,
  `frame_time_p99_below`) never gate; promote warning-class
  (`bevy_warning`, `log_warn`) with scenario-level
  `"strict_warnings": true`

## Robustness margins (what you get back)

Every invariant computes a signed robustness ρ (STL-style distance to
violation) in addition to the boolean verdict: positive when
satisfied — "passed, but came within 0.3 of the bound" — negative
when violated. `report.robustness.per_invariant` carries the minimum
ρ, the worst frame, and the normalized value. This is the substrate
for guided search: see the robustness section of the README.

## Pitfalls (validated at load — you can't get these wrong silently)

- Duplicate invariant names — rejected (they'd shadow each other)
- Numeric checks against string values — rejected
- `requires_*` without a WHEN path — rejected
- `after_s >= before_s`, inverted bounds, negative rates — rejected
- Unknown keys, resets, cheats, oracles — rejected, with did-you-mean

All of these surface in `cargo bevy-swarm validate` and at load time,
never mid-run.
