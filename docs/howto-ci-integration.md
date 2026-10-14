# How-to: Run bevy_swarm in CI

*Goal-oriented — wire playtests into GitHub Actions (or GitLab/Jenkins)
so regressions fail builds but known issues don't.*

## The short version

Add a job that runs your swarm tests like any other cargo test:

```yaml
# .github/workflows/swarm.yml
name: swarm
on: [push, pull_request]
jobs:
  playtest:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - run: cargo test --test playtest
      - uses: actions/upload-artifact@v4
        if: always()          # upload reports even (especially) on failure
        with:
          name: swarm-reports
          path: target/bevy_swarm/runs/
```

Headless by design: no GPU, no window, no display server needed —
the harness strips render/window/audio backends before running.

## Gate semantics: new or regressed, not "any failure"

Raw test exit codes fail on ANY violation. That's correct for
pre-merge branches, but as a project accumulates known bugs you want
the gate to distinguish:

| Classification | Meaning | Gates CI? |
|---|---|---|
| `[NEW]` | Fingerprint never seen before | **yes** |
| `[REGRESSED]` | Seen before, was muted/resolved | **yes** |
| `[known]` | Tracked open issue | no |
| `[muted]` | Explicitly silenced (justification required) | no |

`report.gate.passed` implements exactly this. Use `autotest()` with
assertion on the gate rather than per-violation, and your main branch
can stay green while known issues are triaged:

```rust,no_run
# fn main() {
let report = /* autotest(...) */;
assert!(report.gate_passed(), "new or regressed failures:\n{}",
        report.failure_summary());
# }
```

## Tracking known issues (the issue DB)

Fingerprints are stable identifiers for a failure (rule + scenario +
context — immune to seed/length churn). The issue DB lives at
`tests/swarm/issues.json`: fingerprint → state `open|muted|resolved`.

- **CI reads, never writes**: writes happen only locally with
  `BEVY_SWARM_UPDATE=issues cargo test ...`, then you review and
  commit the diff
- Muting requires a justification string (recorded in the DB) —
  silencing without saying why is rejected

```sh
# after a red local run:
BEVY_SWARM_UPDATE=issues cargo test --test playtest
git diff tests/swarm/issues.json   # review, then commit
```

## JUnit XML for dashboards

Autotest runs emit `runs/<run-id>/junit.xml` automatically. For
multi-variant matrices:

```rust,ignore
let matrix = run_branch_matrix(build_app, vec![variant_a, variant_b])?;
std::fs::write("target/swarm-junit.xml", matrix.to_junit("swarm"))?;
```

Upload `target/swarm-junit.xml` alongside the run directory and CI
dashboards (GitHub Actions reports, GitLab test tab, Jenkins) render
per-seed results natively.

## Keeping runtime sane in CI

- `AutotestConfig::default()` (4 × 5 s) fits a normal test job
- Reserve `thorough()` (16 × 20 s) for scheduled/nightly jobs
- Upload `target/bevy_swarm/runs/` as an artifact **always** — the
  action log in a red run is the debugging starting point

## Reproducing a CI failure locally

CI failure output includes the seed. Pull the artifact, then:

```sh
cargo swarm reproduce --report <downloaded-report>/report.json
# or directly:
BEVY_SWARM_SEED=<seed> cargo test --test playtest
```

See [debugging a failing run](howto-debug-failure.md) for the full
workflow.
