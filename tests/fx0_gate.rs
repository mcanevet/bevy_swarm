//! FX0 gate: acceptance suite for v0.2/v0.3 trustworthiness (in-process).
//!
//! - Clean fixtures pass at duration_s >= 6.0 on seeds 1..8 (no false positives)
//! - Bug fixtures are detected by their specific signal (must_report rules)
//!   across seeds 1..8 — no seed cherry-picking (xd5)
//! - Bug signals do NOT fire with FIXTURE_BUG unset (FX2 guard)
//! - autotest() runs bots + oracles (frames > 0, non-empty coverage)
//!
//! All sweeps run IN-PROCESS via ScenarioRunner (seconds, not minutes).
//! NOTE: tests must run single-threaded (--test-threads=1) to avoid
//! FIXTURE_BUG env-var races across parallel threads.
//! One subprocess CLI test in tests/acceptance.rs keeps report-file/
//! CLI fidelity coverage.
//!
//! Known gaps (seed-dependent misses tracked by follow-up beads):
//! - walker/bug_dead_left_key: seed 5 misses dead_verb (swarm-lgn)

use serde_json::Value;
use serial_test::serial;
use std::path::PathBuf;

use bevy::prelude::*;
use bevy_swarm::branch::{InProcess, ScenarioRunner};
use bevy_swarm::contract::*;
use bevy_swarm::harness::Scenario;
use fixture_regression_pair::RegressionPairGamePlugin;
use fixture_runner::{
    headless_app, RegressionPairAdapterPlugin, SpawnerAdapterPlugin, SpinnerAdapterPlugin,
    TurnBasedAdapterPlugin, WalkerAdapterPlugin,
};
use fixture_spawner::SpawnerGamePlugin;
use fixture_turn_based::TurnBasedGamePlugin;
use fixture_walker::WalkerGamePlugin;

const SEEDS: &[u64] = &[
    1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26,
    27, 28, 29, 30, 31, 32,
];
const DUR_S: f64 = 6.0;

fn expectations_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/swarm/expectations")
}

fn discover_fixtures() -> Vec<String> {
    let root = expectations_dir();
    let mut out: Vec<_> = std::fs::read_dir(&root)
        .expect("expectations dir exists")
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    out.sort();
    out
}

fn load_expectation(fixture: &str, variant: &str) -> Value {
    let path = expectations_dir()
        .join(fixture)
        .join(format!("{}.json", variant));
    let raw =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {}", path.display(), e));
    // le3.20 (RF24): typed parse first — unknown keys (typos like
    // `must_reprot`) must REJECT, not silently skip. Falls back to
    // Value only for pending-stub cases.
    let typed: Result<TypedExpectation, _> = serde_json::from_str(&raw);
    if let Err(e) = typed {
        panic!(
            "{}: expectation schema violation (unknown or mistyped key?): {}",
            path.display(),
            e
        );
    }
    serde_json::from_str(&raw).unwrap_or_else(|e| panic!("parse {}: {}", path.display(), e))
}

