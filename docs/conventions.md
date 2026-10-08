# bevy_swarm Conventions

This document is the single source of truth for workspace layout, artifact paths,
update mode, versioning, rule names, report schema governance, and vocabulary.
Every bead references these conventions instead of inventing new paths or formats.

---

## 1. Cargo Workspace

**Members:**
- `.` — the `bevy_swarm` library crate (publishable)
- `bevy_swarm_cli/` — CLI tool (created by Z10; `publish = false`)
- `fixtures/*` — plain fixture games for black-box testing (created by X1/X2)
- `tests/fixture-runner` — test harness for fixtures (created by X1)
- `bench/*` — benchmark suites (created by R3/R4)

**Shared dependencies** (single place to pin, per Z0):

```toml
[workspace.dependencies]
bevy = { version = "0.20.0-rc.2", default-features = false }
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

**Run ID format:** `<UTC_yyyymmddThhmmss>-<8hex-of-scenario+seed-hash>`
(helper: `bevy_swarm::conventions::run_id`).

**Build directory for special builds** (Z14 entropy cfg, R4): `target/bevy_swarm-build/`
(separate from the outputs; respects `CARGO_TARGET_DIR`).

## 3. Persisted, Reviewable Files (Committed by Humans)

All persisted files live under `tests/swarm/`:

```
tests/swarm/
├── config.json                  # Budgets, allow-lists, fingerprint rules, defaults
├── issues.json                  # T13 known issues
├── golden/                      # Z11 golden expectations
├── regressions/                 # E3 regression scenarios
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
- Schema definition: `docs/report-schema.json` (owned by X1's schema test).

## 5. Update Mode

Single environment variable:

```bash
BEVY_SWARM_UPDATE=golden,issues,model,perf,regressions  # or 'all'
```

- Without this env var, **nothing** under `tests/swarm/` is ever written.
- CI never commits changes (external side effects need user approval).
- Insta-style semantics: only the listed categories update.
- Helpers: `bevy_swarm::conventions::{parse_update_mode, can_update}`.

## 6. Rule-Name Registry

- Registry doc: `docs/rules.md`
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
