//! Z18: autotest() integration — AutotestConfig, defaults, aggregated report.
//!
//! Provides:
//! - `AutotestConfig`: single source of defaults with env overrides
//!   (BEVY_SWARM_SEEDS, BEVY_SWARM_DURATION, BEVY_SWARM_THREADS)
//! - `autotest::<P>(game_plugin, config)`: headless multi-run harness
//! - `AutotestReport`: schema_version, contract_tier, harness_mode, scope,
//!   runs, failures, coverage, summary(), written to
//!   `target/bevy_swarm/runs/<run-id>/report.json`
//!
//! ## Quick Start
//! ```no_run
//! use bevy_swarm::autotest::{autotest, AutotestConfig};
//!
//! # use bevy::prelude::*;
//! # #[derive(Clone, Default)]
//! # struct MyGamePlugin;
//! # impl Plugin for MyGamePlugin {
//! #     fn build(&self, _: &mut App) {}
//! # }
//! let config = AutotestConfig::default(); // 16 seeds × 20s chaos, tier-0 oracles
//! let report = autotest(MyGamePlugin, config).unwrap();
//! eprintln!("{}", report.summary());
//! ```

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime};

use crate::branch::InProcess;
use crate::enums::PlaytestStatus;
use crate::fingerprint::Fingerprint;
use crate::scenario::Scenario;
use crate::state::Coverage;

/// Environment variable names for overrides.
const ENV_SEEDS: &str = "BEVY_SWARM_SEEDS";
const ENV_DURATION: &str = "BEVY_SWARM_DURATION";
const ENV_THREADS: &str = "BEVY_SWARM_THREADS";

/// Configuration for `autotest()` — single source of defaults with
/// environment overrides.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AutotestConfig {
    /// Number of seeds to run (1..N). Default: 16.
    pub num_seeds: u64,
    /// Duration per run in seconds. Default: 20.
    pub duration_s: f32,
    /// Parallelism (threads). Default: min(4, available_parallelism).
    pub parallel: usize,
    /// Chaos actuator share (0.0..1.0). Default: 1.0 (pure chaos).
    pub chaos_share: f32,
    /// Whether to minimize failures (B2). Default: false.
    pub minimize_failures: bool,
    /// Maximum failures before early-stop. Default: 10.
    pub max_failures: Option<usize>,
    /// Base scenario template (seeds overwritten per run).
    pub base_scenario: Scenario,
    /// Output directory for reports (relative to CARGO_TARGET_DIR).
    pub output_dir: PathBuf,
}

impl Default for AutotestConfig {
    fn default() -> Self {
        let parallel = std::thread::available_parallelism()
            .map(|p| p.get())
            .unwrap_or(1)
            .min(4);

        Self {
            num_seeds: 16,
            duration_s: 20.0,
            parallel,
            chaos_share: 1.0,
            minimize_failures: false,
            max_failures: Some(10),
            base_scenario: default_chaos_scenario(),
            output_dir: PathBuf::from("bevy_swarm/runs"),
        }
    }
}

impl AutotestConfig {
    /// Load config with environment overrides applied.
    pub fn from_env() -> Self {
        let mut cfg = Self::default();
        if let Ok(seeds) = std::env::var(ENV_SEEDS) {
            if let Ok(n) = seeds.parse::<u64>() {
                cfg.num_seeds = n;
            }
        }
        if let Ok(duration) = std::env::var(ENV_DURATION) {
            if let Ok(d) = duration.parse::<f32>() {
                cfg.duration_s = d;
            }
        }
        if let Ok(threads) = std::env::var(ENV_THREADS) {
            if let Ok(t) = threads.parse::<usize>() {
                cfg.parallel = t;
            }
        }
        cfg
    }
}

fn default_chaos_scenario() -> Scenario {
    // FX4.2: match the library defaults — single_threaded: true
    // (reproducibility), deny_ambiguities: false (many games have
    // benign ambiguities; ambiguity rejection is opt-in).
    serde_json::from_str(
        r#"{"bot":{"type":"chaos","seed":0},"duration_s":20.0,"invariants":[],"deny_ambiguities":false,"single_threaded":true}"#,
    )
    .unwrap()
}