/// le3.20 (RF24): typed expectation — a typo'd key like `must_reprot`
/// previously deserialized fine (extra keys ignored) and the check
/// silently skipped. deny_unknown_fields makes typos loud.
/// Fields exist for schema validation only; readers use the Value form.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[expect(dead_code)]
struct TypedExpectation {
    #[serde(default)]
    schema_version: Option<u64>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    scaffold: Option<String>,
    #[serde(default)]
    seeds: Option<Vec<u64>>,
    #[serde(default)]
    duration_s: Option<f64>,
    #[serde(default)]
    invariants: Option<Value>,
    #[serde(default)]
    must_report: Option<Vec<TypedMustReport>>,
    #[serde(default)]
    must_not_report: Option<Vec<TypedMustReport>>,
    #[serde(default)]
    liveness: Option<Value>,
    #[serde(default)]
    bot: Option<Value>,
    #[serde(default)]
    input_rate_hz: Option<u64>,
    #[serde(default)]
    check_determinism: Option<u64>,
    #[serde(default)]
    pending: Option<Value>,
    /// Regressions carry arbitrary metadata — allowed explicitly.
    #[serde(default)]
    metadata: Option<Value>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[expect(dead_code)]
struct TypedMustReport {
    rule: String,
    #[serde(default)]
    detail_contains: Option<String>,
    #[serde(default)]
    target: Option<String>,
    #[serde(default)]
    min_count: Option<u64>,
}

/// Build a scenario JSON honouring the expectation's overrides
/// (bot, invariants, liveness, input_rate_hz) at a given seed.
fn build_scenario_json(fixture: &str, variant: &str, seed: u64) -> String {
    let exp = load_expectation(fixture, variant);
    let mut scenario = serde_json::json!({
        "bot": { "type": "chaos", "seed": seed },
        "duration_s": exp.get("duration_s").and_then(|d| d.as_f64()).unwrap_or(DUR_S),
        "invariants": exp.get("invariants").cloned().unwrap_or(serde_json::json!([])),
        "setup": {}
    });
    if let Some(liveness) = exp.get("liveness") {
        scenario["liveness"] = liveness.clone();
    }
    if let Some(bot) = exp.get("bot") {
        scenario["bot"] = bot.clone();
    }
    if let Some(rate) = exp.get("input_rate_hz").and_then(|r| r.as_u64()) {
        scenario["bot"]["input_rate_hz"] = serde_json::json!(rate);
    }
    if let Some(check_det) = exp.get("check_determinism").and_then(|d| d.as_u64()) {
        scenario["check_determinism"] = serde_json::json!(check_det);
    }
    scenario.to_string()
}

/// Build the in-process app for a fixture, mirroring fixture-runner's
/// main.rs wiring (adapters + IntentSurface + game plugin).
fn build_app_for(fixture: &str) -> App {
    let mut app = headless_app();
    app.add_plugins(TestConventionsPlugin);
    app.add_plugins(bevy_swarm::driver::PlaytestPlugin);
    match fixture {
        "spinner" => {
            app.insert_resource(IntentSurface::new(vec![SurfaceVariant::Wait]));
            app.add_plugins(SpinnerAdapterPlugin);
            app.add_plugins(fixture_spinner::SpinnerGamePlugin);
        }
        "walker" => {
            // le3.33 (RF43): Wait restored — the RF27 removal masked
            // the frozen_frames/timeout accounting bug.
            app.insert_resource(IntentSurface::new(vec![
                SurfaceVariant::Move,
                SurfaceVariant::Wait,
            ]));
            app.add_plugins(WalkerAdapterPlugin);
            app.add_plugins(WalkerGamePlugin);
        }
        "spawner" => {
            app.insert_resource(IntentSurface::new(vec![SurfaceVariant::Wait]));
            app.add_plugins(SpawnerAdapterPlugin);
            app.add_plugins(SpawnerGamePlugin);
        }
        "turn_based" => {
            app.insert_resource(IntentSurface::new(vec![
                SurfaceVariant::Choice(3),
                SurfaceVariant::Wait,
            ]));
            app.add_plugins(TurnBasedAdapterPlugin);
            app.add_plugins(TurnBasedGamePlugin);
        }
        "regression_pair" => {
            app.insert_resource(IntentSurface::new(vec![SurfaceVariant::Wait]));
            app.add_plugins(RegressionPairAdapterPlugin);
            app.add_plugins(RegressionPairGamePlugin);
        }
        _ => panic!("unknown fixture: {}", fixture),
    }
    app
}

/// RAII guard that saves the prior value of `FIXTURE_BUG` and restores
/// it on scope exit — including panics inside the factory/runner, which
/// would otherwise leak a set env var into subsequent tests.
struct FixtureBugGuard {
    prev: Option<String>,
}

impl FixtureBugGuard {
    fn set(env_bug: Option<&str>) -> Self {
        let prev = std::env::var("FIXTURE_BUG").ok();
        match env_bug {
            Some(b) => std::env::set_var("FIXTURE_BUG", b),
            None => std::env::remove_var("FIXTURE_BUG"),
        }
        Self { prev }
    }
}

impl Drop for FixtureBugGuard {
    fn drop(&mut self) {
        match self.prev.take() {
            Some(p) => std::env::set_var("FIXTURE_BUG", p),
            None => std::env::remove_var("FIXTURE_BUG"),
        }
    }
}

/// Run one fixture variant in-process. When `enable_bug` is true and the
/// variant is a bug_* case, FIXTURE_BUG is set for the run.
fn run_fixture(
    fixture: &str,
    variant: &str,
    scenario_json: &str,
    enable_bug: bool,
) -> bevy_swarm::driver::PlaytestReport {
    let scenario: Scenario = serde_json::from_str(scenario_json).expect("parse scenario");
    let fixture = fixture.to_string();
    let mut env_bug = None;
    if enable_bug {
        // Z18: scaffold:none copies are suffixed _none; drop suffix.
        let bug = variant
            .strip_prefix("bug_")
            .and_then(|s| s.strip_suffix("_none"))
            .or_else(|| variant.strip_prefix("bug_"));
        if let Some(bug) = bug {
            env_bug = Some(bug.to_string());
        }
    }
    // Env-var communication with the fixture game plugin: fixtures read
    // FIXTURE_BUG at plugin-build time, so set/remove around the run.
    // Drop guard ensures restoration even if the factory panics.
    let _guard = FixtureBugGuard::set(env_bug.as_deref());
    let runner = InProcess {
        factory: move || build_app_for(&fixture),
    };
    let result = ScenarioRunner::run(&runner, &scenario);
    result.expect("run scenario")
}

fn violation_rules(report: &bevy_swarm::driver::PlaytestReport) -> Vec<String> {
    report.violations.iter().map(|v| v.rule.clone()).collect()
}

fn check_clean_pass(report: &bevy_swarm::driver::PlaytestReport, seed: u64) -> Vec<String> {
    let mut errors = Vec::new();
    if report.status != bevy_swarm::enums::PlaytestStatus::Pass {
        errors.push(format!(
            "clean seed {} expected pass, got {:?}",
            seed, report.status
        ));
    }
    if !report.violations.is_empty() {
        errors.push(format!(
            "clean seed {} has {} violations: {:?}",
            seed,
            report.violations.len(),
            violation_rules(report)
        ));
    }
    errors
}

fn check_bug_signal(
    report: &bevy_swarm::driver::PlaytestReport,
    expect: &Value,
    seed: u64,
) -> Vec<String> {
    let mut errors = Vec::new();
    let violation_objects: Vec<&bevy_swarm::state::ViolationEntry> =
        report.violations.iter().collect();
    if let Some(must) = expect.get("must_report").and_then(|m| m.as_array()) {
        for entry in must {
            let rule = entry
                .get("rule")
                .and_then(|r| r.as_str())
                .expect("rule name");
            // Match rule name first.
            let matching: Vec<_> = violation_objects
                .iter()
                .filter(|v| v.rule == rule)
                .collect();
            if matching.is_empty() {
                errors.push(format!(
                    "bug seed {} missing must_report rule '{}'",
                    seed, rule
                ));
                continue;
            }
            // detail_contains filter (same as acceptance).
            if let Some(want) = entry.get("detail_contains").and_then(|d| d.as_str()) {
                if !matching.iter().any(|v| v.detail.contains(want)) {
                    errors.push(format!(
                        "bug seed {}: must_report '{}' reported but no detail contains {:?}",
                        seed, rule, want
                    ));
                }
            }
            // target filter (same as acceptance).
            if let Some(target) = entry.get("target").and_then(|d| d.as_str()) {
                if !matching.iter().any(|v| v.target == target) {
                    errors.push(format!(
                        "bug seed {}: must_report '{}' reported but not on target {:?}",
                        seed, rule, target
                    ));
                }
            }
        }
    }
    errors
}

#[serial]
#[test]
fn fx0_fixture_bug_env_restored_after_factory_panic() {
    // Factory panic (unknown fixture) while a bug env var is set:
    // the Drop guard must restore a clean env.
    std::env::remove_var("FIXTURE_BUG");
    let scenario = build_scenario_json("walker", "bug_dead_left_key", 1);
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run_fixture("no_such_fixture", "bug_dead_left_key", &scenario, true)
    }));
    assert!(
        caught.is_err(),
        "factory panic must unwind through run_fixture"
    );

    assert!(
        std::env::var("FIXTURE_BUG").is_err(),
        "FIXTURE_BUG must be unset after a panicking run"
    );

    // Clean-variant path (bug unset going in) also ends clean.
    let clean_scenario = build_scenario_json("walker", "clean", 1);
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run_fixture("no_such_fixture", "clean", &clean_scenario, false)
    }));
    assert!(
        caught.is_err(),
        "factory panic must unwind through run_fixture"
    );

    assert!(
        std::env::var("FIXTURE_BUG").is_err(),
        "FIXTURE_BUG must stay unset after a panicking clean run"
    );
}

