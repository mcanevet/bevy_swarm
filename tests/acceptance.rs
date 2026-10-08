//! Acceptance test suite: plain fixtures + JSON expectations (outer loop)
//!
//! Per bead X1:
//! - runs each fixture variant via the fixture-runner binary (the user path)
//! - reads the report JSON FILE (target/bevy_swarm/runs/last/report.json),
//!   not the in-memory struct
//! - validates schema_version and must_report / must_not_report
//! - strict xfail semantics for "pending" cases
//! - prints a summary: passed / pending / failed / unexpected passes

use std::path::PathBuf;
use std::process::Command;

#[derive(Debug)]
struct CaseOutcome {
    fixture: String,
    variant: String,
    outcome: Outcome,
}

#[derive(Debug, PartialEq)]
enum Outcome {
    Passed,
    Failed(Vec<String>),
    Pending(String),
    UnexpectedPass(String),
}

fn golden_dir(fixture: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/swarm/expectations")
        .join(fixture)
}

fn discover_cases() -> Vec<(String, String)> {
    let mut cases = Vec::new();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/swarm/expectations");
    let mut fixtures: Vec<_> = std::fs::read_dir(&root)
        .expect("golden dir exists")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    fixtures.sort();
    for fixture_dir in fixtures {
        let fixture = fixture_dir
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();
        let mut variants: Vec<_> = std::fs::read_dir(&fixture_dir)
            .expect("fixture golden dir readable")
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "json"))
            .collect();
        variants.sort();
        for v in variants {
            let variant = v.file_stem().unwrap().to_string_lossy().to_string();
            cases.push((fixture.clone(), variant));
        }
    }
    cases
}

/// Run fixture-runner for a fixture+variant and return the report JSON read
/// from the file the harness wrote (the user path).
fn run_case(fixture: &str, variant: &str, expectation: &serde_json::Value) -> serde_json::Value {
    let scenario = build_scenario(expectation);
    run_case_with_scenario(fixture, variant, &scenario)
}

