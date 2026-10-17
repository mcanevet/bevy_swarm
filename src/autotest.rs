//! Z18: autotest() integration — AutotestConfig, defaults, aggregated report.
//!
//! Provides:
//! - `AutotestConfig`: single source of defaults with env overrides
//!   (BEVY_PLAYTEST_SEEDS, BEVY_PLAYTEST_DURATION, BEVY_PLAYTEST_THREADS)
//! - `autotest::<P>(game_plugin, config)`: headless multi-run harness
//! - `AutotestReport`: schema_version, contract_tier, harness_mode, scope,
//!   runs, failures, coverage, summary(), written to
//!   `target/bevy_playtest/runs/<run-id>/report.json`
//!
//! ## Quick Start
//! ```no_run
//! use bevy_playtest::autotest::{autotest, AutotestConfig};
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
use crate::branch::ScenarioRunner;
use crate::enums::PlaytestStatus;
use crate::fingerprint::Fingerprint;
use crate::scenario::Scenario;
use crate::state::Coverage;

/// Environment variable names for overrides.
const ENV_SEEDS: &str = "BEVY_PLAYTEST_SEEDS";
const ENV_DURATION: &str = "BEVY_PLAYTEST_DURATION";
const ENV_THREADS: &str = "BEVY_PLAYTEST_THREADS";
/// smu: pin ONE exact seed (reproduction). Wins over ENV_SEEDS.
const ENV_SEED: &str = "BEVY_PLAYTEST_SEED";

/// wyx: pre-rename aliases — BEVY_SWARM_* still honored if the new
/// name isn't set (cheap backward compatibility for existing CI).
fn env_override(new_name: &str) -> Option<String> {
    match std::env::var(new_name) {
        Ok(v) => Some(v),
        Err(_) => {
            let legacy = new_name.replace("BEVY_PLAYTEST_", "BEVY_SWARM_");
            std::env::var(&legacy).ok()
        }
    }
}

/// Configuration for `autotest()` — single source of defaults with
/// environment overrides.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AutotestConfig {
    /// Number of seeds to run (1..N). Default: 4 (dbt: fast first
    /// `cargo test` — a headless sprite game failed red on its first
    /// run because 16 seeds x 20s meant 5+ minutes before any signal).
    /// Use [`AutotestConfig::thorough`] for the release-grade preset.
    pub num_seeds: u64,
    /// Explicit seed list. When set, overrides `num_seeds`
    /// (expectations pin exact seeds like `[42]`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seeds: Option<Vec<u64>>,
    /// Duration per run in seconds. Default: 5.
    pub duration_s: f32,
    /// Parallelism (threads). Default: min(4, available_parallelism).
    pub parallel: usize,
    /// Whether to minimize failures (B2). Default: false.
    pub minimize_failures: bool,
    /// Maximum failures before early-stop. Default: 10.
    pub max_failures: Option<usize>,
    /// CI gate over the failure classification (T13). Default:
    /// NewOrRegressed.
    #[serde(default)]
    pub gate: crate::issues::GateSerde,
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
            num_seeds: 4,
            seeds: None,
            duration_s: 5.0,
            parallel,
            minimize_failures: false,
            max_failures: Some(10),
            gate: crate::issues::GateSerde::default(),
            base_scenario: default_chaos_scenario(),
            output_dir: PathBuf::from("bevy_playtest/runs"),
        }
    }
}

impl AutotestConfig {
    /// dbt: release-grade preset — the pre-dbt defaults (16 seeds ×
    /// 20s). Use for nightly runs and release gating, where you want
    /// deeper seed coverage instead of fast `cargo test` feedback.
    pub fn thorough() -> Self {
        Self {
            num_seeds: 16,
            duration_s: 20.0,
            ..Self::default()
        }
    }

