//! Acceptance test suite: plain fixtures + JSON expectations (outer loop)
//!
//! Per bead X1:
//! - runs each fixture variant via the fixture-runner binary (the user path)
//! - reads the report JSON FILE (target/bevy_swarm/runs/last/report.json),
//!   not the in-memory struct
//! - validates schema_version and must_report / must_not_report
//! - strict xfail semantics for "pending" cases
//! - prints a summary: passed / pending / failed / unexpected passes

use std::path::{Path, PathBuf};
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

fn golden_dir(fixture: impl AsRef<str>) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/swarm/expectations")
        .join(fixture.as_ref())
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

/// Locate the fixture-runner binary WITHOUT a nested `cargo run`
/// (swarm-13e). `cargo test --workspace` builds every workspace bin
/// before integration tests run, so `target/<profile>/fixture-runner`
/// already exists; resolve it directly. Falls back to building once
/// (with a visible, non-quiet error) if the harness was invoked in a
/// narrower way.
fn fixture_runner_bin() -> std::path::PathBuf {
    use std::sync::OnceLock;
    static BIN: OnceLock<std::path::PathBuf> = OnceLock::new();
    BIN.get_or_init(|| {
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        // target dir: honour CARGO_TARGET_DIR (hermetic runs) else the
        // workspace-root target/ next to the manifest.
        let profile = if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        };
        // le3.7 (RF3/stale-binary): NEVER trust a pre-existing
        // binary — `cargo test -p bevy_swarm` does NOT rebuild other
        // packages' bins. Always run `cargo build -p fixture-runner`
        // (a cheap no-op when fresh) so the tested binary reflects
        // current fixture/harness sources.
        let status = Command::new(env!("CARGO"))
            .args(["build", "-p", "fixture-runner"])
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .status()
            .expect("spawn cargo build -p fixture-runner");
        assert!(status.success(), "building fixture-runner failed");
        let path = std::env::var("CARGO_TARGET_DIR")
            .map(|t| PathBuf::from(t).join(profile).join("fixture-runner"))
            .unwrap_or_else(|_| manifest.join("target").join(profile).join("fixture-runner"));
        assert!(
            path.is_file(),
            "fixture-runner binary not found at {}",
            path.display()
        );
        path
    })
    .clone()
}

/// Run fixture-runner for a fixture+variant and return the report JSON read
/// from the file the harness wrote (the user path).
fn run_case(fixture: &str, variant: &str, expectation: &serde_json::Value) -> serde_json::Value {
    let scenario = build_scenario(expectation);
    let (report, _) = run_case_with_scenario(fixture, variant, &scenario, expectation);
    report
}

