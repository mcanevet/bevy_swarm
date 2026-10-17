# bevy_playtest Documentation

Documentation follows the [Diataxis](https://diataxis.fr) framework:
four quadrants by purpose.

## Tutorials — learning-oriented (start here)

- [Your first playtest](tutorial-first-playtest.md) — zero-contract
  chaos testing in 15 minutes, and how to read failure output
- [Wire your game's TestApi](tutorial-wiring-testapi.md) — the ladder
  from zero contract to planner bots and typed invariants

## How-to guides — task-oriented

- [Debug a failing run](howto-debug-failure.md) — inline summaries,
  seed pinning, reproduction, minimization
- [Add an invariant](howto-add-invariant.md) — the invariant DSL:
  thresholds, deadlines, bounds, counts, differentials, expert rules
- [Run in CI](howto-ci-integration.md) — gate semantics (new vs
  known), JUnit XML, artifact uploads
- [Suppress a known-benign failure](howto-suppress-known-failure.md)
  — fingerprints, the issue DB, scoped alternatives
- [Smoke-test a game you don't own](howto-smoke-external-game.md) —
  the external BRP tier in 30 seconds

## Reference — information-oriented

- [Rule catalog](reference-rules.md) → [rules.md](rules.md) —
  every rule: detection semantics, false-positive modes (generated)
- [Contract tiers](reference-contract-tiers.md) — detection matrix,
  unlock table, definitional cautions
- [Scenario schema](schema.json) — the scenario JSON contract
  (generated)
- [Report schema](report-schema.json) — the output contract
- [Scenario Cookbook](scenarios.md) — six annotated example
  scenarios
- [Golden snapshots](reference-golden.md) — Z11 snapshot testing
- [Robustness margins](reference-robustness.md) — J1 ρ semantics
- [Invariant DSL](reference-invariant-dsl.md) — canonical wire forms
- [Security (agent)](reference-security.md) — threat model, token
- [Usage/API](reference-usage.md) — single-scenario API, env vars,
  features
- API reference: [docs.rs/bevy_playtest](https://docs.rs/bevy_playtest)

## Explanation — understanding-oriented

- [The zero-contract philosophy](explanation-zero-contract.md) —
  why invariants beat scripted tests, why the tiers are optional
- [Coverage: what it catches](explanation-coverage.md) — failure
  classes, panic/log capture, limitations
- [Determinism strategy](explanation-determinism.md) — why replay
  needs determinism, and how the entropy hook provides it
- [Architecture](explanation-architecture.md) — how the harness
  executes inside your Bevy app, code map
- [Agent vs Bevy E2E](explanation-agent-vs-bevy-e2e.md) — design
  decision: the `agent` feature alongside Bevy's own testing goal

## Project conventions

Workspace layout, artifact paths, update modes, vocabulary —
[reference-workspace.md](reference-workspace.md) (single source of truth).
