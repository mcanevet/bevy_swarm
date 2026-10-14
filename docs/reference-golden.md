# Reference: Golden Snapshot Testing (Z11)

Record and compare per-frame state digests to detect gameplay
regressions across versions.

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

- **Approval workflow**: Set `BEVY_SWARM_UPDATE=golden` (or `all`) to
  rewrite golden files; commit the diff for review.
  `BEVY_SWARM_UPDATE` unset means read-only.
- **Determinism required**: Run `check_determinism` first;
  nondeterministic scenarios are flagged.
- **Tolerance**: float tolerance smooths timing/platform noise;
  same-platform digest mismatches are still failures (platform is not
  compared separately).
- **Readable diffs**: If your game exposes state via `TestApi`, diffs
  show field names/values instead of raw hashes.

Golden divergences surface as the `golden_diverged` rule — see the
[rule catalog](rules.md). For the determinism substrate goldens rely
on, see the [determinism explanation](explanation-determinism.md).
