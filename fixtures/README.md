# Fixtures

Plain Bevy games for the outer-loop acceptance suite. Zero bevy_swarm plumbing.

## Fixture list

| Fixture        | Description                          | Bug variants (FIXTURE_BUG)                    |
|----------------|--------------------------------------|-----------------------------------------------|
| spinner        | Rotating entity                      | `frozen` (freeze world), `desync`             |
| walker         | 2D top-down movement                 | `desync`, `dead_left_key`                     |
| spawner        | Entity spawner                       | `leak`                                        |
| turn_based     | Card game vs AI                      | `checkop_boundary`                            |
| regression_pair| Golden-divergence pair               | `v2_damage`, `v2_slow_menu`                   |

## Adding a fixture

1. Create `fixtures/<game>/` with `Cargo.toml`, `src/lib.rs`, `src/main.rs`, `README.md`.
2. Depend on **bevy only** (no bevy_swarm).
3. Wire bugs via `FIXTURE_BUG` env var at startup (one build per fixture).
4. Add adapter in `tests/fixture-runner/src/lib.rs` + arm in `main.rs`.
5. Write expectation files under `tests/swarm/golden/<game>/` (clean.json + bug_*.json).

## Bug → Rule → Bead mapping

| Fixture       | Bug variant         | Expected rule(s)      | Owner bead (pending) |
|---------------|---------------------|-----------------------|----------------------|
| spinner       | frozen              | frozen_world          | A1 (adapter)         |
| spinner       | desync              | frozen_world          | A4 (adapter)         |
| walker        | desync              | frozen_world          | A4 (adapter)         |
| walker        | dead_left_key       | dead_verb             | B1, I5 (adapter)     |
| spawner       | leak                | entity_count_bound    | S1 (pending)         |
| turn_based    | checkop_boundary    | custom (score=50)     | C1                   |
| regression_pair| v2_damage          | golden_diverged       | Z11 (pending)        |

See `docs/rules.md` for rule definitions and `docs/conventions.md` for the acceptance protocol.