    /// Load config with environment overrides applied.
    pub fn from_env() -> Self {
        let mut cfg = Self::default();
        if let Some(seeds) = env_override(ENV_SEEDS).as_deref() {
            if let Ok(n) = seeds.parse::<u64>() {
                cfg.num_seeds = n;
            }
        }
        if let Some(duration) = env_override(ENV_DURATION).as_deref() {
            if let Ok(d) = duration.parse::<f32>() {
                cfg.duration_s = d;
            }
        }
        if let Some(threads) = env_override(ENV_THREADS).as_deref() {
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
    /// Schema version (for tooling compatibility; conventions §4 —
    /// integer, matching `conventions::SCHEMA_VERSION`).
    pub schema_version: u32,
    /// Contract tier exercised, derived from the per-run
    /// `PlaytestReport.scope.tier` (Z1) — the MAXIMUM tier across runs.
    pub contract_tier: crate::contract::ContractTier,
    /// Harness mode (U1 vocabulary: "in_process").
    pub harness_mode: String,
    /// Union of the per-run Z1 `CheckedScope`s.
    pub scope: crate::contract::CheckedScope,
    /// Run budget (seeds, duration, parallelism).
    pub budget: BudgetSummary,
    /// Individual run reports (filtered to failures + last N passes).
    pub runs: Vec<RunSummary>,
    /// Failure fingerprints (deduplicated).
    pub failures: Vec<FingerprintSummary>,
    /// Minimized failures (B2), when config.minimize_failures is on.
    pub minimized: Vec<MinimizedFailureSummary>,
    /// True when the seed loop stopped early at max_failures.
    pub early_stop: bool,
    /// Aggregate coverage (union across runs).
    pub coverage: Option<Coverage>,
    /// Wall-clock timing.
    pub timing: TimingSummary,
    /// FX4.5: stable run identifier (conventions::run_id) — prevents
    /// directory collisions between consecutive autotests.
    pub run_id: Option<String>,
    /// Z4: input surface discovered by differential probing, when
    /// the configuration ran discovery (no IntentSurface).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub discovered_surface: Option<crate::discover::DiscoveredSurface>,
    /// E3: persisted regressions replayed before the seed matrix.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub regressions: Vec<RegressionReplayOutcome>,
    /// T13: new/regressed gate classification + verdict.
    #[serde(default)]
    pub gate: AutotestGateSummary,
}

/// Budget actually spent (formerly `ScopeSummary` — renamed because
/// `scope` is now the Z1 checked-scope union).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BudgetSummary {
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
    /// Per-run violations (rule, detail, fingerprint) so
    /// expectations can use `detail_contains` against the aggregate.
    #[serde(default)]
    pub violations: Vec<RunViolation>,
}

/// Violation summary embedded in a `RunSummary`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RunViolation {
    pub rule: String,
    pub detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<Fingerprint>,
}

/// E3 regression replay outcome inside the autotest report.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RegressionReplayOutcome {
    pub fingerprint: Fingerprint,
    /// "still_failing" | "now_passing"
    pub status: String,
}

/// T13 gate summary embedded in the report.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AutotestGateSummary {
    /// Classification of observed failures.
    pub classification: crate::issues::GateSummary,
    /// Gate verdict: true = the suite passes the configured gate.
    pub passed: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FingerprintSummary {
    pub fingerprint: Fingerprint,
    pub rule: String,
    pub first_seen_seed: u64,
    pub occurrences: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TimingSummary {
    pub start: String,
    pub end: String,
    pub elapsed_ms: u64,
}

/// Result of minimizing one autotest failure (B2): a shrunken input
/// sequence that still reproduces the failure signature.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MinimizedFailureSummary {
    pub fingerprint: Fingerprint,
    pub rule: String,
    pub seed: u64,
    /// Length of the minimal replay-input subsequence (0 = failure
    /// reproduces with no player input at all).
    pub minimal_inputs_len: usize,
    /// smu: the standalone minimal regression scenario (replay bot,
    /// trimmed duration). Written next to the report as
    /// `regression-<seed>-<rule>.json` so it can be committed as a
    /// regression test directly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub regression_scenario: Option<crate::scenario::Scenario>,
}

