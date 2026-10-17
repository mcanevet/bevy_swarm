# How-to: Suppress a Known-Benign Failure

*Goal-oriented — the harness is telling you something you already know
about, and you want it out of your CI signal without losing the
audit trail.*

Three mechanisms, narrowest first. Prefer the narrowest.

## 1. Mute the fingerprint (issue DB)

A **fingerprint** stably identifies a failure (rule + scenario +
context — stable across seeds and run lengths). Muting is per-failure,
per-scenario, reversible, and auditable:

```sh
# after a red local run:
BEVY_PLAYTEST_UPDATE=issues cargo test --test playtest
git diff tests/swarm/issues.json   # review before committing
```

The record captures state `muted` and REQUIRES a justification string —
an unjustified mute is rejected at write time. Muted fingerprints:

- still appear in reports (marked `[muted]`)
- never gate CI
- surface as `[REGRESSED]` if the underlying rule fires differently
  than when muted

```rust,no_run
# fn main() {
let report = /* autotest(...) */;
report.issue_counts().muted; // how many were silenced this run
# }
```

## 2. Narrow the scenario (fix the false-positive trigger)

Many suppressions are really scenario bugs:

- **`dead_verb` on a menu key during gameplay** — constrain the
  bot's verb surface (`IntentSurface` or the raw key list)
- **`frozen_world` on an attract screen** — start checking after
  `after_s`, or scope `duration_s` to the interactive phase
- **`fps_floor` below CI hardware reality** — raise the floor or
  drop the rule (it's informational, never gating)

The [rule catalog](reference-rules.md) documents each rule's
false-positive modes with the matching remedy.

## 3. Escalate instead (when the "noise" is real)

The inverse operation exists too: `bevy_warning` and `log_warn` are
informational by default, but if your game should NEVER warn:

```json
{ "strict_warnings": true, "...": "..." }
```

promotes them to gating for that scenario. The demotion is deliberate
(Bevy games emit many benign warnings); the escalation restores
strictness where you want it.

## What NOT to do

- Don't blanket-ignore rule names — the narrowest scope wins
- Don't mute to make a flaky test green — flaky means nondeterministic,
  fix that first (see [determinism](explanation-determinism.md))
- Don't edit reports or expectations by hand —
  `BEVY_PLAYTEST_UPDATE` modes with committed diffs are the audit path