/// FX1: fixture-runner writes reports to the SHARED runs/last path —
/// concurrent tests would race on it and read each other's reports
/// (the spinner "frozen_world" flake was actually the spawner's
/// report). Serialize all fixture-runner invocations process-wide.
static RUNNER_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Run fixture-runner with an explicit scenario string (FX1: the
/// meta-test builds long-duration scenarios directly).
fn run_case_with_scenario(fixture: &str, variant: &str, scenario: &str) -> serde_json::Value {
    let bug_env = match variant {
        "clean" => None,
        v => Some(
            v.strip_prefix("bug_")
                .expect("variant is clean or bug_*")
                .to_string(),
        ),
    };

    let _guard = RUNNER_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut cmd = Command::new("cargo");
    cmd.args(["run", "--quiet", "-p", "fixture-runner", "--", fixture])
        .arg(scenario)
        .current_dir(env!("CARGO_MANIFEST_DIR"));
    if let Some(bug) = &bug_env {
        cmd.env("FIXTURE_BUG", bug);
    }

    let output = cmd.output().expect("run fixture-runner");
    if !output.status.success() {
        panic!(
            "fixture-runner failed for {}/{}: {}",
            fixture,
            variant,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let report_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(conventions_report_relpath());
    let json = std::fs::read_to_string(&report_path)
        .unwrap_or_else(|e| panic!("read report {}: {}", report_path.display(), e));
    serde_json::from_str(&json).expect("report is valid JSON")
}

fn conventions_report_relpath() -> String {
    // tests run with cwd = crate root; CARGO_TARGET_DIR may relocate target/
    let target = std::env::var("CARGO_TARGET_DIR").unwrap_or_else(|_| "target".into());
    format!("{}/bevy_swarm/runs/last/report.json", target)
}

/// Build the scenario JSON from the expectation file (duration, seeds,
/// invariants). Falls back to a minimal chaos scenario.
fn build_scenario(expectation: &serde_json::Value) -> String {
    let mut scenario = serde_json::json!({
        "bot": { "type": "chaos", "seed": 42 },
        "duration_s": expectation.get("duration_s").and_then(|d| d.as_f64()).unwrap_or(1.0),
        "invariants": expectation.get("invariants").cloned().unwrap_or(serde_json::json!([])),
        "setup": {}
    });
    if let Some(seeds) = expectation.get("seeds").and_then(|s| s.as_array()) {
        if let Some(first) = seeds.first().and_then(|s| s.as_u64()) {
            scenario["bot"]["seed"] = serde_json::json!(first);
        }
    }
    if let Some(liveness) = expectation.get("liveness") {
        scenario["liveness"] = liveness.clone();
    }
    if let Some(rate) = expectation.get("input_rate_hz").and_then(|r| r.as_u64()) {
        scenario["bot"]["input_rate_hz"] = serde_json::json!(rate);
    }
    scenario.to_string()
}

fn check_case(report: &serde_json::Value, expectation: &serde_json::Value) -> Vec<String> {
    let mut errors = Vec::new();

    let schema_version = report
        .get("schema_version")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    if schema_version != 1 {
        errors.push(format!(
            "schema_version: expected 1, got {}",
            schema_version
        ));
    }

    let expected_status = expectation
        .get("status")
        .and_then(|s| s.as_str())
        .expect("expectation has status");
    let actual_status = report
        .get("status")
        .and_then(|s| s.as_str())
        .unwrap_or("<missing>");
    if expected_status != actual_status {
        errors.push(format!(
            "status: expected {}, got {}",
            expected_status, actual_status
        ));
    }

    let violation_rules: Vec<&str> = report
        .get("violations")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.get("rule").and_then(|r| r.as_str()))
                .collect()
        })
        .unwrap_or_default();

    // FX2: expectations pin the ACTUAL bug, not just a rule name —
    // optional detail_contains / target / min_count narrow the match.
    let violation_objects: Vec<&serde_json::Value> = report
        .get("violations")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().collect())
        .unwrap_or_default();

    if let Some(must) = expectation.get("must_report").and_then(|m| m.as_array()) {
        for entry in must {
            let rule = entry
                .get("rule")
                .and_then(|r| r.as_str())
                .expect("rule name");
            let matching: Vec<&&serde_json::Value> = violation_objects
                .iter()
                .filter(|v| v.get("rule").and_then(|r| r.as_str()) == Some(rule))
                .collect();
            if matching.is_empty() {
                errors.push(format!("must_report: '{}' not reported", rule));
                continue;
            }
            if let Some(want) = entry.get("detail_contains").and_then(|d| d.as_str()) {
                if !matching.iter().any(|v| {
                    v.get("detail")
                        .and_then(|d| d.as_str())
                        .is_some_and(|d| d.contains(want))
                }) {
                    errors.push(format!(
                        "must_report: '{}' reported but no detail contains {:?}",
                        rule, want
                    ));
                }
            }
            if let Some(target) = entry.get("target").and_then(|d| d.as_str()) {
                if !matching
                    .iter()
                    .any(|v| v.get("target").and_then(|d| d.as_str()) == Some(target))
                {
                    errors.push(format!(
                        "must_report: '{}' reported but not on target {:?}",
                        rule, target
                    ));
                }
            }
            if let Some(min_count) = entry.get("min_count").and_then(|d| d.as_u64()) {
                // Aggregated reports collapse repeats into count — use
                // that when present, else the array length.
                let n = matching
                    .iter()
                    .filter_map(|v| v.get("count").and_then(|c| c.as_u64()))
                    .max()
                    .unwrap_or(matching.len() as u64);
                if n < min_count {
                    errors.push(format!(
                        "must_report: '{}' reported {} times, want >= {}",
                        rule, n, min_count
                    ));
                }
            }
        }
    }
    if let Some(must_not) = expectation
        .get("must_not_report")
        .and_then(|m| m.as_array())
    {
        for entry in must_not {
            let rule = entry
                .get("rule")
                .and_then(|r| r.as_str())
                .expect("rule name");
            if violation_rules.contains(&rule) {
                errors.push(format!("must_not_report: '{}' was reported", rule));
            }
        }
    }

    errors
}