#[serial]
#[test]
fn fx0_clean_fixtures_pass_seeds_1_to_8() {
    let mut total_errors = Vec::new();
    for fixture in discover_fixtures() {
        let exp = load_expectation(&fixture, "clean");
        if exp.get("status").and_then(|s| s.as_str()) != Some("pass") {
            continue;
        }
        for seed in SEEDS {
            let scenario = build_scenario_json(&fixture, "clean", *seed);
            let report = run_fixture(&fixture, "clean", &scenario, false);
            total_errors.extend(check_clean_pass(&report, *seed));
        }
    }
    assert!(
        total_errors.is_empty(),
        "clean fixture failures:\n{}",
        total_errors.join("\n")
    );
}

#[serial]
#[test]
fn fx0_bug_fixtures_emit_specific_signals_seeds_1_to_8() {
    let mut total_errors = Vec::new();
    let mut summary: Vec<String> = Vec::new();
    for fixture in discover_fixtures() {
        let variants: Vec<String> = std::fs::read_dir(expectations_dir().join(&fixture))
            .expect("fixture dir readable")
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                p.extension().is_some_and(|x| x == "json")
                    && p.file_stem()
                        .is_some_and(|s| s.to_string_lossy().starts_with("bug_"))
            })
            .map(|p| p.file_stem().unwrap().to_string_lossy().to_string())
            .collect();
        for variant in variants {
            let exp = load_expectation(&fixture, &variant);
            if exp.get("pending").is_some() {
                continue;
            }
            // le3.20 (RF24): scaffold:none cases run the PLAIN game via
            // the fixture-runner binary in the acceptance suite; the
            // in-process gate cannot reproduce that build (no adapter,
            // autotest tier), so they are skipped here — covered by
            // acceptance instead of silently run against the WRONG build.
            if variant.ends_with("_none") {
                continue;
            }
            if exp.get("pending").is_some() {
                continue;
            }
            let mut hits = 0;
            let mut total = 0;
            let mut misses: Vec<String> = Vec::new();
            for seed in SEEDS {
                let scenario = build_scenario_json(&fixture, &variant, *seed);
                let report = run_fixture(&fixture, &variant, &scenario, true);
                let errors = check_bug_signal(&report, &exp, *seed);
                total += 1;
                if errors.is_empty() {
                    hits += 1;
                } else {
                    misses.push(format!("seed {} ({})", seed, errors[0]));
                    total_errors.extend(errors);
                }
            }
            summary.push(format!(
                "{}/{}: {}/{} seeds detected (misses: {})",
                fixture,
                variant,
                hits,
                total,
                misses.join(", ")
            ));
        }
    }
    eprintln!("=== bug detection rates (seeds 1..8) ===");
    for line in &summary {
        eprintln!("{}", line);
    }
    // 6wt: per-seed detection floor — a variant detected on ZERO seeds
    // must fail loudly even if errors were somehow empty; and any
    // variant below the floor is reported explicitly.
    let mut rate_errors: Vec<String> = Vec::new();
    for line in &summary {
        // Format: "fixture/variant: H/T seeds detected (...)"
        if let Some(rest) = line.split(": ").nth(1) {
            if let Some((ht, _)) = rest.split_once(' ') {
                if let Some((hits, total)) = ht.split_once('/') {
                    let (hits, total): (u64, u64) =
                        (hits.parse().unwrap_or(0), total.parse().unwrap_or(1));
                    if total > 0 && hits * 2 < total {
                        rate_errors.push(format!(
                            "{line} — detection rate {}/{} below floor",
                            hits, total
                        ));
                    }
                }
            }
        }
    }
    total_errors.extend(rate_errors);

    assert!(
        total_errors.is_empty(),
        "bug fixture signal failures:\n{}",
        total_errors.join("\n")
    );
}

