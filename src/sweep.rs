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
///
/// FX12 (E3): runs in PARALLEL via the E1 matrix (was sequential);
/// a single ScenarioError no longer aborts the sweep — it becomes a
/// Crash-status failure for that seed only.
pub fn sweep_seeds<F: Fn() -> bevy::app::App + Sync>(
    runner: &InProcess<F>,
    base_scenario: Scenario,
    config: SweepConfig,
) -> Result<SweepReport, Box<dyn std::error::Error>> {
    let mut failures: Vec<SweepFailure> = vec![];
    let mut seen_fps: std::collections::HashSet<Fingerprint> = std::collections::HashSet::new();
    let mut seeds_run: u64 = 0;
    let mut early_stop = false;

    let variants: Vec<(String, Scenario)> = (config.start_seed..=config.end_seed)
        .map(|seed| {
            let mut scenario = base_scenario.clone();
            scenario.bot.seed = seed;
            (format!("seed-{seed}"), scenario)
        })
        .collect();
    // FX12 (E3): parallel via E1's run_matrix (panic-isolated).
    let matrix = crate::branch::run_matrix(runner, variants, config.parallel.max(1))?;

    for outcome in &matrix.outcomes {
        seeds_run += 1;
        let rep = &outcome.report;
        // FX12 (E3): scenario errors surface as Crash status —
        // recorded as a synthetic failure, not an abort.
        if rep.status == crate::enums::PlaytestStatus::Crash && rep.violations.is_empty() {
            if let Some(err) = &rep.error {
                failures.push(SweepFailure {
                    seed: 0,
                    fingerprint: Fingerprint(format!("crash:{err}")),
                    rule: "scenario_error".to_string(),
                });
            }
        }
        // Extract fingerprints from violations (T1)
        for v in &rep.violations {
            if let Some(fp) = &v.fingerprint {
                if !seen_fps.contains(fp) {
                    seen_fps.insert(fp.clone());
                    failures.push(SweepFailure {
                        seed: outcome
                            .variant_name
                            .strip_prefix("seed-")
                            .and_then(|s| s.parse().ok())
                            .unwrap_or(0),
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

/// Write a regression record to disk (filename: `regression_<fp>.json`).
pub fn write_regression(record: &RegressionRecord, dir: &Path) -> Result<(), std::io::Error> {
    fs::create_dir_all(dir)?;
    let filename = format!("regression_{}.json", record.fingerprint.0);
    fs::write(dir.join(filename), serde_json::to_string_pretty(record)?)
}

/// Replay outcome for a persisted regression.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RegressionReplay {
    pub fingerprint: Fingerprint,
    /// true = still reproduces (failures present); false = flaky/fixed.
    pub still_fails: bool,
}

/// Combined report (FX12 E3): replay outcomes + sweep, serializable
/// for embedding in the run report.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RegressionsAndSweepReport {
    pub replays: Vec<RegressionReplay>,
    pub sweep: SweepReport,
}

/// Should new regression files be written? U1 conventions:
/// BEVY_SWARM_UPDATE=regressions (or =all).
pub fn update_regressions_enabled() -> bool {
    crate::conventions::can_update("regressions")
}

/// Run persisted regressions first (replay), then sweep new seeds.
/// Returns a combined report. FX12 (E3): replay outcomes are TRACKED
/// (still_fails), new failures are persisted (one file per
/// fingerprint) when BEVY_SWARM_UPDATE=regressions, and one
/// ScenarioError aborts only that seed (handled inside sweep_seeds).
pub fn run_regressions_and_sweep<F: Fn() -> bevy::app::App + Sync>(
    runner: &InProcess<F>,
    base_scenario: Scenario,
    sweep_config: SweepConfig,
    reg_dir: &Path,
) -> Result<RegressionsAndSweepReport, Box<dyn std::error::Error>> {
    let records = load_regressions(reg_dir)?;
    let base_for_persist = base_scenario.clone();
    // Replay each regression (they should still fail).
    let mut replays = Vec::new();
    for rec in &records {
        let rep = runner.run(&rec.scenario)?;
        let still_fails = !matches!(rep.status, crate::enums::PlaytestStatus::Pass);
        replays.push(RegressionReplay {
            fingerprint: rec.fingerprint.clone(),
            still_fails,
        });
    }
    let sweep = sweep_seeds(runner, base_scenario, sweep_config)?;
    // FX12 (E3): persist new failures (one file per fingerprint) when
    // the conventions update mode is on.
    if update_regressions_enabled() {
        for f in &sweep.failures {
            let record = RegressionRecord {
                schema_version: 1,
                fingerprint: f.fingerprint.clone(),
                found_by_seeds: vec![f.seed],
                status: "staging".to_string(),
                scenario: {
                    let mut s = base_for_persist.clone();
                    s.bot.seed = f.seed;
                    s
                },
            };
            write_regression(&record, reg_dir)?;
        }
    }
    Ok(RegressionsAndSweepReport { replays, sweep })
}
