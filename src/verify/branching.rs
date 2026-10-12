use crate::harness::*;
use bevy::prelude::*;

#[allow(unused_imports)]
use super::fixtures::*;

// ---------------------------------------------------------------------------
// E1: ScenarioRunner abstraction + parallel runner
// ---------------------------------------------------------------------------

#[test]
fn parallel_matches_sequential() {
    let variants: Vec<(String, Scenario)> = (1..=6)
        .map(|i| {
            (
                format!("seed-{i}"),
                serde_json::from_str(&format!(
                    r#"{{"bot":{{"type":"chaos","seed":{i}}},"duration_s":0.2,"invariants":[]}}"#
                ))
                .unwrap(),
            )
        })
        .collect();
    let seq = run_branch_matrix(build_app, variants.clone()).unwrap();
    let par = crate::branch::run_matrix(
        &crate::branch::InProcess { factory: build_app },
        variants,
        3,
    )
    .unwrap();
    assert_eq!(seq.outcomes.len(), par.outcomes.len());
    for (s, p) in seq.outcomes.iter().zip(par.outcomes.iter()) {
        assert_eq!(s.variant_name, p.variant_name);
        assert_eq!(s.report.status, p.report.status);
        assert_eq!(s.report.violations, p.report.violations);
        assert_eq!(s.report.frame_count, p.report.frame_count);
    }
}

#[test]
fn fresh_thread_per_scenario() {
    // Each scenario runs on its own freshly-spawned thread (thread ids
    // differ across runs; captured via a runner that records them).
    use std::sync::Mutex;
    let seen = Mutex::new(Vec::<std::thread::ThreadId>::new());
    struct ThreadRecording<F> {
        factory: F,
        seen: std::sync::Arc<Mutex<Vec<std::thread::ThreadId>>>,
    }
    impl<F: Fn() -> App + Sync> crate::branch::ScenarioRunner for ThreadRecording<F> {
        fn run(
            &self,
            scenario: &Scenario,
        ) -> Result<crate::harness::PlaytestReport, crate::harness::ScenarioError> {
            self.seen.lock().unwrap().push(std::thread::current().id());
            let runner = crate::branch::InProcess {
                factory: &self.factory,
            };
            crate::branch::ScenarioRunner::run(&runner, scenario)
        }
    }
    let seen = std::sync::Arc::new(seen);
    let runner = ThreadRecording {
        factory: build_app,
        seen: std::sync::Arc::clone(&seen),
    };
    let variants: Vec<(String, Scenario)> = (1..=4)
        .map(|i| {
            (
                format!("s{i}"),
                serde_json::from_str(&format!(
                    r#"{{"bot":{{"type":"chaos","seed":{i}}},"duration_s":0.1,"invariants":[]}}"#
                ))
                .unwrap(),
            )
        })
        .collect();
    crate::branch::run_matrix(&runner, variants, 2).unwrap();
    let ids = seen.lock().unwrap().clone();
    assert_eq!(ids.len(), 4, "one run per scenario");
    let unique: std::collections::HashSet<_> = ids.iter().collect();
    assert_eq!(unique.len(), 4, "each scenario on its own fresh thread");
}

// ---------------------------------------------------------------------------
// T1: Failure fingerprints + normalizer + dedup
// ---------------------------------------------------------------------------

#[test]
fn normalization_merges_entity_and_frame_variants() {
    let n = crate::fingerprint::default_normalizer();
    let a = n.normalize("out of bounds entity:12v3 at frame 10");
    let b = n.normalize("out of bounds entity:44v2 at frame 999");
    assert_eq!(a, b, "entity ids and frames must normalize away");
}

#[test]
fn stable_hash_across_runs() {
    let n = crate::fingerprint::default_normalizer();
    let f1 = n.fingerprint(
        "violation",
        "ball_in_bounds",
        &["Ball".to_string()],
        "out of bounds (10001.0, 0.0, 0.0)",
    );
    let f2 = n.fingerprint(
        "violation",
        "ball_in_bounds",
        &["Ball".to_string()],
        "out of bounds (10001.0, 0.0, 0.0)",
    );
    assert_eq!(f1, f2);
    let f3 = n.fingerprint(
        "violation",
        "ball_in_bounds",
        &["Other".to_string()],
        "out of bounds (10001.0, 0.0, 0.0)",
    );
    assert_ne!(f1, f3, "different location → different fingerprint");
    let f4 = n.fingerprint(
        "violation",
        "other_rule",
        &["Ball".to_string()],
        "out of bounds (10001.0, 0.0, 0.0)",
    );
    assert_ne!(f1, f4, "different rule → different fingerprint");
}

#[test]
fn every_report_failure_has_fingerprint() {
    let scen: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"chaos","seed":42},"duration_s":0.3,"invariants":[
            {"name":"ball_in_bounds","rule":"nodes_in_bounds",
             "min_x":-100,"max_x":100,"min_y":-100,"max_y":100}
        ],"setup":{}}"#,
    )
    .unwrap();
    let mut app = build_app();
    app.add_systems(Update, inject_bounds_bug);
    let rep = run_scenario(&mut app, &scen).unwrap();
    assert!(!rep.violations.is_empty());
    for v in &rep.violations {
        assert!(
            v.fingerprint.is_some(),
            "violation {} lacks a fingerprint",
            v.rule
        );
        assert_eq!(v.fingerprint_scheme, 1);
    }
    // Dedup by fingerprint: the seeded crash bug produces one distinct
    // fingerprint for the bounds rule.
    let uniq: std::collections::HashSet<_> = rep
        .violations
        .iter()
        .filter_map(|v| v.fingerprint.clone())
        .collect();
    assert!(!uniq.is_empty());
}

