# Decision 0001: agent BRP API vs Bevy's E2E-testing goal (#25894)

Status: Accepted (2026-10-07, bead swarm-cj8.17 / G1)

## Context

Bevy issue #25894 (viridia, 2026-09-23) proposes a project goal for
"Playwright/Selenium-like" automated end-to-end testing over BRP:
entity selection by name/path, presence and absence checks, scene-ready
events, timeouts, and language-agnostic test runners over HTTP + JSON.
As of 2026-10-07 the goal is **Proposed, unstaffed** — no design docs, no
working group.

bevy_swarm's `agent` feature already exposes: `playtest/schema`,
`playtest/observe`, `playtest/intent`, `playtest/key`, `playtest/pointer`,
`playtest/reset`, `playtest/cheat`, `playtest/actions`, `playtest/state`,
`playtest/screenshot`, `playtest/diagnostics`, plus a readiness gate
(`GameReady`, the UE IsReady analog), eventually-assertions
(`eventually_s`, the Playwright `toBeVisible(timeout)` analog) and an
actionability gate before pointer/key intents.

Related ecosystem: natepiano's `bevy_brp_mcp`/`bevy_brp_extras` (input
injection + screenshots for LLM agents); new BRP methods `world.inspect`
and `diagnostics` land in Bevy 0.21 (main), not 0.20 (Z17 tracks them).

## Questions and decisions

### 1. Overlap with upstream BRP methods

| Upstream (0.20) | bevy_swarm today | Verdict |
|---|---|---|
| `world.query`, `world.get`, `world.spawn`, `world.insert/remove/mutate`, `world.write_message`, `world.watch` | `playtest/observe` (path-based reads) | Overlaps. Keep `observe` — it resolves game-agnostic percept paths (`Component:Foo.bar`, `Resource:Bar.field`, `TestApi.*`) rather than requiring clients to understand entity ids. |
| Presence/absence checks (#25894) | `eventually_s` + invariants | Keep: ours are scenario-embedded, not client-polled. |
| Timeouts / waits (#25894) | **Missing server-side** | Add `wait_until` (see 3). |
| Selection by name/path (#25894) | `observe` supports `Component:Type{<Name>}.field` (I2 addressing) | Keep, upstream has nothing like it yet. |

The genuinely differentiated surface is: intents (typed verb injection
against declared `IntentSurface`s), actionability-gated pointer/key
input, audited cheats, reset hooks, and eventually-assertions with
per-check semantics. None of that is upstream.

### 2. Namespace: `playtest/*` vs `swarm/*`

**Decision: keep `playtest/*`.**

Reasons:
- Upstream's goal text contemplates a generic E2E surface likely living
  under existing namespaces (`world.*`, `app.*`) or a new `test/*`;
  `swarm/*` would collide with nothing but signals
  crate-affiliation where the methods are actually game-facing.
- `playtest/*` describes what the methods do, not who ships them.
- Renaming now costs every downstream caller (fixture-runner, X2 goldens)
  for zero behavioral gain; revisit only if upstream adopts `playtest/*`
  with different semantics (unlikely — they don't use the term).

### 3. Server-side `wait_until`

**Decision: add it as a follow-up bead (M2 shortlist derives from it),
not inside G1.**

Sketch: `playtest/wait_until { path, check, value, timeout_frames }`
reusing C1's `CheckOp` and D1's `resolve_path`. It is the core of any
Playwright-like runner; today clients must poll `observe`. Blocking a
BRP worker thread until a frame-bound condition holds requires either a
background channel checked by an exclusive system each frame, or a
poll-with-budget inside the method. Defer the mechanism to the follow-up
bead; this spike records only the requirement.

### 4. Contribute upstream?

**Decision: stay an external crate; do not comment on #25894 yet.**

Posting upstream requires the user's explicit approval (policy), and the
goal is unstaffed — commentary now buys nothing. Revisit when the goal is
staffed or when `bevy_swarm` hits a stable release. Internal rule: never
mention internal bead ids or downstream personas in any future upstream
communication; describe features generically.

### 5. MCP wrapper

**Decision: don't build one; document compatibility.**

`bevy_brp_mcp` already bridges BRP to MCP for LLM agents; any BRP method
we expose is automatically reachable through it once registered. Writing
our own wrapper duplicates that work. Documentation task (fold into C4):
add a README section noting that MCP-driven agents can reach
`playtest/*` via `bevy_brp_mcp` unchanged.

## Consequences

- No renames: R2 (loopback/token/release-warning security) proceeds
  against `playtest/*` as-is.
- `wait_until` becomes a follow-up bead, prerequisite for M2
  (LLM-assisted exploration).
- C4's docs get an ecosystem note (`bevy_brp_mcp`) and the "don't ship
  agent-enabled builds" security wording.
- Z17 keeps tracking `world.inspect`/`world.summarize`/`app.info`/
  `diagnostics.*` for the eventual 0.21 upgrade; our `playtest/diagnostics`
  may thin out in favor of upstream's.
