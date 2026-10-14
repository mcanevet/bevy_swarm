# Explanation: Determinism Strategy

*Understanding-oriented — why replay and minimization need
determinism, how bevy_swarm achieves it, and where the remaining
sharp edges are.*

## Why determinism is load-bearing

bevy_swarm's failure workflow promises: a red run gives you a
**seed**, re-running that seed reproduces the failure, and
minimization can shrink the failing input sequence to a committable
regression scenario. All three are lies if the scenario isn't
deterministic — the same "replay" hits a different world state and
the bug vanishes (or worse, an innocent one appears).

So determinism is verified, not assumed:

```rust,no_run
# fn main() {
let report = bevy_swarm::determinism::check_determinism(
    || build_app(), &scenario, 2)?;
if !report.is_deterministic() {
    println!("{:?}", report.first_divergence);
}
# }
```

The ggrs-SyncTest approach: run the scenario multiple times on fresh
`App`s, record per-frame canonical state digests, compare. On
divergence you get the first divergent frame and the differing
fields (`Gameplay[Ball].translation.x`) — the strongest debugging
hint the harness gives.

## The four sources of nondeterminism, handled

### 1. Scheduling

Bevy systems may run in parallel in unspecified order. The harness
swaps in `SingleThreadedExecutor` — one thread, stable order, same
results every run. (Registered systems still execute; CA² coverage
snapshots confirm it — see the architecture explanation.)

### 2. Operating-system entropy

`getrandom` (hence `rand::rng()`, `uuid::new_v4`, `bevy_rand`)
silently injects true randomness into "deterministic" games. The
`deterministic-entropy` feature intercepts it at the source:

- Every `getrandom` call inside a run draws from a stream seeded
  from the scenario seed — identical replay, identical draws
- Calls OUTSIDE a run (startup, between seeds) draw from a
  process-global fallback and increment the
  `entropy_unattributed_draws` informational counter — you learn
  your game draws entropy at odd times without failing spurious

Enable with both the feature AND the cfg (they go together — the
harness's own `cargo swarm` wrapper plans to set both):

```sh
RUSTFLAGS='--cfg getrandom_backend="custom"' \
  cargo test --features bevy_swarm/deterministic-entropy
```

### 3. Wall-clock time

Systems reading real time get different deltas per run. This is a
game-code concern the harness surfaces (via digest divergence) rather
than removes; games that need it read from a mockable clock resource.

### 4. Thread-local RNG caching

`rand::rng()` caches a `ThreadRng` per thread — even the entropy hook
can't fully order draws made from a reused thread across runs. Hence
the rule: **each scenario runs on a fresh thread**. The matrix runner
(`run_scenarios_parallel`) does this automatically; direct
`run_scenario` callers must spawn one thread per scenario or accept
documented risk.

## What digests are made of

Per-frame canonical state digests serialize the game world into a
comparable form: gameplay-relevant components, transforms, the
scenario's own RNG state. Games contribute custom parts (AI
blackboards, custom RNG state) via the `DigestHooks` resource.

Cost control: digests are only computed when a `StateTrace` resource
is present — normal playtests pay nothing.

## Honest failure: `NotReproducible`

When minimization can't reproduce a failure from the action log
alone (dependence on something outside the log — unseeded entropy,
wall clock, state not covered by resets), you get an explicit
`NotReproducible` error instead of a fake "minimal reproduction".
This honesty matters more than the minimization itself: a tool that
lies about reproduction wastes more debugging time than no tool.

## The hook, precisely

Enable the `deterministic-entropy` feature AND set the cfg (they go
together — a `cargo swarm run` wrapper that sets both automatically
is planned):

```sh
RUSTFLAGS='--cfg getrandom_backend="custom"' \
  cargo test --features bevy_swarm/deterministic-entropy
```

The custom backend hook (`__getrandom_v03_custom`) intercepts all
`getrandom` calls — including `rand::rng()`, `uuid::Uuid::new_v4`,
`bevy_rand` — and draws from a deterministic stream seeded from the
scenario seed. Calls outside `enter_run()` draw from a process-global
fallback stream and increment the `entropy_unattributed_draws`
informational rule counter — you learn your game draws entropy at odd
times without failing spuriously.

The signature must match exactly:

```rust,ignore
#[unsafe(no_mangle)]
pub unsafe extern "Rust" fn __getrandom_v03_custom(
    dest: *mut u8,
    len: usize,
) -> Result<(), getrandom::Error>
```

Using `extern "C"` or returning `Result<(), ()>` is undefined behavior
(UB) and will abort the process on panic.

And the `ThreadRng` caveat bears repeating: `rand::rng()` caches a
`ThreadRng` per thread, so **each scenario must run on a fresh
thread** to guarantee determinism across runs. The matrix runner
(`run_scenarios_parallel`) does this automatically; direct
`run_scenario` callers must either (a) spawn a fresh thread per call,
or (b) accept documented risk of cross-run ThreadRng state leakage.
The `InProcess` runner calls `enter_run` before the app factory so
plugin-build draws are seeded.

## Related

- [Deterministic entropy](../README.md#z14-deterministic-entropy-hook)
  — the custom-backend
  signature and UB caveats
- [How-to: debug a failing run](howto-debug-failure.md) — where
  determinism failures show up in practice
- [Zero-contract explanation](explanation-zero-contract.md) — why
  determinism is opt-in depth rather than a day-one requirement
