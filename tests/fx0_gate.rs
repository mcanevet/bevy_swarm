//! FX0 gate: acceptance suite for v0.2/v0.3 trustworthiness.
//!
//! - Clean fixtures pass at duration_s >= 6.0 on seeds 1..8 (no false positives)
//! - Bug fixtures are detected by their specific signal (must_report rules)
//! - autotest() runs bots + oracles (frames > 0, non-empty coverage)

use serde_json::Value;
use std::path::PathBuf;
use std::process::Command;

const SEEDS: &[u64] = &[1, 2, 3, 4, 5, 6, 7, 8];
const DUR_S: f64 = 6.0;

fn expectations_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/swarm/expectations")
}

fn discover_fixtures() -> Vec<String> {
    let root = expectations_dir();
    std::fs::read_dir(&root)
        .expect("expectations dir exists")
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect()
}

fn load_expectation(fixture: &str, variant: &str) -> Value {
    let path = expectations_dir()
        .join(fixture)
        .join(format!("{}.json", variant));
    let raw = std::fs::read_to_string(&path).expect("read expectation");
    serde_json::from_str(&raw).expect("parse expectation")
}

fn build_scenario_json(fixture: &str, variant: &str, seed: u64) -> String {
    let exp = load_expectation(fixture, variant);
    let mut scenario = serde_json::json!({
        "bot": { "type": "chaos", "seed": seed },
        "duration_s": DUR_S,
        "invariants": exp.get("invariants").cloned().unwrap_or(serde_json::json!([])),
        "setup": {}
    });
    if let Some(bot) = exp.get("bot") {
        scenario["bot"] = bot.clone();
    }
    if let Some(liveness) = exp.get("liveness") {
        scenario["liveness"] = liveness.clone();
    }
    if let Some(rate) = exp.get("input_rate_hz").and_then(|r| r.as_u64()) {
        scenario["bot"]["input_rate_hz"] = serde_json::json!(rate);
    }
    scenario.to_string()
}

fn run_fixture(fixture: &str, variant: &str, scenario_json: &str, out_path: &PathBuf) -> Value {
    run_fixture_opt(fixture, variant, scenario_json, out_path, true)
}

/// `enable_bug: false` runs the fixture WITHOUT FIXTURE_BUG — used by
/// the false-positive guard (a bug_* expectation must not fire when the
/// bug is disabled).
fn run_fixture_opt(
    fixture: &str,
    variant: &str,
    scenario_json: &str,
    out_path: &PathBuf,
    enable_bug: bool,
) -> Value {
    let mut cmd = Command::new("cargo");
    cmd.args(["run", "--quiet", "-p", "fixture-runner", "--", fixture])
        .arg(scenario_json)
        .arg("--out")
        .arg(out_path)
        .current_dir(env!("CARGO_MANIFEST_DIR"));

    if enable_bug {
        if let Some(bug) = variant.strip_prefix("bug_") {
            cmd.env("FIXTURE_BUG", bug);
        }
    }

    let output = cmd.output().expect("run fixture-runner");
    if !output.status.success() {
        panic!(
            "fixture-runner failed for {}: {}",
            fixture,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let json = std::fs::read_to_string(out_path)
        .unwrap_or_else(|e| panic!("read report {}: {}", out_path.display(), e));
    serde_json::from_str(&json).expect("valid JSON")
}

fn check_clean_pass(report: &Value, _fixture: &str, seed: u64) -> Vec<String> {
    let mut errors = Vec::new();
    let status = report
        .get("status")
        .and_then(|s| s.as_str())
        .unwrap_or("<missing>");
    if status != "pass" {
        errors.push(format!("clean seed {} expected pass, got {}", seed, status));
    }
    let violations = report
        .get("violations")
        .and_then(|v| v.as_array())
        .map(|arr| arr.len())
        .unwrap_or(0);
    if violations > 0 {
        errors.push(format!("clean seed {} has {} violations", seed, violations));
    }
    errors
}

fn check_bug_signal(report: &Value, expect: &Value, _fixture: &str, seed: u64) -> Vec<String> {
    let mut errors = Vec::new();
    let must = expect.get("must_report").and_then(|m| m.as_array());
    if let Some(rules) = must {
        let violation_rules: Vec<&str> = report
            .get("violations")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.get("rule").and_then(|r| r.as_str()))
                    .collect()
            })
            .unwrap_or_default();
        for entry in rules {
            let rule = entry
                .get("rule")
                .and_then(|r| r.as_str())
                .expect("rule name");
            if !violation_rules.contains(&rule) {
                errors.push(format!(
                    "bug seed {} missing must_report rule '{}'",
                    seed, rule
                ));
            }
        }
    }
    errors
}

