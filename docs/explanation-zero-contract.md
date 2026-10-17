# Explanation: The Zero-Contract Philosophy

*Understanding-oriented — why bevy_playtest requires nothing from your
game, why the tiers exist, and why invariants beat scripted tests.*

## The problem with testing games

UI tests (Selenium/Playwright-style) answer "did pressing X produce
effect Y?" — they're brittle WHEN-THEN scripts. Game QA needs richer
questions: "is the economy conserved?", "can the player ever get
stuck?", "does anything go NaN?", "does the game stay alive under
random input?". Writing those as click-scripts is absurd; writing
them as per-frame properties is natural.

So bevy_playtest's primary authoring surface is the **invariant** — a
declared property checked every frame against the live world, by bots
that explore input space far wider than a human script. You write
WHAT must hold; the harness finds WHEN it doesn't.

## Why zero-contract matters

Most testing tools demand investment before first signal: learn an
SDK, instrument your code, adapt your architecture. Games are
prototypes — you don't know if the game survives the month, let alone
whether it's worth a test SDK. So the FIRST five minutes must cost
nothing:

```rust,no_run
# fn main() {
let report = bevy_playtest::autotest(GamePlugin, AutotestConfig::default())?;
# }
```

Chaos bots hammer raw inputs, generic oracles (panics, log errors,
soft-locks, NaN transforms, frame-time anomalies, liveness) watch the
world, and you get a report. No `TestApi`, no `IntentSurface`, no game
changes. If that's all you ever use, you still caught whole bug
classes (crash-under-random-input is notoriously effective).

## The tier ladder (and why each rung is optional)

Detection is automatic — the harness probes the world and states
what it found in `report.scope`:

| Tier | Detected by | Unlocks |
|---|---|---|
| `zero` | plugin alone | raw-input chaos, generic oracles |
| `observable` | `Name`s, reflection registration | stable entity identity, per-entity percepts |
| `semantic` | `TestApi`, `IntentSurface`, typed oracles, hooks | intent bots, planner goals, TestApi invariants |

The design rules the tiers enforce:

1. **A missing contract piece is never a rejection by itself.** A
   scenario not referencing intents runs fine without an
   `IntentSurface`. Only scenarios that NEED a missing piece fail —
   loudly, with which piece and how to provide it.
2. **No vacuous passes.** The report states what was NOT checked
   (`scope.inactive`, each with a reason). A pass means "everything
   this scenario asked for held", not "the harness didn't look".
3. **Depth is permanent.** The in-process tier is structurally richer
   than any black-box external tier — same-schedule, same-binary
   execution, bit-exact replay, scheduler and RNG control. External
   tiers approach, never replace.

## Why not full E2E black-box?

External-only tools own their input pipeline — synthetic clicks that
approximate the real ones. bevy_playtest instead runs INSIDE the Bevy
app: bots write the same typed `UserIntent` messages your systems
consume, oracles query the same ECS world your systems mutate. There
is no adapter layer to drift, no screenshot heuristics, no
"element found but invisible" ambiguity. The cost is the lib-target
requirement; the payoff is structural fidelity.

## The philosophy in one line

**Meet the game where it is.** Zero contract for the first five
minutes; a deliberately thin, optional contract for depth; and the
report always tells you exactly where you are on the ladder.

## Related

- [Contract tiers reference](reference-contract-tiers.md) — the full
  detection rules and unlock table
- [Architecture explanation](explanation-architecture.md) — how the
  harness executes inside your app
- [Determinism explanation](explanation-determinism.md) — why
  replay/minimization need determinism, and how it's achieved
