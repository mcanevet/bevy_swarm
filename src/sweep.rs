//! E3: Seed sweep + persisted regressions (proptest-regressions style)
//!
//! Sweeps seeds in parallel, deduplicates by fingerprint (T1), and
//! persists one regression file per fingerprint.

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::branch::{InProcess, ScenarioRunner};
use crate::fingerprint::Fingerprint;
use crate::scenario::Scenario;

/// Configuration for a seed sweep.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SweepConfig {
    /// Seed range (inclusive).
    pub start_seed: u64,
    pub end_seed: u64,
    /// Max failures to tolerate before early-stop.
    pub max_failures: Option<usize>,
    /// Parallelism cap (E1 run_matrix).
    pub parallel: usize,
    /// Whether to minimize each failure's scenario (B2).
    pub minimize: bool,
}

impl Default for SweepConfig {
    fn default() -> Self {
        Self {
            start_seed: 1,
            end_seed: 20,
            max_failures: Some(5),
            parallel: 4,
            minimize: false,
        }
    }
}

/// A single failure discovered during a sweep (deduped by fingerprint,
/// lowest seed wins). The report itself is not persisted — the
/// regression record carries the reproducing scenario.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SweepFailure {
    pub seed: u64,
    pub fingerprint: Fingerprint,
    pub rule: String,
}

/// Summary of a seed sweep run.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SweepReport {
    pub seeds_run: u64,
    pub failures: Vec<SweepFailure>,
    pub early_stop: bool,
}

/// Run a seed sweep over the configured range, deduplicating by
/// fingerprint (T1). Early-stop at max_failures if set. Optionally
/// minimizes failures (B2).
pub fn sweep_seeds<F: Fn() -> bevy::app::App + Sync>(
    runner: &InProcess<F>,
    base_scenario: Scenario,
    config: SweepConfig,
) -> Result<SweepReport, Box<dyn std::error::Error>> {
    let mut failures: Vec<SweepFailure> = vec![];
    let mut seen_fps: std::collections::HashSet<Fingerprint> = std::collections::HashSet::new();
    let mut seeds_run: u64 = 0;
    let mut early_stop = false;

    for seed in config.start_seed..=config.end_seed {
        let mut scenario = base_scenario.clone();
        scenario.bot.seed = seed;
        let rep = runner.run(&scenario)?;
        seeds_run += 1;

        // Extract fingerprints from violations (T1)
        for v in &rep.violations {
            if let Some(fp) = &v.fingerprint {
                if !seen_fps.contains(fp) {
                    seen_fps.insert(fp.clone());
                    failures.push(SweepFailure {
                        seed,
                        fingerprint: fp.clone(),
                        rule: v.rule.clone(),
                    });
                    if let Some(max) = config.max_failures {
                        if failures.len() >= max {
                            early_stop = true;
                            break;
                        }
                    }
                }
            }
        }
        if early_stop {
            break;
        }
    }

    Ok(SweepReport {
        seeds_run,
        failures,
        early_stop,
    })
}

/// Regression file format (versioned). One file per fingerprint.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RegressionRecord {
    pub schema_version: u32,
    pub fingerprint: Fingerprint,
    pub found_by_seeds: Vec<u64>,
    pub status: String, // "staging" | "promoted" | "quarantined"
    pub scenario: Scenario,
}

impl Default for RegressionRecord {
    fn default() -> Self {
        Self {
            schema_version: 1,
            fingerprint: Fingerprint("".to_string()),
            found_by_seeds: vec![],
            status: "staging".to_string(),
            scenario: serde_json::from_str(
                r#"{"bot":{"type":"chaos","seed":1},"duration_s":0.1,"invariants":[]}"#,
            )
            .expect("valid default scenario"),
        }
    }
}

/// Load all regression files from a directory (one per fingerprint).
/// Also accepts bare Scenarios (legacy format).
pub fn load_regressions(dir: &Path) -> Result<Vec<RegressionRecord>, std::io::Error> {
    let mut records = vec![];
    if !dir.exists() {
        return Ok(records);
    }
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let contents = fs::read_to_string(&path)?;
        // Try parsing as RegressionRecord first; fallback to Scenario.
        match serde_json::from_str::<RegressionRecord>(&contents) {
            Ok(rec) => records.push(rec),
            Err(_) => {
                // Legacy: bare Scenario — wrap it.
                if let Ok(scen) = serde_json::from_str::<Scenario>(&contents) {
                    records.push(RegressionRecord {
                        scenario: scen,
                        ..RegressionRecord::default()
                    });
                }
            }
        }
    }
    Ok(records)
}

/// Write a regression record to disk (filename: regression_<fp>.json).
pub fn write_regression(record: &RegressionRecord, dir: &Path) -> Result<(), std::io::Error> {
    fs::create_dir_all(dir)?;
    let filename = format!("regression_{}.json", record.fingerprint.0);
    fs::write(dir.join(filename), serde_json::to_string_pretty(record)?)
}

/// Run persisted regressions first (replay), then sweep new seeds.
/// Returns a combined report.
pub fn run_regressions_and_sweep<F: Fn() -> bevy::app::App + Sync>(
    runner: &InProcess<F>,
    base_scenario: Scenario,
    sweep_config: SweepConfig,
    reg_dir: &Path,
) -> Result<(Vec<RegressionRecord>, SweepReport), Box<dyn std::error::Error>> {
    let records = load_regressions(reg_dir)?;
    // Replay each regression (they should still fail)
    for rec in &records {
        let _ = runner.run(&rec.scenario);
        // TODO: track replay status; promote/quarantine logic later.
    }
    let sweep = sweep_seeds(runner, base_scenario, sweep_config)?;
    Ok((records, sweep))
}