#[test]
fn acceptance_suite() {
    let cases = discover_cases();
    assert!(
        !cases.is_empty(),
        "no expectation cases found — tests/swarm/expectations/ is empty?"
    );

    let mut outcomes = Vec::new();
    for (fixture, variant) in &cases {
        let expect_path = golden_dir(&fixture).join(format!("{}.json", variant));
        let raw = std::fs::read_to_string(&expect_path)
            .unwrap_or_else(|e| panic!("read {}: {}", expect_path.display(), e));
        let expectation: serde_json::Value = serde_json::from_str(&raw)
            .unwrap_or_else(|e| panic!("parse {}: {}", expect_path.display(), e));

        let report = run_case(fixture, variant, &expectation);
        let errors = check_case(&report, &expectation);

        let pending = expectation.get("pending").and_then(|p| p.as_str());
        let outcome = match (errors.is_empty(), pending) {
            (true, None) => Outcome::Passed,
            (true, Some(bead)) => Outcome::UnexpectedPass(format!("remove pending: {}", bead)),
            (false, Some(bead)) => {
                Outcome::Pending(format!("{} ({}): {:?}", bead, variant, errors))
            }
            (false, None) => Outcome::Failed(errors),
        };
        outcomes.push(CaseOutcome {
            fixture: fixture.clone(),
            variant: variant.clone(),
            outcome,
        });
    }

    let passed = outcomes
        .iter()
        .filter(|c| c.outcome == Outcome::Passed)
        .count();
    let failed = outcomes
        .iter()
        .filter(|c| matches!(c.outcome, Outcome::Failed(_)))
        .count();
    let unexpected: Vec<&CaseOutcome> = outcomes
        .iter()
        .filter(|c| matches!(c.outcome, Outcome::UnexpectedPass(_)))
        .collect();
    let pending: Vec<&CaseOutcome> = outcomes
        .iter()
        .filter(|c| matches!(c.outcome, Outcome::Pending(_)))
        .collect();

    println!("\n=== Acceptance Summary ===");
    println!("passed: {}", passed);
    println!("pending: {}", pending.len());
    for p in &pending {
        if let Outcome::Pending(note) = &p.outcome {
            println!("  pending: {}/{} — {}", p.fixture, p.variant, note);
        }
    }
    println!("failed: {}", failed);
    println!("unexpected passes: {}", unexpected.len());
    for u in &unexpected {
        if let Outcome::UnexpectedPass(note) = &u.outcome {
            println!("  unexpected pass: {}/{} — {}", u.fixture, u.variant, note);
        }
    }

    for c in &outcomes {
        if let Outcome::Failed(errors) = &c.outcome {
            let expect_path = golden_dir(&c.fixture).join(format!("{}.json", c.variant));
            eprintln!(
                "FAILED: {}/{} ({}) — {:#?}",
                c.fixture,
                c.variant,
                expect_path.display(),
                errors
            );
        }
    }

    assert_eq!(
        unexpected.len(),
        0,
        "pending cases that pass must lose their pending marker (strict xfail)"
    );
    assert_eq!(failed, 0, "acceptance failures");
    assert!(
        passed + pending.len() + unexpected.len() + failed == outcomes.len(),
        "accounting error: every case must be passed, pending, unexpected, or failed"
    );
}

/// Hard rule (bead X1): no file under fixtures/ may mention bevy_swarm
/// harness plumbing. Fixture crates depend on bevy ONLY.
#[test]
fn fixtures_have_no_harness_plumbing() {
    let forbidden = [
        "bevy_swarm",
        "TestApi",
        "UserIntent",
        "IntentSurface",
        "PlaytestPlugin",
        // FX2: the harness-side markers are also plumbing — fixtures
        // must not import/derive their identity from them.
        "RealTime",
        "PlaytestSet",
        "Gameplay,",
        "Gameplay;",
        "Gameplay>",
        "(Gameplay",
        "Gameplay)",
        ": Gameplay",
    ];
    // "Gameplay" the MARKER: the word Gameplay alone may appear in prose;
    // the marker type is `Gameplay` in struct position — approximate by
    // banning the compound forms fixtures would need.
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures");
    let mut violations = Vec::new();
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("fixtures dir readable") {
            let entry = entry.unwrap().path();
            if entry.is_dir() {
                if entry.file_name().unwrap() == "target" {
                    continue;
                }
                stack.push(entry);
            } else if entry.extension().is_some_and(|e| e == "rs" || e == "toml") {
                let content = std::fs::read_to_string(&entry).unwrap();
                let name = entry.to_string_lossy().to_string();
                for term in forbidden {
                    if content.contains(term) {
                        violations.push(format!("{} mentions '{}'", name, term));
                    }
                }
            }
        }
    }
    assert!(
        violations.is_empty(),
        "fixture plumbing violations: {:#?}",
        violations
    );
}

/// Reports from real runs validate against docs/report-schema.json
/// (FX2: FULL draft-2020-12 validation via the jsonschema crate — the
/// hand-rolled top-level-key subset missed nested shape errors).
#[test]
fn report_matches_schema() {
    let cases = discover_cases();
    let schema: serde_json::Value =
        serde_json::from_str(include_str!("../docs/report-schema.json")).expect("schema parses");
    let validator = jsonschema::validator_for(&schema).expect("schema compiles");

    for (fixture, variant) in &cases {
        let expectation = serde_json::from_str::<serde_json::Value>(
            &std::fs::read_to_string(golden_dir(fixture).join(format!("{}.json", variant)))
                .unwrap(),
        )
        .unwrap();
        let report = run_case(fixture, variant, &expectation);

        let errors: Vec<String> = validator
            .iter_errors(&report)
            .map(|e| format!("{} at {}", e, e.instance_path))
            .collect();
        assert!(
            errors.is_empty(),
            "{}/{}: report does not match docs/report-schema.json: {:#?}",
            fixture,
            variant,
            errors
        );
    }
}