/// Z18 (swarm-m2h.19): scaffold:none cases write AutotestReport + per-seed
/// PlaytestReports; acceptance checks each seed-<n>.json with check_case.
fn run_case_with_scenario(
    fixture: &str,
    variant: &str,
    scenario: &str,
    expectation: &serde_json::Value,
) -> (serde_json::Value, PathBuf) {
    let bug_env = match variant {
        // Z18: clean variants (including scaffold:none copies like
        // clean_none) run the unmodified game.
        v if v.starts_with("clean") => None,
        v => Some(
            v.strip_prefix("bug_")
                .expect("variant is clean or bug_*")
                // Z18: scaffold:none copies are suffixed _none; the
                // underlying FIXTURE_BUG id drops the suffix.
                .trim_end_matches("_none")
                .to_string(),
        ),
    };

    // Per-case unique report path (FX2: no races on a shared
    // runs/last path). Conventions put run reports under
    // target/bevy_swarm/runs/<run_id>/report.json — use a case-named
    // id so parallel test threads never collide.
    // le3.7 (RF3): pid alone is NOT enough under `cargo test`
    // (threads share a pid) — add the current thread id AND a unique
    // counter: two invocations of the same (fixture, variant) from
    // different test fns can never collide.
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let case_id = format!(
        "{}-{}-{}-{:?}-{}",
        fixture,
        variant,
        std::process::id(),
        std::thread::current().id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    let target = std::env::var("CARGO_TARGET_DIR").unwrap_or_else(|_| "target".into());
    let out_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(&target)
        .join("bevy_swarm/runs")
        .join(&case_id);
    std::fs::create_dir_all(&out_dir).expect("create per-case out dir");
    let out_path = out_dir.join("report.json");

    let mut cmd = Command::new(fixture_runner_bin());
    cmd.arg(fixture)
        .arg(scenario)
        .arg("--out")
        .arg(&out_path)
        .current_dir(env!("CARGO_MANIFEST_DIR"));
    // Z18 (swarm-m2h.19): scaffold selects the fixture-runner build
    // path — "none" = zero-contract autotest over the plain game
    // plugin; "adapter" (default) = the historical adapter path.
    let scaffold = expectation
        .get("scaffold")
        .and_then(|s| s.as_str())
        .unwrap_or("adapter");
    cmd.arg("--scaffold").arg(scaffold);
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

    let json = std::fs::read_to_string(&out_path)
        .unwrap_or_else(|e| panic!("read report {}: {}", out_path.display(), e));
    let report: serde_json::Value = serde_json::from_str(&json).expect("report is valid JSON");
    (report, out_dir)
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
    // An explicit bot (e.g. a deterministic replay forcing a corner
    // condition) overrides the default chaos bot.
    if let Some(bot) = expectation.get("bot") {
        scenario["bot"] = bot.clone();
    }
    if let Some(rate) = expectation.get("input_rate_hz").and_then(|r| r.as_u64()) {
        scenario["bot"]["input_rate_hz"] = serde_json::json!(rate);
    }
    if let Some(check_det) = expectation
        .get("check_determinism")
        .and_then(|d| d.as_u64())
    {
        scenario["check_determinism"] = serde_json::json!(check_det);
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

    // Z1 (swarm-m2h.2): every report carries a checked scope; an
    // optional "expect_tier" field asserts the detected tier.
    let scope = report.get("scope");
    if scope.is_none() {
        errors.push("scope: missing from report (Z1)".into());
    } else {
        let tier = scope
            .and_then(|s| s.get("tier"))
            .and_then(|t| t.as_str())
            .unwrap_or("<missing>");
        if let Some(expected_tier) = expectation.get("expect_tier").and_then(|t| t.as_str()) {
            if tier != expected_tier {
                errors.push(format!(
                    "scope.tier: expected {}, got {}",
                    expected_tier, tier
                ));
            }
        } else if expectation
            .get("scaffold")
            .and_then(|s| s.as_str())
            .is_some_and(|s| s == "adapter")
        {
            // Adapter cases are semantic by construction (they install
            // TestConventionsPlugin pieces). Guard against accidental
            // tier regressions until Z18 converts them to "none".
            if tier != "semantic" {
                errors.push(format!(
                    "scope.tier: adapter scaffold must be semantic, got {}",
                    tier
                ));
            }
        }
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

/// Conventions §4: every persisted JSON file carries schema_version.
#[test]
fn persisted_files_have_schema_version() {
    for (fixture, case) in discover_cases() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/swarm/expectations")
            .join(&fixture)
            .join(format!("{}.json", case));
        let expectation = std::fs::read_to_string(&path).expect("read case");
        let v: serde_json::Value = serde_json::from_str(&expectation).expect("case parses");
        assert_eq!(
            v.get("schema_version").and_then(|s| s.as_i64()),
            Some(1),
            "{}: missing or wrong schema_version",
            path.display()
        );
    }
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
        let expect_path = golden_dir(fixture).join(format!("{}.json", variant));
        let raw = std::fs::read_to_string(&expect_path)
            .unwrap_or_else(|e| panic!("read {}: {}", expect_path.display(), e));
        let expectation: serde_json::Value = serde_json::from_str(&raw)
            .unwrap_or_else(|e| panic!("parse {}: {}", expect_path.display(), e));

        let scenario_built = build_scenario(&expectation);
        let (report, out_dir) =
            run_case_with_scenario(fixture, variant, &scenario_built, &expectation);
        let scaffold = expectation
            .get("scaffold")
            .and_then(|s| s.as_str())
            .unwrap_or("adapter");
        let errors = if scaffold == "none" {
            // Z18 (swarm-m2h.19): scaffold:none writes AutotestReport +
            // per-seed PlaytestReports; check each seed individually.
            let mut all_errors = Vec::new();
            for entry in std::fs::read_dir(out_dir).expect("read out dir").flatten() {
                let p = entry.path();
                if !p
                    .file_name()
                    .map(|n| n.to_string_lossy().starts_with("seed-"))
                    .unwrap_or(false)
                {
                    continue;
                }
                let text = std::fs::read_to_string(&p).expect("read seed report");
                let seed_report: serde_json::Value =
                    serde_json::from_str(&text).expect("seed report parses");
                let seed_errors = check_case(&seed_report, &expectation);
                if !seed_errors.is_empty() {
                    all_errors.push(format!(
                        "{}: {:#?}",
                        p.file_name().unwrap().to_string_lossy(),
                        seed_errors
                    ));
                }
            }
            all_errors
        } else {
            check_case(&report, &expectation)
        };

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
    // Z18 (amendment 2): scaffold:none cases produce an
    // AutotestReport — validate against its definition in the same
    // schema file.
    let autotest_validator = {
        let wrapper = serde_json::json!({
            "$ref": "#/$defs/autotest_report",
            "$defs": schema["$defs"].clone()
        });
        jsonschema::validator_for(&wrapper).expect("autotest schema compiles")
    };

    for (fixture, variant) in &cases {
        let expectation = serde_json::from_str::<serde_json::Value>(
            &std::fs::read_to_string(golden_dir(fixture).join(format!("{}.json", variant)))
                .unwrap(),
        )
        .unwrap();
        let report = run_case(fixture, variant, &expectation);
        let use_autotest = expectation
            .get("scaffold")
            .and_then(|s| s.as_str())
            .is_some_and(|s| s == "none");
        let errors: Vec<String> = if use_autotest {
            autotest_validator
                .iter_errors(&report)
                .map(|e| format!("{} at {}", e, e.instance_path))
                .collect()
        } else {
            validator
                .iter_errors(&report)
                .map(|e| format!("{} at {}", e, e.instance_path))
                .collect()
        };
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
        let out = Command::new(fixture_runner_bin())
            .arg(fixture)
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
    let out = Command::new(fixture_runner_bin())
        .arg("walker")
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
/// Iterates the fixtures directory (not a hardcoded list) so new
/// fixtures are automatically covered.
#[test]
fn clean_fixtures_pass_long() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let fixtures_dir = manifest.join("fixtures");
    let mut fixtures: Vec<String> = std::fs::read_dir(&fixtures_dir)
        .expect("fixtures dir exists")
        .filter_map(|e| {
            let p = e.ok()?.path();
            if p.is_dir() {
                p.file_name()?.to_str().map(String::from)
            } else {
                None
            }
        })
        // Z19: bin-only fixtures are external smoke binaries, not
        // in-process adapter fixtures (no fixture-runner support).
        // Z20: likewise the BRP fixture (external subprocess tier).
        .filter(|f| !f.starts_with("bin_only") && f != "brp")
        .collect();
    fixtures.sort();
    assert!(
        fixtures.len() >= 5,
        "expected at least 5 fixtures, found {:?}",
        fixtures
    );
    for fixture in &fixtures {
        // le3.8 (RF4): spawner uses IdlePhase watch; others real_time.
        let liveness = if fixture == "spawner" {
            serde_json::json!({"mode": "real_time", "watch": ["IdlePhase"]})
        } else {
            serde_json::json!({"mode": "real_time"})
        };
        for seed in 1..=8 {
            let scenario = serde_json::json!({
                "bot": {"type": "chaos", "seed": seed},
                "duration_s": 6.0,
                "invariants": [],
                "setup": {},
                "liveness": liveness
            })
            .to_string();
            let (report, _) =
                run_case_with_scenario(fixture, "clean", &scenario, &serde_json::json!({}));
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

/// FX1 re-review: a swallow-everything softlock (bug_softlock_turn_4)
/// must be detected EVEN WHILE THE BOT KEEPS RETRYING — the stuck
/// le3.8 (RF4): the sticky-streak-only fix (idle_since_intent check
/// removed) ensures detection even with aggressive/fast bots.
#[test]
fn softlock_detected_while_bot_retries() {
    for timeout_s in [0.5f64, 2.0] {
        let scenario = serde_json::json!({
            "bot": {"type": "chaos", "seed": 42},
            "duration_s": 10.0,
            "invariants": [],
            "setup": {},
            "liveness": {"mode": "after_intent", "timeout_s": timeout_s}
        })
        .to_string();
        let (report, _) = run_case_with_scenario(
            "turn_based",
            "bug_softlock_turn_4",
            &scenario,
            &serde_json::json!({}),
        );
        let stuck = report["violations"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .any(|v| v.get("rule").and_then(|r| r.as_str()) == Some("stuck_after_intent"))
            })
            .unwrap_or(false);
        assert!(
            stuck,
            "bug_softlock_turn_4 (timeout {}s) not detected — stuck streak broken?",
            timeout_s
        );
    }
}

/// le3.8 (RF4): aggressive bot at 30 Hz must still detect softlock.
/// The old guard (idle_since_intent >= quarter) was defeated by any
/// bot retrying at least every 15 frames.
#[test]
fn softlock_aggressive_30hz_detected() {
    let scenario = serde_json::json!({
        "bot": {"type": "chaos", "seed": 42, "persona": "aggressive", "input_rate_hz": 30},
        "duration_s": 10.0,
        "invariants": [],
        "setup": {},
        "liveness": {"mode": "after_intent", "timeout_s": 2.0}
    })
    .to_string();
    let (report, _) = run_case_with_scenario(
        "turn_based",
        "bug_softlock_turn_4",
        &scenario,
        &serde_json::json!({}),
    );
    let stuck = report["violations"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .any(|v| v.get("rule").and_then(|r| r.as_str()) == Some("stuck_after_intent"))
        })
        .unwrap_or(false);
    assert!(
        stuck,
        "bug_softlock_turn_4 not detected with aggressive bot at 30Hz"
    );
}

/// le3.8 (RF4): 32-seed sweep — all seeds must detect the softlock.
#[test]
fn softlock_sweep_seeds_1_to_32() {
    let mut misses = Vec::new();
    for seed in 1..=32 {
        let scenario = serde_json::json!({
            "bot": {"type": "chaos", "seed": seed},
            "duration_s": 10.0,
            "invariants": [],
            "setup": {},
            "liveness": {"mode": "after_intent", "timeout_s": 2.0}
        })
        .to_string();
        let (report, _) = run_case_with_scenario(
            "turn_based",
            "bug_softlock_turn_4",
            &scenario,
            &serde_json::json!({}),
        );
        let stuck = report["violations"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .any(|v| v.get("rule").and_then(|r| r.as_str()) == Some("stuck_after_intent"))
            })
            .unwrap_or(false);
        if !stuck {
            misses.push(seed);
        }
    }
    assert!(
        misses.is_empty(),
        "softlock missed seeds {:?} at 10s duration, 2s timeout",
        misses
    );
}

/// le3.8 (RF4): frozen_world must still fire for liveness timeouts
/// beyond the old 600-frame ring capacity (10s @ 60 TPS). The ring
/// cap now scales with timeout_s — spinner bug_frozen with a 12s
/// timeout over 30s must be detected (it silently passed before).
#[test]
fn frozen_world_fires_for_long_timeouts() {
    let scenario = serde_json::json!({
        "bot": {"type": "chaos", "seed": 1},
        "duration_s": 30.0,
        "invariants": [],
        "setup": {},
        "liveness": {"mode": "real_time", "timeout_s": 12.0}
    })
    .to_string();
    let (report, _) =
        run_case_with_scenario("spinner", "bug_frozen", &scenario, &serde_json::json!({}));
    let frozen = report["violations"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .any(|v| v.get("rule").and_then(|r| r.as_str()) == Some("frozen_world"))
        })
        .unwrap_or(false);
    assert!(
        frozen,
        "bug_frozen with 12s timeout not detected — ring buffer cap regression?"
    );
}

/// FX1 re-review: Wait intents must NOT arm the stuck_after_intent
/// check. Clean turn_based with a tight 0.5s timeout over 8 seeds
/// must report ZERO stuck_after_intent violations.
#[test]
fn clean_turn_based_no_stuck_on_wait() {
    for seed in 1..=8u64 {
        let scenario = serde_json::json!({
            "bot": {"type": "chaos", "seed": seed},
            "duration_s": 3.0,
            "invariants": [],
            "setup": {},
            "liveness": {"mode": "after_intent", "timeout_s": 0.5}
        })
        .to_string();
        let (report, _) =
            run_case_with_scenario("turn_based", "clean", &scenario, &serde_json::json!({}));
        let stuck: Vec<String> = report["violations"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter(|v| {
                        v.get("rule").and_then(|r| r.as_str()) == Some("stuck_after_intent")
                    })
                    .filter_map(|v| {
                        v.get("detail")
                            .or_else(|| v.get("last_detail"))
                            .and_then(|d| d.as_str())
                            .map(String::from)
                    })
                    .collect()
            })
            .unwrap_or_default();
        assert!(
            stuck.is_empty(),
            "clean turn_based seed={} fired stuck_after_intent on Wait intents: {:?}",
            seed,
            stuck
        );
    }
}

/// Z19: build a workspace fixture binary on demand and return its
/// executable path (CI test jobs don't build fixture bins).
fn build_fixture_bin(package: &str) -> std::path::PathBuf {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let out = std::process::Command::new(cargo)
        .args(["build", "-p", package, "--message-format=json"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("cargo build runs");
    assert!(
        out.status.success(),
        "cargo build -p {package} failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
            if v["reason"] == "compiler-artifact"
                && v["target"]["kind"][0] == "bin"
                && v["executable"].is_string()
            {
                return std::path::PathBuf::from(v["executable"].as_str().unwrap());
            }
        }
    }
    panic!("no executable artifact found for {package}");
}

/// Z19 (m2h.20): smoke runner with stderr classifier on a crashing binary.
#[test]
fn smoke_crashing_binary_yields_crash_status() {
    use bevy_swarm::smoke::smoke_run;

    let binary = build_fixture_bin("fixture_bin_only");
    let report = smoke_run(&binary, &[], 5).expect("smoke_run succeeds");

    assert_eq!(report.status, bevy_swarm::enums::PlaytestStatus::Crash);
    assert_eq!(report.harness_mode, "external_smoke");
    assert!(report.violations.iter().any(|v| v.rule == "panic"));
}

/// Z19 amendment: a CLEAN binary must NOT yield crash/error status
/// (false-positive guard — the classifier must not fire on stderr
/// noise).
#[test]
fn smoke_clean_binary_passes() {
    use bevy_swarm::smoke::smoke_run;

    let binary = build_fixture_bin("fixture_bin_only_clean");
    let report = smoke_run(&binary, &[], 30).expect("smoke_run succeeds");

    assert_eq!(
        report.status,
        bevy_swarm::enums::PlaytestStatus::Pass,
        "clean binary must pass: violations={:?}",
        report.violations
    );
    assert_eq!(report.harness_mode, "external_smoke");
    assert!(report.violations.is_empty());
}

/// Z20 (m2h.27): external BRP observe-only run against a BRP-enabled
/// fixture. Verifies the report is stamped external_driven, the
/// liveness oracle polled a changing world, and no crash is reported.
#[test]
fn brp_external_run_observes_live_game() {
    use bevy_swarm::external::{external_brp_run, ExternalBrpConfig};

    let binary = build_fixture_bin("fixture_brp");
    let config = ExternalBrpConfig {
        binary_path: binary.to_string_lossy().to_string(),
        timeout_secs: 8,
        poll_interval_ms: 250,
        ..Default::default()
    };
    let report = external_brp_run(&config).expect("external_brp_run succeeds");

    assert_eq!(report.harness_mode, "external_driven");
    assert_eq!(
        report.status,
        bevy_swarm::enums::PlaytestStatus::Pass,
        "live BRP game must pass: violations={:?} warnings={:?}",
        report.violations,
        report.warnings
    );
    // The liveness oracle must have polled the game several times.
    assert!(
        report.frame_count >= 4,
        "expected several BRP polls, got {}",
        report.frame_count
    );
    // Observe-only scope: input/invariants/determinism must be inactive.
    let inactive: Vec<_> = report
        .scope
        .inactive
        .iter()
        .map(|(family, _)| family.as_str())
        .collect();
    assert!(inactive.contains(&"input"));
    assert!(inactive.contains(&"invariants"));
    assert!(inactive.contains(&"determinism_replay"));
}

/// Z20 amendment: a binary WITHOUT BRP (no RemotePlugin) must not be
/// reported as a pass with silent no-observation — the harness records
/// the unreachable endpoint as an error.
#[test]
fn brp_external_run_reports_unreachable_endpoint() {
    use bevy_swarm::external::{external_brp_run, ExternalBrpConfig};

    let binary = build_fixture_bin("fixture_bin_only_clean");
    let config = ExternalBrpConfig {
        binary_path: binary.to_string_lossy().to_string(),
        timeout_secs: 8,
        startup_grace_secs: 3,
        ..Default::default()
    };
    let report = external_brp_run(&config).expect("external_brp_run succeeds");

    assert_eq!(report.harness_mode, "external_driven");
    assert!(
        report.error.is_some(),
        "non-BRP binary must surface an error"
    );
    assert!(
        report
            .warnings
            .iter()
            .any(|w| w.contains("never became reachable")),
        "warnings must explain the missing BRP endpoint: {:?}",
        report.warnings
    );
}
