# Reference: Workspace Conventions

This document is the single source of truth for workspace layout, artifact paths,
update mode, versioning, rule names, report schema governance, and vocabulary.
Every bead references these conventions instead of inventing new paths or formats.

---

## 0. Vocabulary: Zero-Contract, Tiers, and Integration Modes

**Zero-contract** = no semantic hooks required from the game. The harness works
with unmodified game code by inferring what it can (entities via Name/Reflection,
inputs via raw injection, oracles via generic checks). When the game lacks
declared helpers (TestApi, IntentSurface, typed oracles), the report states
`contract_tier: "zero"` and lists inactive oracle families with reasons.

**Contract tiers** (runtime detection, Z1):
- `zero` — just the game's plugin; raw input, inferred entities, generic oracles only
- `observable` — + identity/observability helpers (Name on entities, Reflect, seeded RNG)
- `semantic` — + typed contract (TestApi resolver, IntentSurface, typed oracles, cheat/reset hooks)

**Integration modes** (how the harness attaches):
- **In-process** — a test file imports the game's root plugin and calls `autotest(GamePlugin)`.
  Requires the game to have a `lib.rs` target exposing the plugin. No game-code changes
  at runtime; the harness runs as part of `cargo test`.
- **Subprocess (smoke)** — the game binary is launched fresh each run via
  `bevy_ci_testing` (enabled with `cargo run --features bevy/bevy_ci_testing`).
  No repo changes, no lib.rs, no test file. Coarse frame control via CI-testing
  config events (AppExit, screenshots); observe-only oracles.
- **External (observe-only, BRP)** — drive the game binary over BRP
  (`bevy/get`/`bevy/query` polling), enabled with
  `cargo run --features bevy/bevy_remote`. Zero repo changes, zero test files;
  one documented build flag. Observe-only: crash/error/log/liveness/state-digest
  oracles. Works on Bevy 0.20.0 stable.
- **External (frame-exact, BRP)** — full remote control via BRP stepping verbs
  and input injection. Zero repo changes. Requires the Bevy release after 0.20
  (stepping verbs exist on Bevy main, unreleased). Until then this mode is
  unavailable.

**Note:** There is no `BrpPool` API in Bevy; the integration surface is
`RemotePlugin`/`BrpClient` and the BRP HTTP verbs.

**Important:** "Zero hand-written lines" (CLI scaffolding) is convenience, not a
contract claim. Whether a human or `cargo swarm init` writes the test file, a new
file enters the repo either way. The zero-contract guarantee is about *semantic*
requirements (no TestApi, no IntentSurface), not about typing effort. Enabling
a cargo feature at build time (e.g. `bevy_remote`) is a *thin build-time
contract* — much lighter than a semantic contract, but must not be advertised
as "zero contract".

---

## 1. Cargo Workspace

**Members:**
- `.` — the `bevy_swarm` library crate (publishable)
- `fixtures/*` — plain fixture games for black-box testing (zero-contract: no bevy_swarm dependency)
- `bevy_swarm_cli/` — optional CLI binary for scaffolding (`cargo swarm init`); convenience only, not part of the zero-contract guarantee

**Shared dependencies** (single place to pin, per Z0):

```toml
[workspace.dependencies]
bevy = { version = "0.20.0", default-features = false }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
```

**Publish policy:**
- `bevy_swarm` — publishable
- All other members — `publish = false`

## 2. Output Artifacts (Never Committed)

All runtime outputs live under `target/bevy_swarm/`:

```
target/bevy_swarm/
├── runs/
│   ├── <run-id>/
│   │   ├── report.json          # Main report (schema_version field)
│   │   ├── junit.xml            # Optional JUnit export
│   │   └── bundle/              # Optional artifact bundle (T3)
│   └── last                     # Copy or symlink to the latest run
├── failures/
│   └── <fingerprint>/           # T3 failure isolation
├── site/                        # T5 HTML report site
├── bisect/                      # T2 bisect worktrees
└── coverage-index.json          # T7 coverage index
```

**Run ID format:** `<UTC_yyyymmddThhmmssZ>-<16hex>` — 16 hex chars mix a
process-local counter, nanoseconds and the PID (`conventions.rs`). It is
UNIQUE per run, not a content hash of scenario+seed.
(helper: `bevy_swarm::conventions::run_id`).

