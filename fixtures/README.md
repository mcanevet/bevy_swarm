# Fixtures

Plain Bevy games for the outer-loop acceptance suite. Zero bevy_swarm plumbing.

## Fixture list

| Fixture        | Description                          | Bug variants (FIXTURE_BUG)                    |
|----------------|--------------------------------------|-----------------------------------------------|
| spinner        | Rotating entity                      | `frozen` (freeze world)                       |
| walker         | 2D top-down movement                 | `desync`, `dead_left_key`, `stuck_corner`, `nan_rotation`, `child_out_of_bounds`, `panic_on_edge`, `panic_on_load`, `select_unnamed`, `dangling_target` |
| spawner        | Entity spawner                       | `leak`                                        |
| turn_based     | Card game vs AI                      | `checkop_boundary`, `softlock_turn_4`, `thread_rng`, `hashmap_order`, `unconsumed_event` |
| regression_pair| Golden-divergence pair               | `v2_damage`, `v2_slow_menu`                   |

## Adding a fixture

1. Create `fixtures/<game>/` with `Cargo.toml`, `src/lib.rs`, `src/main.rs`, `README.md`.
2. Depend on **bevy only** (no bevy_swarm).
3. Wire bugs via `FIXTURE_BUG` env var at startup (one build per fixture).
4. Add adapter in `tests/fixture-runner/src/lib.rs` + arm in `main.rs`.
5. Write expectation files under `tests/swarm/expectations/<game>/` (clean.json + bug_*.json).

## Bug → Rule → Bead mapping

| Fixture       | Bug variant         | Expected rule(s)      | Owner bead (pending) |
|---------------|---------------------|-----------------------|----------------------|
| spinner       | frozen              | frozen_world          | FX2                  |
| walker        | desync              | transform_desync      | FX2                  |
| walker        | dead_left_key       | dead_verb             | FX2                  |
| walker        | stuck_corner        | frozen_world          | FX2                  |
| walker        | nan_rotation        | finite_transforms     | FX2                  |
| walker        | child_out_of_bounds | nodes_in_bounds       | FX2                  |
| walker        | panic_on_edge       | crash                 | FX2                  |
| walker        | panic_on_load       | crash                 | FX2                  |
| walker        | select_unnamed      | crash                 | swarm-qo7.13 (pending) |
| walker        | dangling_target    | dangling_entity       | swarm-716.24 (pending) |
| spawner       | leak                | entity_count_bound    | S1 (pending)         |
| turn_based    | checkop_boundary    | custom (score=50)     | FX2                  |
| turn_based    | softlock_turn_4     | stuck_after_intent    | FX2                  |
| turn_based    | thread_rng          | nondeterministic      | swarm-1w8.13 (pending) |
| turn_based    | hashmap_order       | nondeterministic      | swarm-1w8.13 (pending) |
| turn_based    | unconsumed_event    | unconsumed_message    | swarm-716.24 (pending) |
| regression_pair| v2_damage          | golden_diverged       | swarm-qo7.9 (pending) |

See `docs/rules.md` for rule definitions and `docs/conventions.md` for the acceptance protocol.