/// FX2 guard: each bug_* case must NOT fire its must_report signal
/// when FIXTURE_BUG is unset.
#[serial]
#[test]
fn fx0_bug_signals_do_not_fire_with_bug_disabled() {
    let mut total_errors = Vec::new();
    for fixture in discover_fixtures() {
        let variants: Vec<String> = std::fs::read_dir(expectations_dir().join(&fixture))
            .expect("fixture dir readable")
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                p.extension().is_some_and(|x| x == "json")
                    && p.file_stem()
                        .is_some_and(|s| s.to_string_lossy().starts_with("bug_"))
            })
            .map(|p| p.file_stem().unwrap().to_string_lossy().to_string())
            .collect();
        for variant in variants {
            let exp = load_expectation(&fixture, &variant);
            if exp.get("pending").is_some() {
                continue;
            }
            // le3.20 (RF24): scaffold:none cases run the PLAIN game via
            // the fixture-runner binary in the acceptance suite; the
            // in-process gate cannot reproduce that build (no adapter,
            // autotest tier), so they are skipped here — covered by
            // acceptance instead of silently run against the WRONG build.
            if variant.ends_with("_none") {
                continue;
            }
            if exp.get("pending").is_some() {
                continue;
            }
            let must: Vec<(String, Option<u64>)> = exp
                .get("must_report")
                .and_then(|m| m.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|e| {
                            let rule = e.get("rule")?.as_str()?;
                            let min_count = e.get("min_count").and_then(|m| m.as_u64());
                            Some((rule.to_string(), min_count))
                        })
                        .collect()
                })
                .unwrap_or_default();
            if must.is_empty() {
                continue;
            }
            // le3.7 (RF3): guard must use the BUG VARIANT'S OWN
            // scenario (invariants, bot override, liveness timeout,
            // check_determinism) with FIXTURE_BUG unset, across seeds
            // 1..32. If the bug's invariants fire on clean, the bug is
            // vacuous (clean == bug).
            for seed in SEEDS {
                let scenario = build_scenario_json(&fixture, &variant, *seed);
                let report = run_fixture(&fixture, &variant, &scenario, false);
                for (rule, min_count) in &must {
                    // le3.20 (RF24): use the SAME counting as acceptance —
                    // report_named merges repeats per (rule, target) into
                    // one entry whose `count` field holds the number of
                    // occurrences. Counting array ENTRIES capped the clean
                    // game at 1, making the min_count guard unfalsifiable.
                    let occurrences: u64 = report
                        .violations
                        .iter()
                        .filter(|v| v.rule == *rule)
                        .map(|v| v.count)
                        .max()
                        .unwrap_or(0);
                    let allowed_on_clean = min_count.unwrap_or(1);
                    if occurrences >= allowed_on_clean {
                        total_errors.push(format!(
                            "{}/{}/{}: rule '{}' fired {}x with bug disabled (>= {allowed_on_clean})",
                            fixture, variant, seed, rule, occurrences
                        ));
                    }
                }
            }
        }
    }
    assert!(
        total_errors.is_empty(),
        "bug signals fired with FIXTURE_BUG unset:\n{}",
        total_errors.join("\n")
    );
}

#[serial]
#[test]
fn fx0_autotest_coverage_nonzero() {
    use bevy_swarm::autotest::{autotest, AutotestConfig};

    let config = AutotestConfig::default();
    let report = autotest(WalkerGamePlugin, config).expect("autotest runs");
    assert!(
        report.runs.iter().any(|r| r.frame_count > 0),
        "autotest must run frames > 0"
    );
    assert!(report.coverage.is_some(), "autotest must produce coverage");
    let cov = report.coverage.as_ref().unwrap();
    let non_empty = !cov.intents_emitted.is_empty()
        || !cov.test_api_paths_read.is_empty()
        || !cov.resets_invoked.is_empty()
        || !cov.chaos_surface_indices.is_empty();
    assert!(non_empty, "coverage must be non-empty");
}
