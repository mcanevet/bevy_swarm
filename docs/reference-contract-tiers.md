# Reference: Contract Tiers

The harness detects how deep your game's test contract goes and
states it in every report (`report.scope.tier`), along with the
oracle families that ran vs. could not run (`scope.active` /
`scope.inactive`, each inactive entry carrying a reason and a
remediation pointer) and how the harness drove the game
(`scope.actuators`: `intents`, `raw_input`, `ui_monkey`).

| Tier | Requires | What you get |
|------|----------|--------------|
| `zero` | nothing — the game's plugin alone | raw-input chaos (arrows/WASD default surface), inferred gameplay entities, generic oracles (panic, liveness, NaN, bounds, frame-time) |
| `observable` | `Name` on gameplay entities, or `GameTypes` registered, or reflect-registered components | stable entity identity in reports/replays, per-entity percept paths, effect attribution |
| `semantic` | any of: `TestApi`/`TestApiResolver`, non-empty `IntentSurface`, typed oracles, `ResetHooks`/`CheatHooks`/`NamedIntents` | intent bots, typed oracles, planner goals, reset/cheat hooks, `TestApi.*` invariants |

## Detection semantics

- Tier detection is **automatic** — the harness probes the world at
  scenario-load time; you never declare a tier.
- Detection is **monotone and partial**: having a `TestApi` alone
  unlocks `TestApi.*` invariants but not intent bots (those need the
  `IntentSurface`).
- **A missing contract piece is NEVER a rejection reason by itself.**
  Scenarios that don't reference the missing pieces run fine; the
  report states what was not checked and why.

## Contract piece reference (optional, in increasing depth)

1. **`TestApi`** resource implementing `TestApiResolve` — publish
   observable state (score, phase, hp) for invariants and planner
   goals; world percepts (`Resource:`/`Component:` paths) work with
   zero game-side setup
2. **`IntentSurface`** — declare the game's input verbs; the chaos
   bot samples only from it
3. **`ResetHooks` / `CheatHooks`** — named setup and audited
   mid-run state surgery
4. **Genre markers** (`TurnBased`/`RealTime`), **`Gameplay`** marker
   component — entity scoping, stable identity

Games with none of these still work: reflection-based percepts and
auto-discovery calibration kick in, and the harness fails loudly
rather than passing vacuously when a contract piece it needs is
missing.

## Actuator selection

Automatic: intents when an `IntentSurface` exists, raw input
otherwise. Force per-scenario with
`"actuator": "intents" | "raw" | "ui" | "auto"`.

## Definitional cautions

- **zero-contract** = no semantic hooks required. NOT a claim about
  typing effort: enabling a cargo feature (`bevy_remote`) is a thin
  build-time contract; "zero hand-written lines" (CLI scaffolding)
  is convenience, not a guarantee.
- **zero game-code changes at runtime** — true for in-process mode,
  but requires a `lib.rs` target.

See the [zero-contract explanation](explanation-zero-contract.md)
for the reasoning behind each of these rules.
