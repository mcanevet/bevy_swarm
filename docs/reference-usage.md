# Reference: Usage (single scenario)

Programmatic, single-scenario execution (what `autotest` builds on):

```rust,no_run
use bevy_playtest::harness::{run_scenario, Scenario};

# fn main() -> Result<(), Box<dyn std::error::Error>> {
let scenario: Scenario = serde_json::from_str(
    r#"{"bot":{"type":"planner","goals":{"kind":"primitive","path":"TestApi.score","check":"above","value":3,"emit":[{"intent":"choice","index":0}]}},"duration_s":1.0,"invariants":[]}"#,
)?;

let mut app = build_your_headless_app(); // your game's plugins + PlaytestPlugin
let report = run_scenario(&mut app, &scenario)?;
assert_eq!(report.status, bevy_playtest::enums::PlaytestStatus::Pass);
# Ok(())
# }
# fn build_your_headless_app() -> bevy::prelude::App { unimplemented!() }
```

## Environment variables

| Variable | Purpose |
|---|---|
| `BEVY_PLAYTEST_UPDATE` | categories allowed to rewrite goldens/expectations |
| `BEVY_PLAYTEST_SEEDS` | override seed sweep range |
| `BEVY_PLAYTEST_SEED` | pin ONE exact seed for reproduction (wins over SEEDS) |
| `BEVY_PLAYTEST_DURATION` | override default scenario duration |
| `BEVY_PLAYTEST_THREADS` | parallel worker count for matrix runs |
| `BEVY_PLAYTEST_AGENT_TOKEN` | auth token for the agent feature |
| `SWARM_TRANSFORM_DESYNC_TOL` | tolerance for transform desync detection |

## Feature flags

- `agent` — BRP-based resident agent for live inspection and intent
  injection (see [security](reference-security.md))

- `deterministic-entropy` — intercept OS entropy for deterministic
  replay (see
  [determinism](explanation-determinism.md))

## Integration modes

In-process mode is the primary integration. The external/subprocess
tiers are on the roadmap (see
[reference-workspace §0](reference-workspace.md) for the full modes
table); on current Bevy 0.20 the external observe-only BRP tier
works with `--features bevy/bevy_remote` on the game side — see
[smoke-testing a third-party game](howto-smoke-external-game.md).