/// FX2: unknown FIXTURE_BUG values must PANIC in the fixture, not
/// silently run the clean game (a typo'd case would vacuously pass).
/// RED PHASE: currently unknown bugs fall through to the clean system.
#[test]
fn unknown_fixture_bug_panics() {
    for fixture in [
        "walker",
        "spinner",
        "spawner",
        "turn_based",
        "regression_pair",
    ] {
        let out = Command::new("cargo")
            .args(["run", "--quiet", "-p", "fixture-runner", "--", fixture])
            .arg(
                r#"{"bot":{"type":"replay","inputs":[]},"duration_s":0.5,"invariants":[],"setup":{}}"#,
            )
            .env("FIXTURE_BUG", "definitely_not_a_real_bug")
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .output()
            .expect("run fixture-runner");
        assert!(
            !out.status.success(),
            "{fixture}: unknown FIXTURE_BUG must panic, not silently run clean"
        );
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            stderr.contains("unknown FIXTURE_BUG"),
            "{fixture}: panic must name the problem, stderr: {stderr}"
        );
    }
}

/// FX2: reports must be read from a per-case unique path (--out), not
/// the shared runs/last copy that parallel tests race on.
#[test]
fn fixture_runner_supports_out_flag() {
    let out_dir = std::env::temp_dir().join(format!("fx2-out-test-{}", std::process::id()));
    let out_path = out_dir.join("report.json");
    let out = Command::new("cargo")
        .args(["run", "--quiet", "-p", "fixture-runner", "--", "walker"])
        .arg(r#"{"bot":{"type":"replay","inputs":[]},"duration_s":0.5,"invariants":[],"setup":{}}"#)
        .arg("--out")
        .arg(&out_path)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("run fixture-runner");
    assert!(
        out.status.success(),
        "fixture-runner --out must work: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let content = std::fs::read_to_string(&out_path)
        .unwrap_or_else(|e| panic!("--out report must exist at {}: {}", out_path.display(), e));
    assert!(
        content.contains("\"schema_version\""),
        "--out report must be the report JSON"
    );
    let _ = std::fs::remove_dir_all(&out_dir);
}

/// FX2: every "pending" expectation must reference an OPEN bead, per
/// tests/swarm/pending.json (kept in sync manually; bd isn't callable
/// from CI). A pending case referencing a CLOSED bead is dead weight
/// that hides a real failure.
#[test]
fn pending_expectations_reference_open_beads() {
    let pending_registry: serde_json::Value =
        serde_json::from_str(include_str!("swarm/pending.json"))
            .expect("tests/swarm/pending.json parses");
    let closed: Vec<&str> = pending_registry["closed"]
        .as_array()
        .expect("closed: array of CLOSED bead ids")
        .iter()
        .filter_map(|v| v.as_str())
        .collect();

    for (fixture, variant) in discover_cases() {
        let expect_path = golden_dir(&fixture).join(format!("{}.json", variant));
        let raw = std::fs::read_to_string(&expect_path).unwrap();
        let expectation: serde_json::Value = serde_json::from_str(&raw).unwrap();
        if let Some(bead) = expectation.get("pending").and_then(|p| p.as_str()) {
            assert!(
                !closed.contains(&bead),
                "{}/{} is pending on '{}' but that bead is CLOSED in pending.json —                  investigate (it may now pass, or the case needs fixing)",
                fixture,
                variant,
                bead
            );
        }
    }
}

/// FX1 meta-test: clean fixtures with long duration must have zero violations.
/// Currently fails (frozen_world + dead_verb false positives) — proves the bug.
#[test]
fn clean_fixtures_pass_long() {
    let fixtures = ["walker", "spinner", "turn_based", "regression_pair"];
    for fixture in &fixtures {
        for seed in 1..=8 {
            let scenario = serde_json::json!({
                "bot": {"type": "chaos", "seed": seed},
                "duration_s": 6.0,
                "invariants": [],
                "setup": {}
            })
            .to_string();
            let report = run_case_with_scenario(fixture, "clean", &scenario);
            let violations: Vec<String> = report["violations"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.get("rule").and_then(|r| r.as_str()).map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            assert!(
                violations.is_empty(),
                "clean {}/seed={} has violations: {:?}",
                fixture,
                seed,
                violations
            );
        }
    }
}
