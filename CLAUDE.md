# Project Instructions for AI Agents

This file provides instructions and context for AI coding agents working on this project.

<!-- BEGIN BEADS INTEGRATION v:1 profile:minimal hash:46cd31e7 -->
## Beads Issue Tracker

This project uses **bd (beads)** for issue tracking. Run `bd prime` to see full workflow context and commands.

### Quick Reference

```bash
bd ready              # Find available work
bd show <id>          # View issue details
bd update <id> --claim  # Claim work
bd close <id>         # Complete work
```


## Release Order and Dependencies

- **Release order is mandatory**: v0.N work starts only after RV v0.(N-1) is closed.
  - v0.1: RV swarm-le3.1
  - v0.2: RV swarm-2yc.1
  - v0.3: RV swarm-1w8.8
  - v0.4: RV swarm-1w8.9
  - v0.5: RV swarm-1w8.10
  - v0.6: RV swarm-1w8.11
- **Never remove a dependency on a release-review (RV) bead**.
- **Never close a bead whose dependencies are open** (no --force). If blocked, take other work from `bd ready`.
- **New beads must get a release:vX label** (+ gating) or non-gating, and depend on the previous release's RV bead.
- **Close reasons list every item as DONE** (commit/test) or NOT DONE (+ follow-up bead id).
- **Never publish, push tags, create releases, change licensing, or post externally without the user's explicit approval** (swarm-le3.15 is the user's publish task).


### Rules

- Use `bd` for ALL task tracking — do NOT use TodoWrite, TaskCreate, or markdown TODO lists
- Run `bd prime` for detailed command reference and session close protocol
- Use `bd remember` for persistent knowledge — do NOT use MEMORY.md files

**Architecture in one line:** issues live in a local Dolt DB; sync uses `refs/dolt/data` on your git remote; `.beads/issues.jsonl` is a passive export. See https://github.com/gastownhall/beads/blob/main/docs/core-concepts/sync-concepts.md for details and anti-patterns.

## Agent Context Profiles

The managed Beads block is task-tracking guidance, not permission to override repository, user, or orchestrator instructions.

- **Conservative (default)**: Use `bd` for task tracking. Do not run git commits, git pushes, or Dolt remote sync unless explicitly asked. At handoff, report changed files, validation, and suggested next commands.
- **Minimal**: Keep tool instruction files as pointers to `bd prime`; use the same conservative git policy unless active instructions say otherwise.
- **Team-maintainer**: Only when the repository explicitly opts in, agents may close beads, run quality gates, commit, and push as part of session close. A current "do not commit" or "do not push" instruction still wins.

## Session Completion

This protocol applies when ending a Beads implementation workflow. It is subordinate to explicit user, repository, and orchestrator instructions.

1. **File issues for remaining work** - Create beads for anything that needs follow-up
2. **Run quality gates** (if code changed) - Tests, linters, builds
3. **Update issue status** - Close finished work, update in-progress items
4. **Handle git/sync by active profile**:
   ```bash
   # Conservative/minimal/default: report status and proposed commands; wait for approval.
   git status

   # Team-maintainer opt-in only, unless current instructions forbid it:
   git pull --rebase
   bd dolt push
   git push
   git status
   ```
5. **Hand off** - Summarize changes, validation, issue status, and any blocked sync/commit/push step

**Critical rules:**
- Explicit user or orchestrator instructions override this Beads block.
- Do not commit or push without clear authority from the active profile or the current user request.
- If a required sync or push is blocked, stop and report the exact command and error.
<!-- END BEADS INTEGRATION -->


## Build & Test

```bash
cargo build -p fixture-runner  # REQUIRED before acceptance tests
cargo test -p bevy_playtest --lib # Unit tests
cargo test -p bevy_playtest --test fx0_gate # Gate integrity tests
cargo test --workspace --all-features # Full battery (end of bead)
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --check
```

**TMPDIR note:** If you encounter temp-directory permission issues, set
`export TMPDIR=/var/folders/zh/4b27878j7wz6dq22rh05qnz80000gn/T/opencode`.

## Architecture Overview

`bevy_playtest` is an in-process playtest harness for Bevy games: typed
intent bots (chaos/replay/planner/pursuit) drive the game through
`UserIntent` messages; oracle systems evaluate invariants each frame;
the driver (`src/driver.rs`) pumps the schedule and collects a
`PlaytestReport` with violations, fingerprints and metrics. Golden
snapshots, determinism checks, seed sweeps and JUnit XML output are
built on top. `fixtures/*` are zero-contract pure-Bevy games used by the
acceptance suite (`tests/acceptance.rs`, run via `fixture-runner`).

## Conventions & Patterns

- **Zero-contract fixtures:** `fixtures/*` have NO bevy_playtest dependency.
- **Acceptance tests:** run `cargo build -p fixture-runner` first (stale
  binary = misleading failures).
- **Post-review process:** implement per bead, test, PR, green CI,
  merge, close bead, `bd dolt push`.
- **Autonomous implementation:** team-maintainer opt-in for the
  overnight implementer (commit/PR/merge/close OK).
- **Never tag/publish/release:** tagging, crates.io publishing, GitHub
  releases are USER-ONLY actions.
