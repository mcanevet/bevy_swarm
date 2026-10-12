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
    /// 716.35 (qa8.14): regression files skipped due to parse errors.
    pub skipped_regression_files: u64,
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
        skipped_regression_files: 0,
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
/// 716.35 (qa8.14): logs a warning for each malformed file and returns
/// the count of skipped files via the output parameter.
pub fn load_regressions(
    dir: &Path,
    skipped_out: &mut u64,
) -> Result<Vec<RegressionRecord>, std::io::Error> {
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
            Err(e1) => {
                // Legacy: bare Scenario — wrap it.
                if let Ok(scen) = serde_json::from_str::<Scenario>(&contents) {
                    records.push(RegressionRecord {
                        scenario: scen,
                        ..RegressionRecord::default()
                    });
                } else {
                    // 716.35 (qa8.14): visible warning naming the file
                    // and the parse error (silently skipping corrupt
                    // regression files causes false confidence).
                    let name = path.file_name().unwrap_or_default().to_string_lossy();
                    eprintln!(
                        "WARNING: skipped regression file '{}' (malformed JSON): {}",
                        name, e1
                    );
                    *skipped_out += 1;
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
    let mut skipped_reg_files = 0u64;
    let records = load_regressions(reg_dir, &mut skipped_reg_files)?;
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
    let mut sweep = sweep_seeds(runner, base_scenario, sweep_config)?;
    sweep.skipped_regression_files = skipped_reg_files;
    // FX12 (E3): persist new failures (one file per fingerprint) when
    // the conventions update mode is on.
    if update_regressions_enabled() {
        // Dedup by fingerprint: multiple seeds hitting the SAME bug
        // must produce ONE regression record, not one per seed. Keep
        // the lowest seed as the persisted scenario seed; accumulate
        // the others into found_by_seeds.
        let mut records: Vec<RegressionRecord> = Vec::new();
        for f in &sweep.failures {
            let seed = f.seed;
            if let Some(rec) = records.iter_mut().find(|r| r.fingerprint == f.fingerprint) {
                if seed < rec.scenario.bot.seed {
                    rec.scenario.bot.seed = seed;
                }
                if !rec.found_by_seeds.contains(&seed) {
                    rec.found_by_seeds.push(seed);
                }
            } else {
                records.push(RegressionRecord {
                    schema_version: 1,
                    fingerprint: f.fingerprint.clone(),
                    found_by_seeds: vec![seed],
                    status: "staging".to_string(),
                    scenario: {
                        let mut s = base_for_persist.clone();
                        s.bot.seed = seed;
                        s
                    },
                });
            }
        }
        for record in records {
            write_regression(&record, reg_dir)?;
        }
    }
    Ok(RegressionsAndSweepReport { replays, sweep })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fingerprint::Fingerprint;
    use crate::scenario::Scenario;

    #[test]
    fn regression_dedup_same_fingerprint_across_seeds() {
        // Multiple seeds hitting the SAME bug must produce ONE
        // regression record with found_by_seeds accumulated.
        let failures = vec![
            SweepFailure {
                seed: 3,
                fingerprint: Fingerprint("fp1".to_string()),
                rule: "bug".to_string(),
            },
            SweepFailure {
                seed: 1,
                fingerprint: Fingerprint("fp1".to_string()),
                rule: "bug".to_string(),
            },
            SweepFailure {
                seed: 5,
                fingerprint: Fingerprint("fp2".to_string()),
                rule: "bug".to_string(),
            },
        ];

        let base: Scenario =
            serde_json::from_str(r#"{"bot":{"type":"chaos","seed":0},"duration_s":0.1}"#).unwrap();

        // Dedup logic from regressions_and_sweep
        let mut records: Vec<RegressionRecord> = Vec::new();
        for f in &failures {
            let seed = f.seed;
            if let Some(rec) = records.iter_mut().find(|r| r.fingerprint == f.fingerprint) {
                if seed < rec.scenario.bot.seed {
                    rec.scenario.bot.seed = seed;
                }
                if !rec.found_by_seeds.contains(&seed) {
                    rec.found_by_seeds.push(seed);
                }
            } else {
                records.push(RegressionRecord {
                    schema_version: 1,
                    fingerprint: f.fingerprint.clone(),
                    found_by_seeds: vec![seed],
                    status: "staging".to_string(),
                    scenario: {
                        let mut s = base.clone();
                        s.bot.seed = seed;
                        s
                    },
                });
            }
        }

        assert_eq!(records.len(), 2, "two unique fingerprints");
        let fp1_rec = records.iter().find(|r| r.fingerprint.0 == "fp1").unwrap();
        assert_eq!(fp1_rec.scenario.bot.seed, 1, "lowest seed persisted");
        assert!(fp1_rec.found_by_seeds.contains(&1));
        assert!(fp1_rec.found_by_seeds.contains(&3));
    }
}

#[cfg(test)]
mod tests_qa8_14 {
    use super::*;
    use std::io::Write;

    /// qa8.14: malformed regression files are skipped with a visible
    /// warning naming the file, and the skipped count is reported.
    #[test]
    fn malformed_regression_file_warns_and_counts() {
        let dir = std::env::temp_dir().join(format!("qa8_14_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // One good record, one corrupt file (partial write).
        let good: Scenario = serde_json::from_str(
            r#"{"bot":{"type":"replay","inputs":[]},"duration_s":0.1,"tps":60}"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("regression_good.json"),
            serde_json::to_string(&RegressionRecord {
                scenario: good,
                ..Default::default()
            })
            .unwrap(),
        )
        .unwrap();
        let mut f = std::fs::File::create(dir.join("regression_corrupt.json")).unwrap();
        f.write_all(b"{\"scenario\": {\"du").unwrap(); // truncated JSON
        drop(f);

        let mut skipped = 0u64;
        let loaded = load_regressions(&dir, &mut skipped).unwrap();
        assert_eq!(loaded.len(), 1, "good record loads");
        assert_eq!(skipped, 1, "corrupt file counted as skipped");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// SweepReport carries the skipped count for surfacing in reports.
    #[test]
    fn sweep_report_has_skipped_count() {
        let rep = SweepReport {
            seeds_run: 0,
            failures: vec![],
            early_stop: false,
            skipped_regression_files: 2,
        };
        assert_eq!(rep.skipped_regression_files, 2);
    }
}