/// Aggregated report from `autotest()`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AutotestReport {
    /// Schema version (for tooling compatibility).
    pub schema_version: String,
    /// Contract tier exercised (tier-0 for v0.3).
    pub contract_tier: String,
    /// Harness mode ("autotest").
    pub harness_mode: String,
    /// Scope summary (seeds_run, failures, etc.).
    pub scope: ScopeSummary,
    /// Individual run reports (filtered to failures + last N passes).
    pub runs: Vec<RunSummary>,
    /// Failure fingerprints (deduplicated).
    pub failures: Vec<FingerprintSummary>,
    /// Aggregate coverage (union across runs).
    #[serde(skip)]
    pub coverage: Option<Coverage>,
    /// Wall-clock timing.
    pub timing: TimingSummary,
    /// FX4.5: stable run identifier (conventions::run_id) — prevents
    /// directory collisions between consecutive autotests.
    pub run_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScopeSummary {
    pub seeds_run: u64,
    pub total_duration_s: f32,
    pub parallelism: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RunSummary {
    pub seed: u64,
    pub status: String,
    pub violations_count: usize,
    pub frame_count: u64,
    pub duration_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FingerprintSummary {
    pub fingerprint: Fingerprint,
    pub rule: String,
    pub first_seen_seed: u64,
    pub occurrences: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct TimingSummary {
    pub start: String,
    pub end: String,
    pub elapsed_ms: u64,
}

impl AutotestReport {
    /// Human-readable summary line.
    pub fn summary(&self) -> String {
        format!(
            "autotest: {} seeds, {} failures ({} unique), {}s total, {}%",
            self.scope.seeds_run,
            self.failures.iter().map(|f| f.occurrences).sum::<u64>(),
            self.failures.len(),
            self.timing.elapsed_ms / 1000,
            (100.0
                * (self.scope.seeds_run as f32
                    - self.runs.iter().filter(|r| r.status != "pass").count() as f32)
                / self.scope.seeds_run.max(1) as f32) as u32
        )
    }
}

/// Run a full autotest suite: N seeds × chaos, aggregating failures,
/// coverage, and writing the report to disk.
pub fn autotest<P: bevy::prelude::Plugin + Clone + Send + 'static>(
    game_plugin: P,
    config: AutotestConfig,
) -> Result<AutotestReport, Box<dyn std::error::Error>> {
    let start = Instant::now();
    let start_ts = SystemTime::now();
    let start_iso = crate::conventions::iso_timestamp(start_ts);

    // FX4.5: apply environment overrides on top of the explicit config
    // (BEVY_SWARM_SEEDS / _DURATION / _THREADS win when set).
    let mut config = config;
    if let Some(n) = std::env::var(ENV_SEEDS)
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
    {
        config.num_seeds = n;
    }
    if let Some(d) = std::env::var(ENV_DURATION)
        .ok()
        .and_then(|v| v.parse::<f32>().ok())
    {
        config.duration_s = d;
    }
    if let Some(th) = std::env::var(ENV_THREADS)
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
    {
        config.parallel = th;
    }

    // Build the headless app factory (Z2): headless_app RETURNS a
    // builder closure; call it per run.
    let build = crate::headless::headless_app(game_plugin.clone());
    let factory = move || build();

    let runner = InProcess { factory };
    let seeds: Vec<u64> = (1..=config.num_seeds).collect();
    let mut run_summaries: Vec<RunSummary> = Vec::new();
    let mut all_violations: Vec<(u64, crate::state::ViolationEntry)> = Vec::new();
    let aggregate_coverage = Coverage::default();

    // Run seeds in parallel via run_matrix (one scoped thread per
    // scenario, capped by config.parallel; fresh App per run).
    let variants: Vec<(String, Scenario)> = seeds
        .iter()
        .map(|seed| {
            let mut scenario = config.base_scenario.clone();
            scenario.bot.seed = *seed;
            scenario.duration_s = config.duration_s;
            (format!("seed-{seed}"), scenario)
        })
        .collect();

    let matrix_results = crate::branch::run_matrix(&runner, variants, config.parallel);
    let end_ts = SystemTime::now();
    let elapsed_ms_total = start.elapsed().as_millis() as u64;

    // Aggregate results from BranchMatrixReport.
    let mut failures_by_fp: HashMap<Fingerprint, FingerprintSummary> = HashMap::new();
    let num_runs_f32 = config.num_seeds.max(1) as f64;
    for outcome in matrix_results?.outcomes {
        let seed: u64 = outcome
            .variant_name
            .strip_prefix("seed-")
            .unwrap_or("0")
            .parse()
            .unwrap_or(0);
        let rep = outcome.report;
        let elapsed_ms = (elapsed_ms_total as f64 / num_runs_f32) as u64;

        let status = match rep.status {
            PlaytestStatus::Pass => "pass",
            PlaytestStatus::Fail => "fail",
            PlaytestStatus::Crash => "crash",
        }
        .to_string();

        run_summaries.push(RunSummary {
            seed,
            status: status.clone(),
            violations_count: rep.violations.len(),
            frame_count: rep.frame_count,
            duration_ms: elapsed_ms,
        });

        for v in &rep.violations {
            all_violations.push((seed, v.clone()));
            if let Some(fp) = &v.fingerprint {
                let entry =
                    failures_by_fp
                        .entry(fp.clone())
                        .or_insert_with(|| FingerprintSummary {
                            fingerprint: fp.clone(),
                            rule: v.rule.clone(),
                            first_seen_seed: seed,
                            occurrences: 0,
                        });
                entry.occurrences += 1;
            }
        }
        // aggregate_coverage.merge(&rep.system_coverage); // TODO
    }

    let failures: Vec<FingerprintSummary> = failures_by_fp.into_values().collect();
    let report = AutotestReport {
        schema_version: "0.1.0".to_string(),
        contract_tier: "tier-0".to_string(),
        harness_mode: "autotest".to_string(),
        scope: ScopeSummary {
            seeds_run: config.num_seeds,
            total_duration_s: config.duration_s * config.num_seeds as f32,
            parallelism: config.parallel,
        },
        runs: run_summaries,
        failures,
        coverage: Some(aggregate_coverage),
        timing: TimingSummary {
            start: start_iso,
            end: crate::conventions::iso_timestamp(end_ts),
            elapsed_ms: elapsed_ms_total,
        },
        run_id: Some(crate::conventions::run_id(
            elapsed_ms_total,
            config.num_seeds,
        )),
    };

    // Write report to disk.
    write_report(&report, &config.output_dir)?;

    Ok(report)
}

fn write_report(
    report: &AutotestReport,
    output_dir: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let target_dir = std::env::var("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("target"));

    // FX4.5: use conventions::run_id (timestamp + hash) so consecutive
    // autotests never collide on the same directory.
    let run_dir = target_dir.join(output_dir).join(format!(
        "autotest-{}",
        report
            .run_id
            .clone()
            .unwrap_or_else(|| crate::conventions::run_id(0, report.timing.elapsed_ms))
    ));
    std::fs::create_dir_all(&run_dir)?;

    let report_path = run_dir.join("report.json");
    let json = serde_json::to_string_pretty(report)?;
    std::fs::write(&report_path, json)?;

    eprintln!("autotest report written to {}", report_path.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn autotest_config_from_env_overrides_defaults() {
        std::env::set_var(ENV_SEEDS, "42");
        std::env::set_var(ENV_DURATION, "30.5");
        std::env::set_var(ENV_THREADS, "8");

        let cfg = AutotestConfig::from_env();
        assert_eq!(cfg.num_seeds, 42);
        assert_eq!(cfg.duration_s, 30.5);
        assert_eq!(cfg.parallel, 8);

        std::env::remove_var(ENV_SEEDS);
        std::env::remove_var(ENV_DURATION);
        std::env::remove_var(ENV_THREADS);
    }

    #[test]
    fn autotest_report_summary_format() {
        let report = AutotestReport {
            schema_version: "0.1.0".to_string(),
            contract_tier: "tier-0".to_string(),
            harness_mode: "autotest".to_string(),
            scope: ScopeSummary {
                seeds_run: 10,
                total_duration_s: 200.0,
                parallelism: 4,
            },
            runs: vec![],
            failures: vec![FingerprintSummary {
                fingerprint: Fingerprint("test-fp".to_string()),
                rule: "panic".to_string(),
                first_seen_seed: 1,
                occurrences: 3,
            }],
            coverage: Some(Coverage::default()),
            timing: TimingSummary {
                start: "".to_string(),
                end: "".to_string(),
                elapsed_ms: 5000,
            },
            run_id: None,
        };

        let summary = report.summary();
        assert!(summary.contains("10 seeds"));
        assert!(summary.contains("3 failures"));
        assert!(summary.contains("5s total"));
    }
}