**Build directory for special builds** (Z14 entropy cfg, R4): `target/bevy_swarm-build/`
(separate from the outputs; respects `CARGO_TARGET_DIR`).

## 3. Persisted, Reviewable Files (Committed by Humans)

All persisted files live under `tests/swarm/`:

```
tests/swarm/
├── config.json                  # Budgets, allow-lists, fingerprint rules, defaults
├── issues.json                  # T13 known issues
├── expectations/               # X2 acceptance expectations (per fixture)
├── pending.json                 # xfails awaiting oracle rules
├── model.json                   # S2/S3 semantic model
├── corpus/                      # J2 fuzzing corpus
└── perf-baseline.json           # T5 performance baselines
```

**Format:** JSON everywhere (`serde_json` is already a dependency; no RON/TOML deps).
**History stores** (T4 run history) are CI artifacts, never committed by bots.

## 4. Schema Versioning

Every persisted file and every report has a top-level `schema_version: <int>`.

- Readers reject newer versions with a clear error
  (`bevy_swarm::conventions::SCHEMA_VERSION`).
- `schema_version` stays **1** until the v0.2 release (RV v0.2 freezes it).
- After that, breaking changes bump it.
- Sub-reports (determinism, golden, sweep, run summary, mutation) are **embedded**
  in the report JSON under named keys, not separate files.
- Report schema: [report-schema.json](report-schema.json) (owned by X1's schema test).
- Scenario schema: [schema.json](schema.json) — GENERATED from `Scenario` via
  schemars; regenerate with
  `BEVY_SWARM_UPDATE=schema cargo test --test schema_doc` (drift-guarded).
- Rule catalog: [rules.md](rules.md) — GENERATED from the `Rule` enum; regenerate
  with `BEVY_SWARM_UPDATE=schema cargo test --test docs_generation`.

## 5. Update Mode

```bash
BEVY_SWARM_UPDATE=golden,issues,model,perf,regressions,schema  # or 'all'
```

Environment variables read by the harness (full table):

| Variable | Purpose |
|---|---|
| `BEVY_SWARM_UPDATE` | categories allowed to rewrite goldens/expectations |
| `BEVY_SWARM_SEEDS` | override seed sweep range |
| `BEVY_SWARM_SEED` | pin ONE exact seed for reproduction (wins over SEEDS) |
| `BEVY_SWARM_DURATION` | override default scenario duration |
| `BEVY_SWARM_THREADS` | parallel worker count for matrix runs |
| `BEVY_SWARM_AGENT_TOKEN` | auth token for the agent feature |
| `SWARM_TRANSFORM_DESYNC_TOL` | tolerance for transform desync detection |

- Without `BEVY_SWARM_UPDATE`, nothing under `tests/swarm/` or `docs/` is written by
  CHECK paths; `record_golden` is an explicit library call that always writes.
- CI never commits changes (external side effects need user approval).
- Insta-style semantics: only the listed categories update.
- Helpers: `bevy_swarm::conventions::{parse_update_mode, can_update}`.

## 6. Rule-Name Registry

- Registry doc: [rules.md](rules.md)
- Code constants: `bevy_swarm::rules`

Every oracle emits a registry constant; X2 expectations reference only registry
names. `crash` is a report **status**, not a rule. Rules previously described only
in prose are now explicit: `metamorphic_tick_rate`, `aggregate_conservation`,
`nondeterministic`, `golden_diverged`, `dead_widget`, `dead_verb`.

## 7. Vocabulary

- **contract_tier** = Z1's `ContractTier { Zero, Observable, Semantic }` (contract depth only)
- **harness_mode** = `InProcess | Subprocess | ExternalSmoke | ExternalDriven`
- **scaffold** (X1's expectation field) = `"none" | "adapter" | "semantic"` — not "tier"
- **severity** = T12's enum `Blocker | Critical | Major | Minor | Info`.
  Before T12 lands, reports carry `base_severity` (optional) only.

## 8. Determinism-Relevant vs Informational Rules

Wall-clock frame-time rules (`frame_time_anomaly`, `frame_time_p99_below`,
`fps_floor`, `perf_*`) are **informational**:

- Excluded from A4 digests, B2 failure signatures, T1 fingerprint gating,
  and parallel-sweep pass/fail
- Off by default in fixtures
- T5 owns perf budgets

## 9. Persona Default

`Persona::Uniform` (C3) is the default chaos persona, so seeds stay
reproducible as curious (F1) evolves.
