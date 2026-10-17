# Agent Instructions

This project uses **bd** (beads) for issue tracking. Run `bd prime` for full workflow context.

> **Architecture in one line:** Issues live in a local Dolt database
> (`.beads/dolt/`); cross-machine sync uses `bd dolt push/pull` (a
> git-compatible protocol), stored under `refs/dolt/data` on your git
> remote — separate from `refs/heads/*` where your code lives.
> `.beads/issues.jsonl` is a passive export, not the wire protocol.
>
> See [sync-concepts](https://github.com/gastownhall/beads/blob/main/docs/core-concepts/sync-concepts.md)
> for the one-screen overview and anti-patterns (don't treat JSONL as the
> source of truth; don't `bd import` during normal operation; don't
> reach for third-party Dolt hosting before trying the default).

## Quick Reference (curated — see `bd prime` for the full set)

```bash
bd ready              # Find available work
bd show <id>          # View issue details
bd update <id> --claim  # Claim work atomically
bd close <id>         # Complete work
bd dolt push          # Push beads data to remote
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


## Build & Test Commands

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

## Project-Specific Guidelines

- **Zero-contract fixtures:** Games under `fixtures/*` have NO bevy_playtest
  dependency — they are pure Bevy apps exercising the harness.
- **Acceptance tests:** Always run `cargo build -p fixture-runner` before
  `cargo test --test acceptance` (stale binary causes misleading failures).
- **Post-review process:** Follow the workflow from swarm-230: implement
  per bead acceptance criteria, add tests, verify locally, commit, PR,
  watch CI green, merge, close bead, push ledger.
- **Autonomous implementation:** This repo opts into "team-maintainer"
  mode for the overnight implementer — you MAY commit, push, open PRs,
  merge green PRs, and close beads without stopping for approval, subject
  to the "Release Order and Dependencies" rules above (dependency gates,
  close-reason completeness, RV-bead protection).
- **Never tag/publish/release:** Tagging, crates.io publishing, and
  GitHub releases are USER-ONLY actions. Never perform them.

## Non-Interactive Shell Commands

**ALWAYS use non-interactive flags** with file operations to avoid hanging on confirmation prompts.

Shell commands like `cp`, `mv`, and `rm` may be aliased to include `-i` (interactive) mode on some systems, causing the agent to hang indefinitely waiting for y/n input.

**Use these forms instead:**
```bash
# Force overwrite without prompting
cp -f source dest           # NOT: cp source dest
mv -f source dest           # NOT: mv source dest
rm -f file                  # NOT: rm file

# For recursive operations
rm -rf directory            # NOT: rm -r directory
cp -rf source dest          # NOT: cp -r source dest
```

**Other commands that may prompt:**
- `scp` - use `-o BatchMode=yes` for non-interactive
- `ssh` - use `-o BatchMode=yes` to fail instead of prompting
- `apt-get` - use `-y` flag
- `brew` - use `HOMEBREW_NO_AUTO_UPDATE=1` env var

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
