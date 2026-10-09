//! Z18: autotest() integration — config defaults, env overrides, report.

use bevy::prelude::*;
use bevy_swarm::autotest::{autotest, AutotestConfig};
use serial_test::serial;

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
            .map(|entries| entries
                .filter_map(|e| e.ok())
                .any(|e| e.file_name().to_string_lossy().starts_with("autotest-")))
            .unwrap_or(false),
        "expected an autotest-* run dir under {}",
        runs_dir.display()
    );
}

/// FX4.1: autotest adds PlaytestPlugin — bots/oracles run, frames > 0.
#[test]
fn autotest_adds_playtest_plugin_frames_gt_zero() {
    let config = AutotestConfig {
        num_seeds: 2,
        duration_s: 0.1,
        parallel: 1,
        minimize_failures: false,
        max_failures: None,
        ..AutotestConfig::default()
    };
    let report = autotest(SimpleGamePlugin, config).expect("autotest runs");
    // With PlaytestPlugin, oracles/bots run and accumulate frames.
    assert!(
        report.runs.iter().any(|r| r.frame_count > 0),
        "frames must be > 0: {:?}",
        report.runs
    );
}

/// FX4.5: run IDs unique + last pointer written.
#[test]
fn autotest_unique_run_ids_and_last_pointer() {
    // Use a unique output dir to isolate from other tests.
    let unique = format!("bevy_swarm/runs-fx4-{}", std::process::id());
    let target = std::env::var("CARGO_TARGET_DIR").unwrap_or_else(|_| "target".into());
    let runs_dir = std::path::Path::new(&target).join(&unique);
    let _ = std::fs::remove_dir_all(&runs_dir); // clean slate

    let config = AutotestConfig {
        num_seeds: 1,
        duration_s: 0.05,
        parallel: 1,
        minimize_failures: false,
        max_failures: None,
        output_dir: unique.into(),
        ..AutotestConfig::default()
    };
    autotest(SimpleGamePlugin, config.clone()).expect("first autotest");
    let mut first_runs: Vec<_> = std::fs::read_dir(&runs_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.starts_with("autotest-"))
        .collect::<Vec<_>>();
    first_runs.sort();
    assert_eq!(first_runs.len(), 1, "exactly one run dir");

    // Second autotest — different run_id, last pointer updated.
    std::thread::sleep(std::time::Duration::from_millis(10));
    autotest(SimpleGamePlugin, config).expect("second autotest");
    let mut second_runs: Vec<_> = std::fs::read_dir(&runs_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.starts_with("autotest-"))
        .collect::<Vec<_>>();
    second_runs.sort();
    assert_eq!(second_runs.len(), 2, "two distinct run dirs");
    assert_ne!(second_runs[0], second_runs[1], "run IDs differ");

    // Last pointer exists and points to the second run.
    let last_pointer = runs_dir.join("last");
    assert!(last_pointer.exists(), "last pointer exists");
    let last_content = std::fs::read_to_string(&last_pointer).unwrap();
    assert!(
        second_runs[1].contains(&last_content),
        "last points to second run"
    );
}

/// FX4.4: default raw surface non-empty.
#[test]
fn default_raw_surface_nonempty() {
    use bevy_swarm::scenario::RawSurface;
    let surface = RawSurface::default();
    assert!(!surface.keys.is_empty(), "default keys must be non-empty");
    // Movement keys only (arrows + WASD): tier-0 games frequently
    // ignore mouse/gamepad, and the dead_verb oracle would flag every
    // unhandled button — mouse/gamepad are opt-in per scenario.
    let expected = [
        "KeyW",
        "KeyA",
        "KeyS",
        "KeyD",
        "ArrowUp",
        "ArrowDown",
        "ArrowLeft",
        "ArrowRight",
    ];
    for k in expected {
        assert!(
            surface.keys.iter().any(|s| s == k),
            "default keys must contain {k}"
        );
    }
}

/// FX4.8: default persona is Uniform (Persona::default via #[default]).
#[test]
fn default_persona_uniform() {
    assert_eq!(
        bevy_swarm::enums::Persona::default(),
        bevy_swarm::enums::Persona::Uniform
    );
}

/// FX4: tier-0 chaos finds dead_left_key WITHOUT any adapter — the
/// walker reads keyboard via ButtonInput<KeyCode> and chaos samples
/// raw keys into RawActionQueue (actuated by the harness).
#[test]
#[serial]
fn autotest_finds_dead_left_key_without_adapter() {
    std::env::set_var("FIXTURE_BUG", "dead_left_key");
    // 60Hz over 6s: 360 samples, ~45 per key — well past the 10-sample
    // dead_verb threshold for the dead keys.
    let sc: bevy_swarm::scenario::Scenario = serde_json::from_str(
        r#"{"bot":{"type":"chaos","seed":1,"input_rate_hz":10},"duration_s":20.0,"invariants":[],"deny_ambiguities":false,"single_threaded":true}"#,
    ).unwrap();
    let config = AutotestConfig {
        num_seeds: 1,
        parallel: 1,
        base_scenario: sc,
        ..AutotestConfig::default()
    };
    let report = autotest(fixture_walker::WalkerGamePlugin, config).expect("autotest runs");
    std::env::remove_var("FIXTURE_BUG");
    assert!(
        report.failures.iter().any(|f| f.rule == "dead_verb"),
        "tier-0 chaos must find the dead left key: {:?}",
        report.failures
    );
}

/// FX4: clean walker passes tier-0 autotest.
#[test]
#[serial]
fn autotest_clean_walker_passes() {
    std::env::remove_var("FIXTURE_BUG");
    let sc: bevy_swarm::scenario::Scenario = serde_json::from_str(
        r#"{"bot":{"type":"chaos","seed":1,"input_rate_hz":10},"duration_s":20.0,"invariants":[],"deny_ambiguities":false,"single_threaded":true}"#,
    ).unwrap();
    let config = AutotestConfig {
        num_seeds: 1,
        parallel: 1,
        base_scenario: sc,
        ..AutotestConfig::default()
    };
    let report = autotest(fixture_walker::WalkerGamePlugin, config).expect("autotest runs");
    assert!(
        report.failures.is_empty(),
        "clean walker must pass tier-0: {:?}",
        report.failures
    );
}

/// FX4.3: a scenario rejected before running is counted as a FAILURE
/// by autotest (scenario_error rule, fingerprinted), not silently 0.
#[test]
fn autotest_counts_scenario_errors_as_failures() {
    // Planner bot without goals -> ScenarioError::Rejected at validate.
    let sc: bevy_swarm::scenario::Scenario = serde_json::from_str(
        r#"{"bot":{"type":"planner"},"duration_s":0.2,"invariants":[],"deny_ambiguities":false,"single_threaded":true}"#,
    )
    .unwrap();
    let config = AutotestConfig {
        num_seeds: 2,
        duration_s: 0.2,
        parallel: 1,
        base_scenario: sc,
        ..AutotestConfig::default()
    };
    let report = autotest(SimpleGamePlugin, config).expect("autotest runs");
    assert!(
        report.failures.iter().any(|f| f.rule == "scenario_error"),
        "scenario rejection must surface as failure: {:?}",
        report.failures
    );
    assert!(
        report.runs.iter().any(|r| r.status == "crash"),
        "rejected scenario runs are crash status: {:?}",
        report.runs
    );
}
