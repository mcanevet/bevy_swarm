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
    Failed,
    Pending(String),
    UnexpectedPass(String),
}

fn golden_dir(fixture: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/swarm/golden")
        .join(fixture)
}

fn discover_cases() -> Vec<(String, String)> {
    let mut cases = Vec::new();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/swarm/golden");
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
    let bug_env = match variant {
        "clean" => None,
        v => Some(
            v.strip_prefix("bug_")
                .expect("variant is clean or bug_*")
                .to_string(),
        ),
    };

    let scenario = build_scenario(expectation);

    let mut cmd = Command::new("cargo");
    cmd.args(["run", "--quiet", "-p", "fixture-runner", "--", fixture])
        .arg(&scenario)
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

    if let Some(must) = expectation.get("must_report").and_then(|m| m.as_array()) {
        for entry in must {
            let rule = entry
                .get("rule")
                .and_then(|r| r.as_str())
                .expect("rule name");
            if !violation_rules.contains(&rule) {
                errors.push(format!("must_report: '{}' not reported", rule));
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
        "no golden cases found — tests/swarm/golden/ is empty?"
    );

    let mut outcomes = Vec::new();
    for (fixture, variant) in &cases {
        let expect_path = golden_dir(fixture).join(format!("{}.json", variant));
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
            (false, None) => Outcome::Failed,
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
        .filter(|c| matches!(c.outcome, Outcome::Failed))
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
        if let Outcome::Failed = c.outcome {
            let expect_path = golden_dir(&c.fixture).join(format!("{}.json", c.variant));
            eprintln!(
                "FAILED: {}/{} ({})",
                c.fixture,
                c.variant,
                expect_path.display()
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
/// (hand-rolled validator for the subset this suite uses: required fields,
/// types, enum values, and no unknown top-level keys).
#[test]
fn report_matches_schema() {
    let cases = discover_cases();
    let schema: serde_json::Value =
        serde_json::from_str(include_str!("../docs/report-schema.json")).expect("schema parses");

    let allowed_top: Vec<&str> = schema["properties"]
        .as_object()
        .unwrap()
        .keys()
        .map(|k| k.as_str())
        .collect();
    let statuses = ["pass", "fail", "crash"];

    for (fixture, variant) in &cases {
        let expectation = serde_json::from_str::<serde_json::Value>(
            &std::fs::read_to_string(golden_dir(fixture).join(format!("{}.json", variant)))
                .unwrap(),
        )
        .unwrap();
        let report = run_case(fixture, variant, &expectation);

        for key in report.as_object().unwrap().keys() {
            assert!(
                allowed_top.contains(&key.as_str()),
                "{}/{}: unexpected report key '{}'",
                fixture,
                variant,
                key
            );
        }
        let status = report["status"].as_str().unwrap();
        assert!(
            statuses.contains(&status),
            "{}/{}: bad status '{}'",
            fixture,
            variant,
            status
        );
        assert!(
            report["violations"].is_array()
                && report["metrics"].is_object()
                && report["coverage"].is_object()
                && report["frame_count"].is_u64(),
            "{}/{}: missing required fields",
            fixture,
            variant
        );
    }
}
