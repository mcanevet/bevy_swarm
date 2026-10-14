# How-to: Smoke-Test a Game You Don't Own

*Goal-oriented — you want signal on a Bevy game WITHOUT adding a
dev-dependency or a lib target.*

The in-process mode (see the [first-playtest tutorial](tutorial-first-playtest.md))
is the deepest tier, but it requires importing the game as a library.
When you can't (or won't) touch the code, the external tier works in
about 30 seconds:

## The BRP tier (observe-only)

Bevy's `bevy_remote` feature exposes a query/inspect protocol over
HTTP. bevy_swarm's CLI can connect, observe, and run generic oracles:

```sh
# game side: one documented build flag, zero code
cargo run --features bevy/bevy_remote

# harness side:
cargo swarm smoke <brp-url> --duration 30
```

You get: crash detection, log-error capture, liveness, state digests.
You don't get: intent bots, typed oracles, replay, minimization —
that's the trade for touching zero code.

## What "zero-contract" means here

Enabling a cargo feature is a **build-time** contract (thin, but not
nothing), and the external tier is observe-only at runtime. The
[contract-tiers reference](reference-contract-tiers.md) lays out the
full ladder and what each rung buys.

## When to graduate

If the smoke run finds something interesting (a liveness stall, error
spam), the next step for an owned codebase is the one-line in-process
mode — the [tutorials](tutorial-first-playtest.md) cover it. For
third-party games, the external tier is the ceiling, by design.
