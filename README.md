# bevy_swarm

An ECS-native, headless, in-process playtesting harness for [Bevy](https://bevy.org) games. Write WHAT to test, not WHEN to press.

**Targets Bevy 0.20** (`0.20.0` on crates.io). Uses the `MessageReader`/`MessageWriter` buffered-event API.

## Conventions

Workspace layout, artifact paths, update mode (`BEVY_SWARM_UPDATE`), versioning,
rule-name registry, and vocabulary are governed by [docs/conventions.md](docs/conventions.md)
— the single source of truth. Oracle rule names live in the [rule registry](docs/rules.md)
(`bevy_swarm::rules`).

## Bevy compatibility policy

Each bevy_swarm release supports **exactly one Bevy minor version**. No
cross-version compatibility shims — the harness tracks Bevy's API closely
(observers, message buffers, diagnostics), and carrying fallbacks doubles the
surface that can silently diverge from real game behavior.

| bevy_swarm | Bevy |
|-----------|------|
| 0.1.0 (unreleased) | 0.20 (`0.20.0`) |

## Gameplay Snapshot Testing (Z11)

Record and compare per-frame state digests to detect gameplay regressions across versions:

```rust,no_run
use bevy_swarm::golden::{record_golden, check_golden, GoldenSet, Tolerance};
use bevy_swarm::scenario::Scenario;
use bevy::prelude::App;

# fn main() {
// Record a baseline (once):
let scenarios: Vec<Scenario> = vec![];
record_golden(|| App::new(), &scenarios, "tests/swarm/golden").unwrap();

// Check against goldens (in CI or tests):
let set = GoldenSet::load("tests/swarm/golden").unwrap();
let report = check_golden(|| App::new(), &set, &Tolerance::default()).unwrap();
assert!(report.all_same(), "no regressions");
# }
```

- **Approval workflow**: Set `BEVY_SWARM_UPDATE=golden` (or `all`) to rewrite golden files; commit the diff for review. `BEVY_SWARM_UPDATE` unset means read-only.
- **Determinism required**: Run `check_determinism` first; nondeterministic scenarios are flagged.
- **Tolerance**: float tolerance smooths timing/platform noise; same-platform digest mismatches are still failures (platform is not compared separately).
- **Readable diffs**: If your game exposes state via `TestApi`, diffs show field names/values instead of raw hashes.

## What it does

- **Typed intent bots** — chaos (with uniform/aggressive/curious/idle personas), replay, pursuit, planner (aplib-style goal trees), synthetic_pointer and synthetic_keyboard (drive the REAL input chain through picking/ButtonInput, not shortcuts)
- **Invariant DSL** — scenario JSON with bounds checks, TestApi predicates, eventual assertions, query-target entity counts, differential rules (no_decrease/no_increase), rate limits, expert-rule oracles (WHEN/REQUIRE)
- **Oracles** — frozen-world soft-lock detection, Welford frame-time anomaly detection, true rolling p99, readiness gate, finite-transform NaN check (all entities, not only Gameplay)
- **CA² system coverage** — which registered systems actually executed, with zero instrumentation (wrap-safe schedule tick-age snapshots)
- **Failure minimization** — ddmin over the structured action log shrinks ANY failure (crashes and invariant violations) to a ready-to-save regression scenario with a trimmed duration, and fails honestly with `NotReproducible` when the log alone cannot reproduce the failure
- **Calibration** — `diagnostics::calibrate_world` runs a chaos session and derives suggested invariants from archetype-count envelopes, with zero game annotations (library API; there is no CLI flag)
- **Loud contracts** — missing picking backends, unresolved TestApi paths, vacuous setups REJECT the scenario instead of silently passing

## Usage

```toml
[dev-dependencies]
bevy_swarm = "0.1" # or path/git
```

```rust,no_run
use bevy_swarm::harness::{run_scenario, Scenario};

# fn main() -> Result<(), Box<dyn std::error::Error>> {
let scenario: Scenario = serde_json::from_str(
    r#"{"bot":{"type":"planner","goals":{"kind":"primitive","path":"TestApi.score","check":"above","value":3,"emit":[{"intent":"choice","index":0}]}},"duration_s":1.0,"invariants":[]}"#,
)?;

let mut app = build_your_headless_app(); // your game's plugins + PlaytestPlugin
let report = run_scenario(&mut app, &scenario)?;
assert_eq!(report.status, bevy_swarm::enums::PlaytestStatus::Pass);
# Ok(())
# }
# fn build_your_headless_app() -> bevy::prelude::App { unimplemented!() }
```

## Game-side contract (deliberately thin)

1. Optional: a `TestApi` resource implementing `TestApiResolve` (or `app.register_test_api::<YourType>()` for a custom type) — publish observable state (score, phase, hp...) for invariants and planner goals; world percepts (`Resource:`/`Component:` paths) work with ZERO game-side setup
2. `IntentSurface` — declare your game's input verbs; the chaos bot samples only from it
3. Optional: `ResetHooks`, `CheatHooks`, genre markers (`TurnBased`/`RealTime`), `Gameplay` marker component

Games with none of these still work: reflection-based percepts and auto-discovery calibration kick in, and the harness fails loudly rather than passing vacuously when a contract piece it needs is missing.

## Feature flags

- `agent` — BRP-based resident agent for live inspection and intent injection

## Architecture

At its core, the data flow per frame:

```mermaid,ignore
flowchart LR
    Bots[Bots] -->|UserIntent| Game[Game Systems]
    Game --> World[(World)]
    World --> Oracles[Oracles]
    Oracles -->|Violations| Report[PlaytestReport]
```

- **Bots** (chaos, replay, pursuit, planner) sample from your declared `IntentSurface` and write typed `UserIntent` messages
- **Game systems** consume those intents directly — same schedule, same binary, no adapter layer
- **Oracles** (frozen-world, frame-time anomaly, bounds checks, invariant DSL) watch the world and report violations
- **Synthetic input** is opt-in: `synthetic_pointer`/`synthetic_keyboard` bots drive the real `PointerInput`/`KeyPressInput` chain when a scenario asks for it
- The harness itself is pre/post-processing around the Bevy app — `run_scenario()` sets up, pumps the loop, then collects the report

## Invariant DSL (primary API)

The **JSON invariant language** in scenarios is the ONE supported way
to express checks (`check`, `value`, `query`, `path`, windows
`after_s`/`before_s`/`eventually_s`). It parses into one `CheckOp`
enum everywhere: wire format `lt`, `le`, `gt`, `ge`, `equals`, `ne`
(legacy aliases `below` = `le`, `above` = `ge`, `eq` = `equals`; all
inclusive boundaries). Calibration output
(`generate_invariants_from_calibration`) emits exactly this dialect —
round-trip guaranteed by test.

`bevy_swarm::typed` (Rust-native oracle/predicate/policy systems) is
**experimental**: usable, but expect breaking changes without a major
version bump; the JSON DSL is the stable surface for v0.1.

## CI integration (JUnit XML)

Emit JUnit XML for CI test reporters (GitHub Actions, GitLab, Jenkins):

```rust,ignore
let matrix = run_branch_matrix(build_app, vec![variant_a, variant_b])?;
std::fs::write("target/swarm-junit.xml", matrix.to_junit("swarm"))?;
```

Upload `target/swarm-junit.xml` as a CI artifact; autotest runs also
write `runs/<run-id>/junit.xml` automatically.

## Testing

Two loops (see docs/conventions.md):

- **Outer loop (acceptance):** plain fixture games with ZERO bevy_swarm
  plumbing (`fixtures/*`) + JSON expectation files
  (`tests/swarm/expectations/<fixture>/<variant>.json`). The runner
  (`tests/acceptance.rs`) drives the fixture-runner binary — the same
  path a user takes — and asserts on the report JSON contract
  (`docs/report-schema.json`). Bug variants are selected at runtime via
  the `FIXTURE_BUG` env var, so clean and buggy share one build.
- **Inner loop (unit):** module tests inside `src/` for algorithms (determinism, fingerprints, oracles)
  and semantics.

To add a fixture: create `fixtures/<game>/` (depends on bevy ONLY),
add an adapter + arm in `tests/fixture-runner`, and write expectation
files under `tests/swarm/expectations/<game>/` (clean.json is the
false-positive guard; every `bug_<name>.json` documents a planted bug).

## Development

```bash
cargo test              # ~200 lib tests
cargo test --tests       # integration/regression suites
cargo test --features agent
cargo clippy --all-targets -- -D warnings
```

Requires Bevy 0.20 with the `debug` feature for real system names.

## Determinism

Replay, minimization, and seed sweeps are only trustworthy if a scenario is
deterministic. `check_determinism` verifies it (ggrs-SyncTest style): the
scenario runs multiple times on fresh `App`s and per-frame canonical state
digests are compared. On divergence you get the first divergent frame and
the differing fields (e.g. `Gameplay[Ball].translation.x`):

```rust,ignore
let report = bevy_swarm::determinism::check_determinism(build_app, &scenario, 2)?;
if !report.is_deterministic() {
    println!("{:?}", report.first_divergence);
}
```

Games contribute custom digest parts (RNG state, AI blackboards) via the
`DigestHooks` resource. Normal runs pay nothing — digests are only recorded
when a `StateTrace` resource is present.

### Z14: Deterministic Entropy Hook

Enable the `deterministic-entropy` feature and set the `getrandom_backend` cfg:

```bash
RUSTFLAGS='--cfg getrandom_backend="custom"' cargo test --features bevy_swarm/deterministic-entropy
```

(planned: a `cargo swarm run` wrapper that sets both automatically)

The custom backend hook (`__getrandom_v03_custom`) intercepts all `getrandom`
calls (including `rand::rng()`, `uuid::Uuid::new_v4`, `bevy_rand`) and draws
from a deterministic stream seeded from the scenario seed. Calls outside
`enter_run()` draw from a process-global fallback stream and increment the
`entropy_unattributed_draws` informational rule counter.

**Important**: `rand::rng()` uses `ThreadRng`, which is cached per thread.
To guarantee determinism across runs, **each scenario must run on a fresh
thread**. The matrix runner (`run_scenarios_parallel`) does this
automatically. Direct callers of `run_scenario` must either: (a) spawn a
fresh thread per call, or (b) document that they accept the risk of
cross-run ThreadRng state leakage. The `InProcess` runner now calls
`enter_run` before the factory so plugin-build draws are seeded.

**Important**: The hook signature must match exactly:
```rust,ignore
#[unsafe(no_mangle)]
pub unsafe extern "Rust" fn __getrandom_v03_custom(
    dest: *mut u8,
    len: usize,
) -> Result<(), getrandom::Error>
```

Using `extern "C"` or returning `Result<(), ()>` is undefined behavior (UB)
and will abort the process on panic.

## Robustness margins (J1)

Every invariant carries a **signed robustness ρ** — the STL-style distance to
violation: positive when satisfied ("never violated, but came within 0.3 of the
bound"), negative when violated. `report.robustness` holds:

- `per_invariant`: for each invariant, `(min ρ over run, frame of worst, normalized min)`
- `overall`: min of normalized mins across all invariants (< 0 iff the run failed)

Units normalize via an optional `scale` field (default `max(|threshold|, 1.0)`;
bounds use the half-width) so pixel-scale bounds don't swamp HP-scale ones.

**Semantics by invariant kind:**

| Invariant | Robustness ρ |
|-----------|-------------|
| `below`/`le` thr | `thr - v` |
| `above`/`ge` thr | `v - thr` |
| `lt`/`gt` | same as `le`/`ge` (strictness matters at ρ = …) |
| `equals` thr | `-(distance)` (0 when exact) |
| text `equals` | `+1` if equal, else `-1` (boolean lift) |
| `bounds` (per entity) | `min over axes of min(v - lo, hi - v)`; aggregated `min` over entities |
| `count` query | threshold margin on the count |
| `no_decrease` | `v_t - v_{t-1}` (min over run) |
| `max_delta_per_sec` | `max_dps - rate` |
| `eventually` (deadline D) | `max over t≤D of ρ_inner(t)` (best value so far) |
| expert `WHEN→REQUIRE` | `max(-ρ_W, ρ_R)` (anti-masking) |
| always-mode (default) | run-level ρ = `min over t of ρ(t)` |

Boolean verdict is exactly `ρ < 0` (or `ρ ≤ 0` for strict ops); this equivalence
is enforced by `debug_assert!` in the oracle so semantics can't drift.

```json
{"invariants": [{"name": "hp_floor", "rule": "custom", "path": "TestApi.hp",
                 "check": "above", "value": 10, "scale": 100}]}
```

This continuous signal is the substrate for future guided search (choice
mutation, Go-Explore) and LLM falsifiers.

## Security (agent feature)

The `agent` feature exposes BRP (arbitrary world reads and **mutation**) over
HTTP. Threat model: **the agent is an open debugger gated by a shared secret,
safe on loopback only**. Remote use additionally requires a TLS-terminating
proxy — the HTTP transport is plaintext.

- Binds to `127.0.0.1:15702` by default; non-loopback addresses are refused
  unless `AgentConfig { allow_remote: true, .. }` is set explicitly.
- **Default-deny**: a token is ALWAYS required for `playtest/*` methods.
  Configure one via `AgentConfig::token` or the `BEVY_SWARM_AGENT_TOKEN`
  env var. When neither is set, a cryptographically random session token
  is generated and printed to stderr at startup (eprintln, so it survives
  even when the plugin is added before `LogPlugin`).
- Token comparison is constant-time; an explicitly-empty configured token
  is treated as unset and does not authenticate `{token: ""}`.
- Release builds with the agent active log a loud warning; set
  `deny_in_release: true` to panic instead. Never ship agent-enabled builds.

**Experimental features** (known limitations, see docs):
- `autotest()` — false positives on default intent keys and static games (RF8)
- raw input (gamepad/pointer injection) — gamepad actions not in the audit log (RF16)
- `ClickEntity`/pointer picking — `NoViewportSize` failures without a camera (RF11)
- deterministic entropy — `rand::rng()` not reproducible on the same thread (RF10)