#[test]
fn same_bug_across_seeds_shares_fingerprint() {
    // The same planted bounds bug, three seeds → one fingerprint.
    let mut fps: Vec<crate::fingerprint::Fingerprint> = vec![];
    for seed in [1u64, 2, 3] {
        let scen: Scenario = serde_json::from_str(&format!(
            r#"{{"bot":{{"type":"chaos","seed":{seed}}},"duration_s":0.3,"invariants":[
                {{"name":"ball_in_bounds","rule":"nodes_in_bounds",
                 "min_x":-100,"max_x":100,"min_y":-100,"max_y":100}}
            ],"setup":{{}}}}"#
        ))
        .unwrap();
        let mut app = build_app();
        app.add_systems(Update, inject_bounds_bug);
        let rep = run_scenario(&mut app, &scen).unwrap();
        for v in &rep.violations {
            if v.rule == "ball_in_bounds" {
                if let Some(f) = &v.fingerprint {
                    fps.push(f.clone());
                }
            }
        }
    }
    assert!(!fps.is_empty(), "bug never fired");
    let uniq: std::collections::HashSet<_> = fps.iter().collect();
    assert_eq!(uniq.len(), 1, "one fingerprint for one bug across seeds");
}

// ---------------------------------------------------------------------------
// E3: Seed sweep + persisted regressions
// ---------------------------------------------------------------------------

#[test]
fn sweep_dedupes_by_fingerprint() {
    // The planted bounds bug fires across seeds but produces ONE
    // fingerprint → one sweep failure.
    let mut base: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"chaos","seed":1},"duration_s":0.3,"invariants":[
            {"name":"ball_in_bounds","rule":"nodes_in_bounds",
             "min_x":-100,"max_x":100,"min_y":-100,"max_y":100}
        ],"setup":{}}"#,
    )
    .unwrap();
    base.bot.seed = 1;
    let runner = crate::branch::InProcess {
        factory: build_app_with_bounds_bug,
    };
    let cfg = crate::sweep::SweepConfig {
        start_seed: 1,
        end_seed: 6,
        max_failures: Some(3),
        parallel: 1,
        minimize: false,
    };
    let report = crate::sweep::sweep_seeds(&runner, base, cfg).unwrap();
    assert!(report.seeds_run >= 1);
    assert_eq!(
        report.failures.len(),
        1,
        "one fingerprint for one bug: {:?}",
        report.failures
    );
    assert_eq!(report.failures[0].seed, 1, "lowest seed wins");
}

fn build_app_with_bounds_bug() -> App {
    let mut app = build_app();
    app.add_systems(Update, inject_bounds_bug);
    app
}

#[test]
fn regression_roundtrip_one_file_per_fingerprint() {
    let dir = std::env::temp_dir().join(format!("bevy_swarm_regr_test_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);

    // Write two records with different fingerprints.
    let fp1 = crate::fingerprint::Fingerprint("aaaa1111aaaa1111".into());
    let fp2 = crate::fingerprint::Fingerprint("bbbb2222bbbb2222".into());
    let scenario: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"chaos","seed":7},"duration_s":0.1,"invariants":[]}"#,
    )
    .unwrap();
    for fp in [&fp1, &fp2] {
        let rec = crate::sweep::RegressionRecord {
            schema_version: 1,
            fingerprint: fp.clone(),
            found_by_seeds: vec![7],
            status: "staging".into(),
            scenario: scenario.clone(),
        };
        crate::sweep::write_regression(&rec, &dir).unwrap();
    }
    // Two files, one per fingerprint, named by fingerprint.
    let files: Vec<_> = std::fs::read_dir(&dir).unwrap().collect();
    assert_eq!(files.len(), 2);
    let names: Vec<String> = files
        .iter()
        .map(|f| {
            f.as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    assert!(names.contains(&"regression_aaaa1111aaaa1111.json".to_string()));
    assert!(names.contains(&"regression_bbbb2222bbbb2222.json".to_string()));

    // Roundtrip: load_regressions parses both forms (record + bare scenario).
    let loaded = crate::sweep::load_regressions(&dir, &mut 0).unwrap();
    assert_eq!(loaded.len(), 2);
    assert!(loaded.iter().any(|r| r.fingerprint == fp1));
    assert!(loaded.iter().any(|r| r.fingerprint == fp2));
    assert_eq!(loaded[0].schema_version, 1);
    assert_eq!(loaded[0].status, "staging");

    // Legacy bare Scenario file also loads.
    std::fs::write(
        dir.join("legacy.json"),
        serde_json::to_string(&scenario).unwrap(),
    )
    .unwrap();
    let loaded2 = crate::sweep::load_regressions(&dir, &mut 0).unwrap();
    assert_eq!(loaded2.len(), 3);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn sweep_replays_regressions_first() {
    // run_regressions_and_sweep loads persisted records, replays them,
    // then sweeps. The regression dir here is empty → pure sweep.
    let dir = std::env::temp_dir().join(format!("bevy_swarm_regr_empty_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let base: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"chaos","seed":1},"duration_s":0.1,"invariants":[]}"#,
    )
    .unwrap();
    let runner = crate::branch::InProcess { factory: build_app };
    let cfg = crate::sweep::SweepConfig {
        start_seed: 1,
        end_seed: 3,
        max_failures: None,
        parallel: 1,
        minimize: false,
    };
    let report = crate::sweep::run_regressions_and_sweep(&runner, base, cfg, &dir).unwrap();
    assert!(
        report.replays.is_empty(),
        "empty dir → no regressions loaded"
    );
    assert_eq!(report.sweep.seeds_run, 3);
    assert!(report.sweep.failures.is_empty(), "clean game → no failures");
    let _ = std::fs::remove_dir_all(&dir);
}