#[test]
fn fx0_clean_fixtures_pass_seeds_1_to_8() {
    let fixtures = discover_fixtures();
    let target = std::env::var("CARGO_TARGET_DIR").unwrap_or_else(|_| "target".into());
    let out_base = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(&target)
        .join("bevy_swarm/runs/fx0_gate");
    std::fs::create_dir_all(&out_base).expect("create out dir");

    let mut total_errors = Vec::new();
    for fixture in &fixtures {
        let exp = load_expectation(fixture, "clean");
        if exp.get("status").and_then(|s| s.as_str()) != Some("pass") {
            continue; // skip non-clean expectations
        }
        for seed in SEEDS {
            let scenario = build_scenario_json(fixture, "clean", *seed);
            let case_id = format!("{}-clean-seed{}", fixture, seed);
            let out_path = out_base.join(format!("{}_report.json", case_id));
            let report = run_fixture(fixture, "clean", &scenario, &out_path);
            let errors = check_clean_pass(&report, fixture, *seed);
            if !errors.is_empty() {
                total_errors.extend(errors);
            }
        }
    }
    assert!(
        total_errors.is_empty(),
        "clean fixture failures:\n{}",
        total_errors.join("\n")
    );
}

#[test]
fn fx0_bug_fixtures_emit_specific_signals_seeds_1_to_8() {
    let fixtures = discover_fixtures();
    let target = std::env::var("CARGO_TARGET_DIR").unwrap_or_else(|_| "target".into());
    let out_base = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(&target)
        .join("bevy_swarm/runs/fx0_gate_bug");
    std::fs::create_dir_all(&out_base).expect("create out dir");

    let mut total_errors = Vec::new();
    for fixture in &fixtures {
        let variants: Vec<String> = std::fs::read_dir(expectations_dir().join(fixture))
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
            let exp = load_expectation(fixture, &variant);
            // Cases marked "pending" point at an OPEN follow-up bead
            // (see tests/swarm/pending.json) — the bug is a known gap,
            // tracked separately. Skip here; the acceptance suite
            // enforces strict xfail semantics for them.
            if exp.get("pending").is_some() {
                continue;
            }
            // Bug fixtures run on their pinned expectation seeds (detection
            // can be chaos-path dependent; the seed sweep is for clean
            // fixtures only).
            let pinned: Vec<u64> = exp
                .get("seeds")
                .and_then(|s| s.as_array())
                .map(|a| a.iter().filter_map(|v| v.as_u64()).collect())
                .unwrap_or_else(|| vec![42]);
            for seed in pinned {
                let scenario = build_scenario_json(fixture, &variant, seed);
                let case_id = format!("{}-{}-seed{}", fixture, variant, seed);
                let out_path = out_base.join(format!("{}_report.json", case_id));
                let report = run_fixture(fixture, &variant, &scenario, &out_path);
                let errors = check_bug_signal(&report, &exp, fixture, seed);
                if !errors.is_empty() {
                    total_errors.extend(errors);
                }
            }
        }
    }
    assert!(
        total_errors.is_empty(),
        "bug fixture signal failures:\n{}",
        total_errors.join("\n")
    );
}

#[test]
fn fx0_autotest_coverage_nonzero() {
    // Inline autotest checks for a representative fixture (walker)
    use bevy_swarm::autotest::{autotest, AutotestConfig};
    use fixture_walker::WalkerGamePlugin;

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
    assert!(
        non_empty,
        "coverage must be non-empty, got {:?}",
        report.coverage
    );
}

/// FX2 guard: each bug_* case must NOT fire its must_report signal
/// when FIXTURE_BUG is unset — proving the expectation detects the
/// actual injected bug rather than a scenario-side artifact.
#[test]
fn fx0_bug_signals_do_not_fire_with_bug_disabled() {
    let fixtures = discover_fixtures();
    let target = std::env::var("CARGO_TARGET_DIR").unwrap_or_else(|_| "target".into());
    let out_base = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(&target)
        .join("bevy_swarm/runs/fx0_gate_bug_off");
    std::fs::create_dir_all(&out_base).expect("create out dir");

    let mut total_errors = Vec::new();
    for fixture in &fixtures {
        let variants: Vec<String> = std::fs::read_dir(expectations_dir().join(fixture))
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
            let exp = load_expectation(fixture, &variant);
            if exp.get("pending").is_some() {
                continue;
            }
            let must: Vec<&str> = exp
                .get("must_report")
                .and_then(|m| m.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|e| e.get("rule").and_then(|r| r.as_str()))
                        .collect()
                })
                .unwrap_or_default();
            if must.is_empty() {
                continue;
            }
            let scenario = build_scenario_json(fixture, "clean", 42);
            let case_id = format!("{}-{}-bugoff", fixture, variant);
            let out_path = out_base.join(format!("{}_report.json", case_id));
            let report = run_fixture_opt(fixture, &variant, &scenario, &out_path, false);
            let fired: Vec<&str> = report
                .get("violations")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.get("rule").and_then(|r| r.as_str()))
                        .collect()
                })
                .unwrap_or_default();
            for rule in must {
                if fired.contains(&rule) {
                    total_errors.push(format!(
                        "{}/{}: rule '{}' fired with bug disabled",
                        fixture, variant, rule
                    ));
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