impl AutotestReport {
    /// cj8.14 (E2): JUnit XML document for CI dashboards. One
    /// `<testcase>` per seed run; failures/errors counted from
    /// statuses. T13 (Z18.G2): fingerprints classified known/muted map
    /// to `<skipped>` with the issue title; new/regressed stay
    /// failures.
    pub fn to_junit(&self, suite_name: &str) -> String {
        let esc = crate::junit::xml_escape;
        let tests = self.runs.len();
        // Runs whose failing fingerprints are all known/muted count
        // as skipped, not failed.
        let muted_run = |r: &RunSummary| -> bool {
            !r.violations.is_empty()
                && r.violations.iter().all(|v| {
                    v.fingerprint.as_ref().is_some_and(|fp| {
                        self.gate.classification.known.contains(fp)
                            || self.gate.classification.muted.contains(fp)
                    })
                })
        };
        let failures = self
            .runs
            .iter()
            .filter(|r| r.status == "fail" && !muted_run(r))
            .count();
        let errors = self
            .runs
            .iter()
            .filter(|r| r.status == "crash" && !muted_run(r))
            .count();
        let skipped = self.runs.iter().filter(|r| muted_run(r)).count();
        let time: f64 = self
            .runs
            .iter()
            .map(|r| r.duration_ms as f64 / 1000.0)
            .sum();
        let mut out = String::new();
        out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
        out.push_str("<testsuites>\n");
        out.push_str(&format!(
            "  <testsuite name=\"{}\" tests=\"{}\" failures=\"{}\" errors=\"{}\" skipped=\"{}\" time=\"{:.3}\">\n",
            esc(suite_name),
            tests,
            failures,
            errors,
            skipped,
            time
        ));
        for r in &self.runs {
            out.push_str(&format!(
                "      <testcase name=\"seed-{}\" classname=\"{}\" time=\"{:.3}\">\n",
                r.seed,
                esc(suite_name),
                r.duration_ms as f64 / 1000.0
            ));
            if muted_run(r) {
                out.push_str("        <skipped message=\"known/muted issue\"/>\n");
            } else {
                match r.status.as_str() {
                    "fail" => {
                        out.push_str(&format!(
                            "        <failure message=\"{} violations\" type=\"assertion\">seed {}\n{}</failure>\n",
                            r.violations_count,
                            r.seed,
                            esc(&r.violations.iter().map(|v| format!("{}: {}", v.rule, v.detail)).collect::<Vec<_>>().join("\n"))
                        ));
                    }
                    "crash" => {
                        out.push_str(&format!(
                            "        <error message=\"crashed after {} frames\" type=\"crash\">seed {}\n</error>\n",
                            r.frame_count, r.seed
                        ));
                    }
                    _ => {}
                }
            }
            out.push_str("      </testcase>\n");
        }
        out.push_str("  </testsuite>\n");
        out.push_str("</testsuites>\n");
        out
    }

