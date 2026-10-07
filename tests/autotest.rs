//! Z18: autotest() integration — config defaults, env overrides, report.

use bevy::prelude::*;
use bevy_swarm::autotest::{autotest, AutotestConfig};

#[derive(Clone, Default)]
struct SimpleGamePlugin;

impl Plugin for SimpleGamePlugin {
    fn build(&self, _: &mut App) {}
}

#[test]
fn autotest_runs_and_writes_report() {
    // Small config to keep the test fast.
    let config = AutotestConfig {
        num_seeds: 2,
        duration_s: 0.05,
        parallel: 1,
        minimize_failures: false,
        max_failures: None,
        ..AutotestConfig::default()
    };
    let report = autotest(SimpleGamePlugin, config).expect("autotest runs");
    assert_eq!(report.schema_version, "0.1.0");
    assert_eq!(report.harness_mode, "autotest");
    assert_eq!(report.contract_tier, "tier-0");
    assert_eq!(report.scope.seeds_run, 2);
    assert_eq!(report.runs.len(), 2);
    // Clean game: no failures.
    assert!(report.failures.is_empty(), "{:?}", report.failures);
    let summary = report.summary();
    assert!(summary.contains("2 seeds"), "summary: {summary}");
    // Report file was written under target/.
    let target = std::env::var("CARGO_TARGET_DIR").unwrap_or_else(|_| "target".into());
    let runs_dir = std::path::Path::new(&target).join("bevy_swarm/runs");
    assert!(
        std::fs::read_dir(&runs_dir)
            .map(|entries| entries.filter_map(|e| e.ok()).any(|e| e.file_name().to_string_lossy().starts_with("autotest-")))
            .unwrap_or(false),
        "expected an autotest-* run dir under {}",
        runs_dir.display()
    );
}
