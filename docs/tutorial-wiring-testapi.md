# Tutorial: Wire Your Game's TestApi

*Learning-oriented — progressive enhancement from zero-contract to the
full semantic tier. You will publish observable state, declare input
verbs, and use both from invariants and planner goals.*

The quick-start [first playtest](tutorial-first-playtest.md) already
works with **zero** game-side contract. That's the floor. This tutorial
walks the ladder up, one piece at a time — each is optional,
independent, and detected automatically (the report's `scope.tier`
field moves from `zero` → `observable` → `semantic`).

The full tier table is in the
[contract-tiers reference](reference-contract-tiers.md).

## Step 1: Publish observable state (`TestApi`)

`TestApi` is how your game tells the harness what's worth looking at —
score, HP, game phase. It's a plain resource resolving dotted paths:

```rust,no_run
use bevy::prelude::*;

#[derive(Resource, Default)]
pub struct TestApi {
    pub score: i64,
    pub hp: f32,
    pub phase: String, // "menu" | "playing" | "game_over"
}

// Zero game-code annotation: any Resource or Component works as a
// percept path without TestApi ("Resource:hp", "Component:Health").
// TestApi centralizes the canonical surface and keeps paths stable.
```

Register it and invariants can reference `TestApi.*` paths:

```json
{
  "invariants": [
    { "name": "hp_floor", "rule": "custom",
      "path": "TestApi.hp", "check": "above", "value": 10 }
  ]
}
```

World percepts work with **zero** extra setup too: paths like
`"Resource:score"` or `"Component:Health.0"` resolve via reflection.

## Step 2: Declare your input verbs (`IntentSurface`)

By default the chaos bot samples raw keys (WASD/arrows). If your game
consumes typed intents, declare them so the bot speaks your language:

```rust,no_run
use bevy::prelude::*;

#[derive(Event, Clone)]
pub enum GameIntent {
    Move { dir: (f32, f32) },
    Choice { index: usize },
    Select { target: String },
    Wait,
}
```

With a non-empty `IntentSurface`, actuator selection switches to
intents automatically (force it with `"actuator": "intents"`).

## Step 3: Use TestApi from invariants

Every capability of the invariant DSL against a wired TestApi:

```json
{
  "bot": { "type": "chaos", "seed": 42 },
  "duration_s": 30,
  "invariants": [
    { "name": "score_never_drops", "rule": "custom",
      "path": "TestApi.score", "check": "ge", "value": 0,
      "differential": "no_decrease" },

    { "name": "reaches_gameplay", "rule": "custom",
      "path": "TestApi.phase", "check": "equals", "value": "playing",
      "eventually_s": 10 },

    { "name": "enemy_count_cap", "rule": "custom",
      "query": { "with": ["EnemyTag"] },
      "check": "le", "value": 200 }
  ]
}
```

- `differential` — per-frame comparison to the previous value
- `eventually_s` — must hold at least ONCE within the deadline
- `query` — ECS analogue of Playwright's `toHaveCount`

Validate offline before running the game:

```sh
cargo bevy-playtest validate my-scenario.json
```

Syntax + schema + semantic checks, all errors at once, with
did-you-mean suggestions for typos.

## Step 4: Write scenarios in Rust (`ScenarioBuilder`)

JSON is data-friendly; Rust is author-friendly. The
[`ScenarioBuilder`](../src/builder.rs) API compiles the structure:

```rust,no_run
use bevy_playtest::builder::ScenarioBuilder;

let scenario = ScenarioBuilder::chaos()
    .name("hp-watch")
    .duration_s(30.0)
    .custom_invariant("hp_floor", "TestApi.hp", "ge", 0)
    .count_invariant("enemy_cap", vec!["EnemyTag".into()], "le", 200)
    .check_determinism(3)
    .build();
```

The builder panics on invalid configuration at *author* time — the
typos JSON only catches at load.

## Step 5: Drive goals, not inputs (planner bot)

With TestApi predicates published, the planner bot pursues GOALS:

```json
{
  "bot": {
    "type": "planner",
    "goals": {
      "kind": "seq",
      "children": [
        { "kind": "primitive", "path": "TestApi.phase",
          "check": "equals", "value": "playing",
          "emit": [{ "intent": "choice", "index": 0 }] },
        { "kind": "repeat", "max_s": 10,
          "child": { "kind": "primitive", "path": "TestApi.score",
                     "check": "ge", "value": 10,
                     "emit": [{ "intent": "select", "target": "Chest" }] } }
      ]
    }
  }
}
```

`seq` orders, `any` races (first success wins), `repeat` retries with
a deadline. You declare WHAT to achieve; the planner emits the
intents to try while pursuing it.

## Step 6: Optional extras

Each of these is detected and upgrades your tier further:

- `ResetHooks` / `CheatHooks` — named setup and mid-run state surgery
  (cheats are AUDITED: every invocation lands in the action log)
- `Gameplay` marker component — entity-scoped oracles and stable
  identity in reports/replays
- Genre markers (`TurnBased`/`RealTime`) — tune liveness semantics

## Where to go from here

- All the example scenarios referenced here come from the
  [Scenario Cookbook](scenarios.md)
- Per-field JSON reference → [schema.json](schema.json)
- What every rule detects → [rule catalog](reference-rules.md)
- Why the pieces are optional by design →
  [zero-contract philosophy](explanation-zero-contract.md)
