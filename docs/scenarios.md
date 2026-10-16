# Scenario Cookbook

Realistic, ready-to-adapt scenarios. Each lives in `examples/scenarios/`;
this page walks through what each one exercises and why the fields are
there. Validate any of them offline with:

```sh
cargo bevy-swarm validate examples/scenarios/movement-bug-hunt.json
```

Or author them in Rust with [`ScenarioBuilder`](../src/builder.rs):

```rust
use bevy_swarm::builder::ScenarioBuilder;

let scenario = ScenarioBuilder::chaos()
    .name("movement-bug-hunt")
    .duration_s(30.0)
    .seed(1337)
    .input_rate_hz(12)
    .raw_keys(vec!["KeyW".into(), "KeyA".into(), "KeyS".into()])
    .custom_invariant("hp-floor", "TestApi.player_hp", "ge", 0)
    .fps_floor("fps-stays-playable", 50.0)
    .check_determinism(3)
    .build();
```

---

## menu-navigation.json

A **synthetic_pointer** bot drives menu navigation through the *real*
picking chain: each click resolves the named entity's screen position
from its transform, moves the cursor there, presses and releases the
primary button.

- `require_actionable: true` — Playwright-style actionability gate: if
  the click lands on nothing (occluded UI, dead picking backend), the
  run reports `pointer_not_actionable` instead of silently clicking
  nothing.
- `hold_frames: 30` — long-press interactions (drag sliders).
- The invariant uses **eventually-mode** (`eventually_s: 8`): the menu
  phase must *reach* `"options"` at least once within 8s — unlike the
  default always-mode, which would require the condition to hold on
  every single frame.

## movement-bug-hunt.json

A **chaos** bot hunts movement/collision bugs with an aggressive
persona (2x input rate, never idles).

- `raw_surface` declares exactly what the bot may sample: WASD +
  Space + Shift, mouse buttons, wheel. Unbound keys surface as
  informational `unused_input`, so listing extra keys is safe.
- `persona: "aggressive"` finds different bugs than `"uniform"`
  (default) or `"idle"` (mostly waiting, rare jabs).
- Three invariant styles: positional bounds (`nodes_in_bounds` with
  `targets` naming entities), a TestApi threshold, and an FPS floor.
- `check_determinism: 3` re-runs the same seed three times comparing
  state digests — catches nondeterminism (unsorted queries, hash-map
  iteration, hidden wall-clock reads) in one scenario.

## crash-reproduction.json

A **replay** bot reproduces a minimized crash: frame-indexed typed
intents replayed verbatim, plus mid-run **cheats** to set up the crash
state fast (audited: every invocation lands in the action log).

- `select` by entity **Name** — resolved against live entities at fire
  time, so the replay survives entity-id churn across resets.
- The final input uses `raw` to drive the real keyboard chain — mixed
  typed/raw replays are fine.
- `setup.resets` puts the game in a known start state first.
- `check_determinism: 2` proves the reproduction is stable before you
  file it as a regression.

## invariant-monitoring.json

A slow **idle-chaos** soak (60s, 5 Hz) that mostly watches invariants
while the game sits near its attractors.

- `differential: "no_decrease"` — score may never drop (per-frame
  comparison to the previous value, not a threshold).
- `max_delta_per_sec` — energy drains at most 25/s; faster drains
  flag a runaway system.
- `query` invariants are the ECS analogue of Playwright's
  `toHaveCount`: entity counts filtered by component.
- `requires_*` fields encode game **policy** ("low HP ⇒ healing
  active") — fires only when the WHEN clause holds.

## state-machine-poke.json

A **synthetic_keyboard** bot exercises the full keyboard chain
(`KeyboardInput` → `ButtonInput<KeyCode>` → game) with rapid
pause/unpause/tab sequences.

- Key names accept several spellings: `"Space"`, `"Escape"`, bare
  letters (`"w"`), `"KeyW"` style, digits. See
  [the validate command](#validating-scenarios) — typos get
  did-you-mean suggestions.
- Time-windowed invariants (`after_s`/`before_s`) check that pause
  *actually pauses* during a specific window, not the whole run.
- `strict_warnings: true` escalates `bevy_warning`/`log_warn` from
  informational to gating — this game treats warnings as bugs.

## goal-directed-exploration.json

A **planner** bot navigates rooms by *goal*, not by input: you declare
WHAT to achieve (TestApi predicates), the planner figures out WHEN to
emit.

- `seq` — children in order; `any` — first child that succeeds (with
  per-child timeouts); `repeat` — retry with a deadline.
- Every primitive carries an `emit` payload: the typed intent to try
  while pursuing it.
- Primitives validate at load: numeric checks (`ge`, `gt`) against
  string values are rejected, not silently unfirable.

---

## Validating scenarios

```sh
cargo bevy-swarm validate my-scenario.json
```

Runs load-time validation WITHOUT running the game: JSON parse
errors, schema conformance, and every structural check
(`validate_scenario`) — all errors reported at once, with
did-you-mean suggestions for typos:

```
error: key_presses: unknown key name 'Spaxe' — did you mean 'Space'?
```

See also: the [rule catalog](rules.md) for what each oracle detects,
and [the JSON schema](schema.json) for the full field reference.
