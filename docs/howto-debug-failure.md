# How-to: Debug a Failing Run

*Goal-oriented — you have a red playtest and want to understand,
minimize, and fix the failure.*

## Start from the inline summary

A failing autotest prints to stderr before writing anything:

```
FAILED: 1 failing runs (1 unique fingerprints)
seed 2: fail (1 violation)
  [stuck_after_intent] no liveness within 0.5s after intent choice#2 (frame 214) [NEW]
  Repro: BEVY_PLAYTEST_SEED=2 cargo test --test <your swarm test>
```

Read it top-down: which seed, which rule, the observed detail, and
the fingerprint classification (`[NEW]` / `[known]` / `[muted]` /
`[REGRESSED]`).

## Reproduce it deterministically

```sh
# Exact seed, exact scenario:
BEVY_PLAYTEST_SEED=2 cargo test --test playtest

# Or via the CLI (resolves the seed from the last report):
cargo bevy-playtest reproduce
cargo bevy-playtest reproduce --fingerprint <fp>   # a specific failure
```

## Open the report artifacts

`target/bevy_playtest/runs/<run-id>/`:

| File | What's in it |
|---|---|
| `report.json` | Aggregate: per-seed runs, fingerprints, coverage, gate |
| `seed-2.json` | The failing seed's full report — **action log** included |
| `regression-2-<rule>.json` | Standalone minimal reproduction (if minimization ran) |

The **action log** is every input the bot fired, frame-indexed —
the failing sequence is right there. Pretty-print it:

```sh
cargo bevy-playtest report          # inline summary from the last run
jq '.action_log' target/bevy_playtest/runs/<run-id>/seed-2.json
```

## Minimize the failure

Turn minimization on to shrink the failing input sequence to a
ready-to-commit regression scenario:

```rust,no_run
# fn main() {
let config = bevy_playtest::autotest::AutotestConfig {
    minimize_failures: true,
    ..Default::default()
};
# }
```

ddmin over the action log produces the smallest subsequence that
STILL reproduces the failure, plus a trimmed duration, written as
`regression-<seed>-<rule>.json` next to the report. Commit it as a
regression test directly — it replays verbatim.

If the log alone can't reproduce the failure, you get an honest
`NotReproducible` error instead of a bogus "minimal" scenario — the
bug depends on something outside the log (wall clock, unseeded RNG,
state not covered by resets).

## Decode the rule

Look the rule up in the [rule catalog](reference-rules.md) — every
rule documents what it detects, its false-positive modes, and how to
suppress it when it's benign.

Common cases:

- **`dead_verb`** — an input the game never reacts to. Real bug
  (unbound key) or benign (menu key during gameplay). Scope
  suppression or `raw_surface` adjustment fixes the benign case.
- **`stuck_after_intent`** — soft-lock: intent consumed but no
  liveness followed. Usually a real state-machine bug.
- **`frozen_world`** — no state changed at all for a long window.
  Either the game is genuinely static (menus, attract mode) or
  something is wedged.
- **`nondeterministic`** — same seed, different state digests. See
  [the determinism explanation](explanation-determinism.md).

## Handle nondeterminism

If `check_determinism` flags divergence, the report gives you the
first divergent frame and differing fields:

```
Gameplay[Ball].translation.x: 12.3 vs 12.7 (frame 418)
```

Chase down unordered iteration (sort queries), hidden wall-clock or
`rand::rng()` reads, or per-frame real-time deltas. For the harness
side, the `deterministic-entropy` feature can intercept OS entropy
(see [determinism](explanation-determinism.md)).

## Suppress or track a known failure

Not every failure blocks your CI. Options, from narrowest to broadest:

1. **Mute the fingerprint** — tracked, never gating (issue registry,
   `gate.classification.muted`)
2. **Fix the game** — usually the point
3. **Escalate/de-escalate severities per scenario** —
   `strict_warnings: true` promotes informational rules to gating
   when your game should never log warnings

## When your CI is red

JUnit XML is emitted automatically (`runs/<run-id>/junit.xml`), and
the gate verdict (`report.gate.passed`) implements new-or-regressed
semantics: known/muted failures don't fail CI, NEW fingerprints do.

```sh
cargo bevy-playtest report    # classification + coverage + surface, inline
```