    /// Human-readable summary line. Counts FAILING RUNS for the pass
    /// percentage (FX4) and lists New/Regressed fingerprints (Z18.F).
    pub fn summary(&self) -> String {
        let surface_info = match &self.discovered_surface {
            Some(s) => format!(
                ", surface: {} keys [{}]",
                s.keys.len(),
                s.keys
                    .iter()
                    .map(|(k, _)| k.clone())
                    .collect::<Vec<_>>()
                    .join(",")
            ),
            None => String::new(),
        };
        let gate_info = if !self.gate.classification.new.is_empty()
            || !self.gate.classification.regressed.is_empty()
        {
            format!(
                ", NEW: [{}], REGRESSED: [{}]",
                self.gate
                    .classification
                    .new
                    .iter()
                    .map(|f| f.0.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
                self.gate
                    .classification
                    .regressed
                    .iter()
                    .map(|f| f.0.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        } else {
            String::new()
        };
        // RF38: "pass" must never read as "it renders" — headless
        // runs do not verify pixels. Always print the caveat.
        let render_note = if self.scope.inactive.iter().any(|(f, _)| f == "rendering") {
            " [rendering not verified]"
        } else {
            ""
        };
        format!(
            "autotest: {} seeds, {} failing runs ({} unique), {}s total, {}%{}, {}{}{}",
            self.budget.seeds_run,
            self.runs.iter().filter(|r| r.status != "pass").count(),
            self.failures.len(),
            self.timing.elapsed_ms / 1000,
            (100.0
                * (self.budget.seeds_run as f32
                    - self.runs.iter().filter(|r| r.status != "pass").count() as f32)
                / self.budget.seeds_run.max(1) as f32) as u32,
            if self.gate.passed {
                "gate: pass"
            } else {
                "gate: FAIL"
            },
            surface_info,
            gate_info,
            render_note
        )
    }

    /// smu: formatted inline failure summary printed to stderr when
    /// the suite fails. Per-violation details with frames, plus
    /// fingerprint classification and a one-line repro hint — enough
    /// to understand and locally reproduce the failure without
    /// opening report.json.
    pub fn failure_summary(&self) -> String {
        let mut out = String::new();
        let failing_runs = self.runs.iter().filter(|r| r.status != "pass").count();
        out.push_str(&format!(
            "FAILED: {} failing runs ({} unique fingerprints)\n",
            failing_runs,
            self.failures.len()
        ));
        for run in self.runs.iter().filter(|r| r.status != "pass") {
            out.push_str(&format!(
                "seed {}: {} ({} violation{})\n",
                run.seed,
                run.status,
                run.violations_count,
                if run.violations_count == 1 { "" } else { "s" }
            ));
            // Show up to 3 violations inline with detail text.
            for v in run.violations.iter().take(3) {
                let known = if self.gate.classification.known.contains(
                    v.fingerprint
                        .as_ref()
                        .unwrap_or(&Fingerprint(String::new())),
                ) {
                    " [known]"
                } else if self.gate.classification.muted.contains(
                    v.fingerprint
                        .as_ref()
                        .unwrap_or(&Fingerprint(String::new())),
                ) {
                    " [muted]"
                } else if self.gate.classification.regressed.contains(
                    v.fingerprint
                        .as_ref()
                        .unwrap_or(&Fingerprint(String::new())),
                ) {
                    " [REGRESSED]"
                } else {
                    " [NEW]"
                };
                out.push_str(&format!("  [{}] {}{}\n", v.rule, v.detail, known));
            }
            if run.violations.len() > 3 {
                out.push_str(&format!(
                    "  ... and {} more (see report.json)\n",
                    run.violations.len() - 3
                ));
            }
            out.push_str(&format!(
                "  Repro: BEVY_PLAYTEST_SEED={} cargo test --test <your swarm test>\n",
                run.seed
            ));
        }
        out
    }

    /// Whether the entire suite passed the T13 gate: no New/Regressed
    /// fingerprints (known/muted issues do not fail CI) AND every run
    /// passed.
    pub fn passed(&self) -> bool {
        self.gate.passed && self.runs.iter().all(|r| r.status == "pass")
    }
}

/// Run a full autotest suite: N seeds × chaos, aggregating failures,
/// coverage, and writing the report to disk.
///
/// RF8: `Clone` is no longer required — use `autotest_factory` for
/// non-Clone plugins (common for game plugins holding state).
pub fn autotest<P: bevy::prelude::Plugin + Clone + Send + 'static>(
    game_plugin: P,
    config: AutotestConfig,
) -> Result<AutotestReport, Box<dyn std::error::Error>> {
    autotest_factory(move || game_plugin.clone(), config)
}

/// Factory-based autotest — accepts a closure that constructs the
/// plugin, so games without `Clone` work natively. Game-type inference
/// uses the constructed type, preserving Z6 semantics.
pub fn autotest_factory<P, F>(
    make_plugin: F,
    config: AutotestConfig,
) -> Result<AutotestReport, Box<dyn std::error::Error>>
where
    P: bevy::prelude::Plugin + Send + 'static,
    F: Fn() -> P + Send + Sync + 'static,
{
    autotest_inner(make_plugin, config)
}

/// Alias for non-Clone plugins (Z18.H). Unit-struct plugins without
/// `#[derive(Clone)]` use this factory variant.
pub fn autotest_with<P, F>(
    make_plugin: F,
    config: AutotestConfig,
) -> Result<AutotestReport, Box<dyn std::error::Error>>
where
    P: bevy::prelude::Plugin + Send + 'static,
    F: Fn() -> P + Send + Sync + 'static,
{
    autotest_factory::<P, F>(make_plugin, config)
}

fn autotest_inner<P, F>(
    make_plugin: F,
    config: AutotestConfig,
) -> Result<AutotestReport, Box<dyn std::error::Error>>
where
    P: bevy::prelude::Plugin + Send + 'static,
    F: Fn() -> P + Send + Sync + 'static,
{
    let start = Instant::now();
    let start_ts = SystemTime::now();
    let start_iso = crate::conventions::iso_timestamp(start_ts);

    // FX4.5: apply environment overrides on top of the explicit config
    // (BEVY_PLAYTEST_SEEDS / _DURATION / _THREADS win when set).
    let mut config = config;
    if let Some(n) = env_override(ENV_SEEDS).and_then(|v| v.parse::<u64>().ok()) {
        config.num_seeds = n;
    }
    // smu: exact-seed pinning (BEVY_PLAYTEST_SEED=N) — the repro path
    // for `cargo bevy-playtest reproduce` and CI failure hints.
    if let Some(seed) = env_override(ENV_SEED).and_then(|v| v.parse::<u64>().ok()) {
        config.seeds = Some(vec![seed]);
    }
    if let Some(d) = env_override(ENV_DURATION).and_then(|v| v.parse::<f32>().ok()) {
        config.duration_s = d;
    }
    if let Some(th) = env_override(ENV_THREADS).and_then(|v| v.parse::<usize>().ok()) {
        config.parallel = th;
    }

    // Build the headless app factory (Z2): headless_app_factory RETURNS
    // a builder closure; call it per run. Arc-shared so the minimize
    // pass below can reuse the same factory.
    let make_plugin = std::sync::Arc::new(make_plugin);
    let factory = crate::headless::headless_app_factory({
        let make_plugin = make_plugin.clone();
        move || (make_plugin)()
    });

    let runner = InProcess { factory };

    // Z4: discover the input surface when the game has no declared
    // contract (chaos bot without a raw_surface): probing replaces
    // the hard-coded default surface, weighted by effect breadth.
    let discovered_surface = if config.base_scenario.bot.bot_type == crate::enums::BotType::Chaos
        && config.base_scenario.bot.raw_surface.is_none()
    {
        let factory_fn = crate::headless::headless_app_factory({
            let make_plugin = make_plugin.clone();
            move || (make_plugin)()
        });
        let surface =
            crate::discover::discover_surface(factory_fn, &crate::discover::ProbeConfig::default());
        // Feed the discovered surface back into the scenario: chaos
        // samples the discovered (group-completed) keys, weighted.
        // If NOTHING was discovered (no observable game state), keep
        // the FX4 default surface — an empty surface would make chaos
        // refuse (chaos_bot_config) and the run vacuous.
        let non_empty = !surface.keys.is_empty()
            || !surface.mouse_buttons.is_empty()
            || surface.mouse_motion.is_some()
            || surface.wheel.is_some()
            || !surface.gamepad_buttons.is_empty()
            || !surface.gamepad_axes.is_empty();
        if non_empty {
            config.base_scenario.bot.raw_surface =
                Some(crate::scenario::RawSurface::from_discovered(&surface));
        }
        Some(surface)
    } else {
        None
    };

    // Z18.H: explicit seed list overrides num_seeds (expectations pin
    // seeds like [42]).
    let seeds: Vec<u64> = match &config.seeds {
        Some(explicit) => explicit.clone(),
        None => (1..=config.num_seeds).collect(),
    };
    let mut run_summaries: Vec<RunSummary> = Vec::new();
    let mut run_scopes: Vec<crate::contract::CheckedScope> = Vec::new();
    // Z18: every per-seed PlaytestReport, written as seed-<n>.json
    // next to the aggregate report.
    let mut per_seed_reports: Vec<(u64, crate::driver::PlaytestReport)> = Vec::new();
    let mut all_violations: Vec<(u64, crate::state::ViolationEntry)> = Vec::new();
    let mut failures_by_fp: HashMap<Fingerprint, FingerprintSummary> = HashMap::new();
    let mut aggregate_coverage = Coverage::default();
    // Number of failures (non-pass runs + unique fingerprints) seen so
    // far, for max_failures early-stop.
    let mut runs_failed: usize = 0;
    let mut early_stop = false;
    // B2 minimize candidates: (seed, base scenario, full report) of
    // failed runs — the report's action log is the replay material.
    let mut failed_reports: Vec<(u64, Scenario, crate::driver::PlaytestReport)> = Vec::new();

    // E3 (Z18.E): replay persisted regressions FIRST, before the seed
    // matrix — a still-failing known bug must surface even if the new
    // sweep would miss it. Each record is reported as still_failing |
    // now_passing under AutotestReport.regressions.
    let regression_dir = crate::conventions::regressions_dir();
    let mut regressions: Vec<RegressionReplayOutcome> = Vec::new();
    {
        let mut skipped_reg_files = 0u64;
        if let Ok(records) = crate::sweep::load_regressions(&regression_dir, &mut skipped_reg_files)
        {
            for rec in &records {
                let rep = runner.run(&rec.scenario)?;
                let still_failing = !matches!(rep.status, PlaytestStatus::Pass);
                regressions.push(RegressionReplayOutcome {
                    fingerprint: rec.fingerprint.clone(),
                    status: if still_failing {
                        "still_failing".to_string()
                    } else {
                        "now_passing".to_string()
                    },
                });
            }
        }
    }

    // max_failures early-stop: run seeds in batches of config.parallel;
    // check the budget BETWEEN batches (run_matrix itself is
    // fixed-size — a mid-batch abort would needlessly complicate the
    // parallel join).
    let batch_size = config.parallel.max(1);
    let mut batch_start = 0u64;
    while (batch_start as usize) < seeds.len() {
        if let Some(max) = config.max_failures {
            if runs_failed >= max {
                early_stop = true;
                break;
            }
        }
        let batch: Vec<u64> = seeds[batch_start as usize..]
            .iter()
            .take(batch_size)
            .copied()
            .collect();
        // Run seeds in parallel via run_matrix (one scoped thread per
        // scenario, capped by config.parallel; fresh App per run).
        let variants: Vec<(String, Scenario)> = batch
            .iter()
            .map(|seed| {
                let mut scenario = config.base_scenario.clone();
                scenario.bot.seed = *seed;
                scenario.duration_s = config.duration_s;
                (format!("seed-{seed}"), scenario)
            })
            .collect();
        // Keep the seeded scenarios around for B2 minimization (the
        // matrix consumes the variants).
        let seeded: HashMap<u64, Scenario> = batch
            .iter()
            .cloned()
            .zip(variants.iter().map(|(_, s)| s.clone()))
            .collect();
        batch_start += batch.len() as u64;

        let matrix_results = crate::branch::run_matrix(&runner, variants, config.parallel)?;

        // Aggregate results from BranchMatrixReport.
        for outcome in matrix_results.outcomes {
            let seed: u64 = outcome
                .variant_name
                .strip_prefix("seed-")
                .unwrap_or("0")
                .parse()
                .unwrap_or(0);
            let rep = outcome.report;
            // le3.6 (RF2): use wall_time_s (driver-measured) if
            // present, else simulated_duration_s fallback.
            let elapsed_ms = if rep.wall_time_s > 0.0 {
                (rep.wall_time_s * 1000.0) as u64
            } else {
                (rep.simulated_duration_s * 1000.0) as u64
            };

            let status = match rep.status {
                PlaytestStatus::Pass => "pass",
                PlaytestStatus::Fail => "fail",
                PlaytestStatus::Crash => "crash",
            }
            .to_string();
            if status != "pass" {
                runs_failed += 1;
            }

            run_summaries.push(RunSummary {
                seed,
                status: status.clone(),
                violations_count: rep.violations.len(),
                frame_count: rep.frame_count,
                duration_ms: elapsed_ms,
                violations: rep
                    .violations
                    .iter()
                    .map(|v| RunViolation {
                        rule: v.rule.clone(),
                        detail: v.detail.clone(),
                        fingerprint: v.fingerprint.clone(),
                    })
                    .collect(),
            });
            // Z18.B: union of per-run Z1 checked scopes.
            run_scopes.push(rep.scope.clone());
            per_seed_reports.push((seed, rep.clone()));

            // B2 candidates: keep (seed, seeded scenario, full
            // report) for every failed run so minimize_failure can
            // replay the action log at the SAME seed.
            if rep.status != PlaytestStatus::Pass {
                if let Some(scen) = seeded.get(&seed) {
                    failed_reports.push((seed, scen.clone(), rep.clone()));
                }
            }

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
            // Union coverage across runs (intent variants, TestApi paths,
            // reset kinds, chaos surface indices).
            for (k, c) in rep.coverage.intents_emitted {
                *aggregate_coverage.intents_emitted.entry(k).or_default() += c;
            }
            for p in rep.coverage.test_api_paths_read {
                aggregate_coverage.test_api_paths_read.insert(p);
            }
            for k in rep.coverage.resets_invoked {
                aggregate_coverage.resets_invoked.insert(k);
            }
            for i in rep.coverage.chaos_surface_indices {
                aggregate_coverage.chaos_surface_indices.insert(i);
            }
        }
    }

    let end_ts = SystemTime::now();
    let elapsed_ms_total = start.elapsed().as_millis() as u64;

    // minimize_failures (B2): post-matrix, for each FAILED run with
    // replayable inputs, shrink the failure to a minimal regression
    // scenario. Results are folded into the report's minimized field.
    let mut minimized: Vec<MinimizedFailureSummary> = Vec::new();
    if config.minimize_failures {
        for (seed, base_scen, rep) in &failed_reports {
            // headless_app_factory returns an opaque, non-Clone builder —
            // construct a fresh one per minimization (it captures the
            // plugin factory internally).
            let factory = crate::headless::headless_app_factory({
                let make_plugin = make_plugin.clone();
                move || (make_plugin)()
            });
            let outcome = crate::minimize::minimize_failure(factory, base_scen, rep, None);
            if let Ok(out) = outcome {
                // RF16: derive the rule from the minimizer's
                // FailureSignature (violations.first() reports
                // 'bevy_error' for a Crash signature).
                let (fp, rule) = match &out.signature {
                    crate::minimize::FailureSignature::Crash => (
                        rep.violations
                            .first()
                            .and_then(|v| v.fingerprint.clone())
                            .unwrap_or_else(|| Fingerprint(String::new())),
                        "crash".to_string(),
                    ),
                    crate::minimize::FailureSignature::Violation { rule, .. } => (
                        rep.violations
                            .iter()
                            .find(|v| v.rule == *rule)
                            .and_then(|v| v.fingerprint.clone())
                            .unwrap_or_else(|| Fingerprint(String::new())),
                        rule.clone(),
                    ),
                };
                minimized.push(MinimizedFailureSummary {
                    fingerprint: fp,
                    rule,
                    seed: *seed,
                    minimal_inputs_len: out.minimal_inputs.len(),
                    regression_scenario: Some(out.regression_scenario.clone()),
                });
            }
        }
    }

    let failures: Vec<FingerprintSummary> = failures_by_fp.into_values().collect();

    // T13 (Z18.F): classify observed failure fingerprints against the
    // issue DB and apply the configured gate.
    let issue_db = crate::issues::IssueDb::load();
    let observed: Vec<(Fingerprint, u64)> = failures
        .iter()
        .map(|f| (f.fingerprint.clone(), f.first_seen_seed))
        .collect();
    let classification = issue_db.classify(&observed);
    // Z18.F: update-mode write — record observed occurrences so the
    // second consecutive autotest classifies them as known.
    if crate::conventions::can_update("issues") {
        let mut db = issue_db.clone();
        let observed_full: Vec<(Fingerprint, u64, String, u64)> = failures
            .iter()
            .map(|f| {
                (
                    f.fingerprint.clone(),
                    f.first_seen_seed,
                    f.rule.clone(),
                    f.occurrences,
                )
            })
            .collect();
        db.record(&observed_full);
        db.save()?;
    }
    let gate_enum: crate::issues::Gate = config.gate.into();
    let gate_passed = gate_enum.apply(&classification);
    let gate = AutotestGateSummary {
        classification,
        passed: gate_passed,
    };

    // Z18.B: contract tier from the per-run scopes (never hardcoded).
    let scope_union = crate::contract::CheckedScope::union(run_scopes);
    let contract_tier = scope_union.tier.unwrap_or_default();

    let report = AutotestReport {
        schema_version: crate::conventions::SCHEMA_VERSION,
        contract_tier,
        harness_mode: "in_process".to_string(),
        scope: scope_union,
        budget: BudgetSummary {
            seeds_run: run_summaries.len() as u64,
            total_duration_s: config.duration_s * run_summaries.len() as f32,
            parallelism: config.parallel,
        },
        runs: run_summaries.clone(),
        failures,
        minimized,
        early_stop,
        coverage: Some(aggregate_coverage),
        timing: TimingSummary {
            start: start_iso,
            end: crate::conventions::iso_timestamp(end_ts),
            elapsed_ms: elapsed_ms_total,
        },
        run_id: Some(crate::conventions::run_id(
            elapsed_ms_total,
            run_summaries.len() as u64,
        )),
        discovered_surface,
        regressions,
        gate,
    };

    // Write report to disk. Z18: per-seed PlaytestReports are written
    // alongside the aggregate so expectations can check each seed.
    write_report(
        &report,
        &config.output_dir,
        per_seed_reports
            .iter()
            .map(|(seed, rep)| (*seed, rep.clone()))
            .collect(),
    )?;

    Ok(report)
}

fn write_report(
    report: &AutotestReport,
    output_dir: &Path,
    per_seed: Vec<(u64, crate::driver::PlaytestReport)>,
) -> Result<(), Box<dyn std::error::Error>> {
    // smu: inline failure summary to stderr BEFORE the report is
    // written — a failing CI run's console output alone must suffice
    // to understand and locally reproduce the failure.
    if !report.passed() {
        eprint!("{}", report.failure_summary());
    }
    // Z18.B: honour CARGO_TARGET_DIR (U1 layout: runs/<run-id>/, no
    // autotest- prefix — the run id itself identifies the mode).
    let target_dir = std::env::var("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("target"));
    let runs_root = target_dir.join(output_dir);
    let run_dir = runs_root.join(
        report
            .run_id
            .clone()
            .unwrap_or_else(|| crate::conventions::run_id(0, report.timing.elapsed_ms)),
    );
    std::fs::create_dir_all(&run_dir)?;

    let report_path = run_dir.join("report.json");
    let json = serde_json::to_string_pretty(report)?;
    std::fs::write(&report_path, json)?;

    // Z18: per-seed PlaytestReports — one seed-<n>.json per run.
    for (seed, rep) in per_seed {
        let path = run_dir.join(format!("seed-{seed}.json"));
        std::fs::write(path, serde_json::to_string_pretty(&rep)?)?;
    }

    // smu: minimized failures emit standalone runnable regression
    // scenarios next to the report (not only embedded in report JSON) —
    // commit them as regression tests directly.
    for m in &report.minimized {
        if let Some(scenario) = &m.regression_scenario {
            let safe_rule: String = m
                .rule
                .chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                        c
                    } else {
                        '-'
                    }
                })
                .collect();
            let path = run_dir.join(format!("regression-{}-{safe_rule}.json", m.seed));
            std::fs::write(path, serde_json::to_string_pretty(scenario)?)?;
        }
    }

