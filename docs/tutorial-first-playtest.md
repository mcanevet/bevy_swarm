# Tutorial: Your First Playtest

*Learning-oriented — hands-on, ~15 minutes. You will add bevy_playtest to a
small Bevy game, run your first chaotic playtest, and learn to read the
failure output.*

bevy_playtest runs your game headlessly, drives it with bots, and checks
generic oracles — crashes, log errors, soft-locks, NaNs, liveness —
with **zero semantic contract**: no `TestApi`, no `IntentSurface`, no
game-code changes. You write WHAT to test, not WHEN to press.

This tutorial assumes a Bevy 0.20 game whose plugin setup lives in a
`src/lib.rs` (if your game is binary-only, promote the plugin setup
from `main.rs` into `lib.rs` first — it's a mechanical refactor, and
integration tests need the lib target).

**What's a seed?** A seed fixes the bot's random input sequence.
Same seed, same replayable run. Different seeds explore different
sequences.

## Step 1: Add the dev-dependency

```toml
[dev-dependencies]
bevy_playtest = "0.1"
```

## Step 2: Write the playtest

Create `tests/playtest.rs`:

```rust,no_run
use bevy::prelude::*;

// Replace with your game's root plugin.
#[derive(Clone, Default)]
struct MyGamePlugin;
impl Plugin for MyGamePlugin {
    fn build(&self, _: &mut App) {}
}

#[test]
fn swarm_playtest() {
    let report = bevy_playtest::autotest(
        MyGamePlugin,
        bevy_playtest::autotest::AutotestConfig::default(),
    )
    .expect("autotest runs");
    assert!(report.passed(), "{}", report.summary());
}
```

That's the whole wiring. `AutotestConfig::default()` runs 4 seeds × 5
seconds of chaos each — enough for a first signal without slowing
your test loop. When you want deeper coverage, use
`AutotestConfig::thorough()` (16 seeds × 20 seconds).

## Step 3: Run it

```sh
cargo test --test playtest
```

`headless_app()` strips the render/window/audio backends and swaps in
`SingleThreadedExecutor` for deterministic scheduling. Your game
systems run normally — same schedule, same binary, no adapter layer.
Render systems still execute but target no window; guard any systems
that read `Window` state or camera output.

## Step 4: Read the output

On success, you'll see a summary line like:

```
autotest: 4 seeds, 0 failing runs (0 unique), 20s total, 100%, gate: pass
```

On failure, bevy_playtest prints an **inline failure summary** to stderr
*before* writing the report:

```
FAILED: 1 failing runs (1 unique fingerprints)
seed 2: fail (1 violation)
  [dead_verb] ArrowLeft: no gameplay effect observed in 45+ frames [NEW]
  Repro: BEVY_PLAYTEST_SEED=2 cargo test --test <your swarm test>
```

The bracketed name is the violated **rule** (see the
[rule catalog](reference-rules.md)); the detail tells you what was
observed; `[NEW]` marks a fingerprint never seen before (vs `[known]`
— tracked in your issue registry).

## Step 5: Reproduce a failure

Copy the `Repro:` line verbatim to re-run exactly the failing seed:

```sh
BEVY_PLAYTEST_SEED=2 cargo test --test playtest
```

Or use the CLI (after `cargo install --path bevy_playtest_cli`):

```sh
cargo bevy-playtest reproduce            # first failing seed from the last report
cargo bevy-playtest reproduce --fingerprint <fp>  # a specific failure
```

## Where the artifacts live

Every run writes `target/bevy_playtest/runs/<run-id>/`:

- `report.json` — the aggregate (per-seed runs, fingerprints, coverage)
- `seed-<n>.json` — the per-seed `PlaytestReport` with the full action
  log: every input the bot fired, frame-indexed
- `junit.xml` — for CI dashboards
- `regression-<seed>-<rule>.json` — standalone minimized reproductions
  (when minimization is on), ready to commit as regression tests

## What's next

- Found a failure you don't understand? →
  [How-to: debug a failing run](howto-debug-failure.md)
- Want to check game-specific properties (HP floors, entity counts)?
  → [Tutorial: wire your game's TestApi](tutorial-wiring-testapi.md)
- Wondering how this works without touching your game code? →
  [Explanation: the zero-contract philosophy](explanation-zero-contract.md)
