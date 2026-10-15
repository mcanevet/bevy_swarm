# Explanation: What bevy_swarm Catches

*Understanding-oriented — the failure classes covered, how capture
works, and the known limitations.*

## What it does

- **Typed intent bots** — chaos (with uniform/aggressive/curious/idle
  personas), replay, pursuit, planner (aplib-style goal trees),
  synthetic_pointer and synthetic_keyboard (drive the REAL input
  chain through picking/ButtonInput, not shortcuts)
- **Invariant DSL** — scenario JSON with bounds checks, TestApi
  predicates, eventual assertions, query-target entity counts,
  differential rules (no_decrease/no_increase), rate limits,
  expert-rule oracles (WHEN/REQUIRE)
- **Oracles** — frozen-world soft-lock detection, Welford frame-time
  anomaly detection, true rolling p99, readiness gate,
  finite-transform NaN check (all entities, not only Gameplay)
- **CA² system coverage** — which registered systems actually
  executed, with zero instrumentation (wrap-safe schedule tick-age
  snapshots)
- **Failure minimization** — ddmin over the structured action log
  shrinks ANY failure (crashes and invariant violations) to a
  ready-to-save regression scenario with a trimmed duration, and
  fails honestly with `NotReproducible` when the log alone cannot
  reproduce the failure
- **Calibration** — `diagnostics::calibrate_world` runs a chaos
  session and derives suggested invariants from archetype-count
  envelopes, with zero game annotations (library API; there is no
  CLI flag)
- **Loud contracts** — missing picking backends, unresolved TestApi
  paths, vacuous setups REJECT the scenario instead of silently
  passing

## What we capture (panics, logs, background threads)

- **ECS-thread panics** — systems, observers, commands: fully
  captured with system-name attribution (via the tracing span when
  built with `trace`), severity, and location.
- **bevy_tasks pool-thread panics** (IoTaskPool,
  AsyncComputeTaskPools, ComputeTaskPool) — captured and reported
  with target `pool_thread`. Game code routinely detaches
  background work (asset decode, pathfinding); those panics used to
  escape the harness entirely.
- **Logs** — WARN/ERROR records from any thread with a run active.
- **At run end** the harness waits a bounded grace window (300 ms)
  for detached pool work to land before draining sinks, so late
  panics aren't lost.
- **Known limitation (cross-seed stragglers)**: task pools are
  process-global and never shut down between seeds. A straggler
  task from seed N that completes during seed N+1 is attributed to
  the run active when it lands. For strict isolation, use the
  external subprocess tiers (smoke / BRP-driven).

## What bevy_swarm does NOT check

**Rendering/pixels.** Headless runs strip the render family — a
game that renders nothing would otherwise pass. The structural
checks `no_active_camera` (drawables present, zero active cameras)
and `nothing_on_screen` (nothing projects inside the viewport)
catch the common blank-window causes, but **pixels are never
verified**: every headless run lists "rendering" in
`scope.unchecked` with reason "headless: pixels not verified", and
the summary prints `[rendering not verified]`. Recommendation: run
your game windowed once before shipping. Deeper verification
(screenshots, a `rendering_verified` flag) is planned in
swarm-1tw (v0.3).

## Known experimental limitations

- `autotest()` — false positives on default intent keys and static
  games (RF8)
- raw input (gamepad/pointer injection) — gamepad actions not in
  the audit log (RF16)
- `ClickEntity`/pointer picking — `NoViewportSize` failures without
  a camera (RF11)
- deterministic entropy — `rand::rng()` not reproducible on the same
  thread (RF10)

## The causal chain

The design premise: game bugs live in state evolution across time,
not in single UI interactions. So the coverage story is:

1. **Crashes and errors** — anything that panics or logs an error,
   on ANY thread, gets captured and attributed
   ([architecture](explanation-architecture.md) §panic capture)
2. **State corruption** — NaNs, out-of-bounds positions,
   unnormalized rotations, aggregate conservation (mass/energy
   sums), metamorphic tick-rate properties
3. **Liveness/soft-locks** — frozen worlds, intents with no
   effect, stuck-after-intent, planner timeouts
4. **Declared properties** — whatever YOU say must hold, via the
   [invariant DSL](howto-add-invariant.md)

Related: the [rule catalog](rules.md) is the complete enumeration.