    // cj8.14 (E2): JUnit XML for CI dashboards, per U1 convention
    // runs/<run-id>/junit.xml.
    let junit = report.to_junit("bevy_playtest.autotest");
    std::fs::write(run_dir.join("junit.xml"), junit)?;

    // Conventions §2: write a `last` pointer to the latest run.
    let last_pointer = runs_root.join("last");
    let _ = std::fs::write(
        &last_pointer,
        run_dir
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .as_bytes(),
    );

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
            runs: vec![RunSummary {
                seed: 1,
                status: "fail".to_string(),
                violations_count: 1,
                frame_count: 10,
                duration_ms: 500,
                violations: vec![],
            }],
            schema_version: crate::conventions::SCHEMA_VERSION,
            contract_tier: crate::contract::ContractTier::Zero,
            harness_mode: "in_process".to_string(),
            scope: crate::contract::CheckedScope::default(),
            budget: BudgetSummary {
                seeds_run: 10,
                total_duration_s: 200.0,
                parallelism: 4,
            },
            failures: vec![FingerprintSummary {
                fingerprint: Fingerprint("test-fp".to_string()),
                rule: "panic".to_string(),
                first_seen_seed: 1,
                occurrences: 3,
            }],
            minimized: vec![],
            early_stop: false,
            coverage: Some(Coverage::default()),
            timing: TimingSummary {
                start: "".to_string(),
                end: "".to_string(),
                elapsed_ms: 5000,
            },
            run_id: None,
            discovered_surface: None,
            regressions: vec![],
            gate: AutotestGateSummary {
                classification: crate::issues::GateSummary::default(),
                passed: true,
            },
        };

        let summary = report.summary();
        assert!(summary.contains("10 seeds"));
        assert!(summary.contains("(1 unique)"));
        assert!(summary.contains("5s total"));
        // Z10: passed() = all runs pass + no failures recorded.
        assert!(!report.passed());
        let passing = AutotestReport {
            runs: vec![RunSummary {
                seed: 1,
                status: "pass".to_string(),
                violations_count: 0,
                frame_count: 10,
                duration_ms: 1,
                violations: vec![],
            }],
            failures: vec![],
            ..report
        };
        assert!(passing.passed());
        let crashed = AutotestReport {
            runs: vec![RunSummary {
                seed: 2,
                status: "crash".to_string(),
                violations_count: 1,
                frame_count: 5,
                duration_ms: 1,
                violations: vec![],
            }],
            ..passing
        };
        assert!(!crashed.passed());
    }

    /// smu: failure_summary format — violations inline with detail
    /// text and a repro hint per failing run.
    #[test]
    fn autotest_report_failure_summary_format() {
        let report = AutotestReport {
            runs: vec![RunSummary {
                seed: 42,
                status: "fail".to_string(),
                violations_count: 2,
                frame_count: 214,
                duration_ms: 5000,
                violations: vec![
                    RunViolation {
                        rule: "stuck_after_intent".to_string(),
                        detail: "no liveness within 0.5s after intent choice#2".to_string(),
                        fingerprint: None,
                    },
                    RunViolation {
                        rule: "bounds_violation".to_string(),
                        detail: "Entity \"player\" translation.y = -15.3 (min: -10.0)".to_string(),
                        fingerprint: None,
                    },
                ],
            }],
            schema_version: crate::conventions::SCHEMA_VERSION,
            contract_tier: crate::contract::ContractTier::Zero,
            harness_mode: "in_process".to_string(),
            scope: crate::contract::CheckedScope::default(),
            budget: BudgetSummary {
                seeds_run: 4,
                total_duration_s: 20.0,
                parallelism: 1,
            },
            failures: vec![],
            minimized: vec![],
            early_stop: false,
            coverage: None,
            timing: TimingSummary {
                start: "".to_string(),
                end: "".to_string(),
                elapsed_ms: 5000,
            },
            run_id: None,
            discovered_surface: None,
            regressions: vec![],
            gate: AutotestGateSummary {
                classification: crate::issues::GateSummary::default(),
                passed: false,
            },
        };
        let s = report.failure_summary();
        assert!(s.contains("FAILED: 1 failing runs"), "{s}");
        assert!(s.contains("seed 42: fail (2 violations)"), "{s}");
        assert!(
            s.contains("[stuck_after_intent] no liveness within 0.5s"),
            "{s}"
        );
        assert!(
            s.contains("[bounds_violation] Entity \"player\" translation.y = -15.3"),
            "{s}"
        );
        assert!(s.contains("Repro: BEVY_PLAYTEST_SEED=42"), "{s}");
    }
}
