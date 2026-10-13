//! Z18: autotest() integration — config defaults, env overrides, report.

use bevy::prelude::*;
use bevy_swarm::autotest::{autotest, autotest_with, AutotestConfig};
use bevy_swarm::contract::ContractTier;
use serial_test::serial;

#[derive(Clone, Default)]
struct SimpleGamePlugin;

impl Plugin for SimpleGamePlugin {
    fn build(&self, _: &mut App) {}
}

#[test]
fn autotest_runs_and_writes_report() {
    // Small config to keep the test fast.
    let config = AutotestConfig {
        num_seeds: 2,
        duration_s: 0.05,
        parallel: 1,
        minimize_failures: false,
        max_failures: None,
        ..AutotestConfig::default()
    };
    let report = autotest(SimpleGamePlugin, config).expect("autotest runs");
    // Z18.B (kill the tautology): schema_version is the conventions
    // INTEGER, harness_mode uses U1 vocabulary, contract_tier comes
    // from the per-run Z1 scopes (fixture-derived, not hardcoded).
    assert_eq!(
        report.schema_version,
        bevy_swarm::conventions::SCHEMA_VERSION
    );
    assert_eq!(report.harness_mode, "in_process");
    assert!(
        matches!(
            report.contract_tier,
            ContractTier::Zero | ContractTier::Observable | ContractTier::Semantic
        ),
        "contract_tier must come from a Z1 tier"
    );
    assert_eq!(report.budget.seeds_run, 2);
    assert_eq!(report.runs.len(), 2);
    // Clean game: no failures.
    assert!(report.failures.is_empty(), "{:?}", report.failures);
    let summary = report.summary();
    assert!(summary.contains("2 seeds"), "summary: {summary}");
    // Report file was written under target/.
    let target = std::env::var("CARGO_TARGET_DIR").unwrap_or_else(|_| "target".into());
    let runs_dir = std::path::Path::new(&target).join("bevy_swarm/runs");
    assert!(
        std::fs::read_dir(&runs_dir)
            .map(|entries| entries
                .filter_map(|e| e.ok())
                .any(|e| e.file_name().to_string_lossy() != "last"))
            .unwrap_or(false),
        "expected a run dir under {}",
        runs_dir.display()
    );
}

/// FX4.1: autotest adds PlaytestPlugin — bots/oracles run, frames > 0.
#[test]
fn autotest_adds_playtest_plugin_frames_gt_zero() {
    let config = AutotestConfig {
        num_seeds: 2,
        duration_s: 0.1,
        parallel: 1,
        minimize_failures: false,
        max_failures: None,
        ..AutotestConfig::default()
    };
    let report = autotest(SimpleGamePlugin, config).expect("autotest runs");
    // With PlaytestPlugin, oracles/bots run and accumulate frames.
    assert!(
        report.runs.iter().any(|r| r.frame_count > 0),
        "frames must be > 0: {:?}",
        report.runs
    );
}

/// FX4.5: run IDs unique + last pointer written.
#[test]
fn autotest_unique_run_ids_and_last_pointer() {
    // Use a unique output dir to isolate from other tests.
    let unique = format!("bevy_swarm/runs-fx4-{}", std::process::id());
    let target = std::env::var("CARGO_TARGET_DIR").unwrap_or_else(|_| "target".into());
    let runs_dir = std::path::Path::new(&target).join(&unique);
    let _ = std::fs::remove_dir_all(&runs_dir); // clean slate

    let config = AutotestConfig {
        num_seeds: 1,
        duration_s: 0.05,
        parallel: 1,
        minimize_failures: false,
        max_failures: None,
        output_dir: unique.into(),
        ..AutotestConfig::default()
    };
    autotest(SimpleGamePlugin, config.clone()).expect("first autotest");
    let mut first_runs: Vec<_> = std::fs::read_dir(&runs_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n != "last")
        .collect::<Vec<_>>();
    first_runs.sort();
    assert_eq!(first_runs.len(), 1, "exactly one run dir");

    // Second autotest — different run_id, last pointer updated.
    std::thread::sleep(std::time::Duration::from_millis(10));
    autotest(SimpleGamePlugin, config).expect("second autotest");
    let mut second_runs: Vec<_> = std::fs::read_dir(&runs_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n != "last")
        .collect::<Vec<_>>();
    second_runs.sort();
    assert_eq!(second_runs.len(), 2, "two distinct run dirs");
    assert_ne!(second_runs[0], second_runs[1], "run IDs differ");

    // Last pointer exists and points to the SECOND (newer) run.
    // ml5: run_id suffixes are splitmix64-hashed (not lexically
    // ordered), so identify the newer run by set difference, not by
    // sort position.
    let last_pointer = runs_dir.join("last");
    assert!(last_pointer.exists(), "last pointer exists");
    let last_content = std::fs::read_to_string(&last_pointer).unwrap();
    let newer: Vec<_> = second_runs
        .iter()
        .filter(|n| !first_runs.contains(n))
        .collect();
    assert_eq!(
        newer.len(),
        1,
        "exactly one new run dir from second autotest"
    );
    assert!(
        newer[0].contains(&last_content),
        "last points to second run: last={last_content} newer={}",
        newer[0]
    );
}

/// FX4.4: default raw surface non-empty.
#[test]
fn default_raw_surface_nonempty() {
    use bevy_swarm::scenario::RawSurface;
    let surface = RawSurface::default();
    assert!(!surface.keys.is_empty(), "default keys must be non-empty");
    // Movement keys only (arrows + WASD): tier-0 games frequently
    // ignore mouse/gamepad, and the dead_verb oracle would flag every
    // unhandled button — mouse/gamepad are opt-in per scenario.
    let expected = [
        "KeyW",
        "KeyA",
        "KeyS",
        "KeyD",
        "ArrowUp",
        "ArrowDown",
        "ArrowLeft",
        "ArrowRight",
    ];
    for k in expected {
        assert!(
            surface.keys.iter().any(|s| s == k),
            "default keys must contain {k}"
        );
    }
}

/// FX4.8: default persona is Uniform (Persona::default via #[default]).
#[test]
fn default_persona_uniform() {
    assert_eq!(
        bevy_swarm::enums::Persona::default(),
        bevy_swarm::enums::Persona::Uniform
    );
}

/// FX4: tier-0 chaos finds dead_left_key WITHOUT any adapter — the
/// walker reads keyboard via ButtonInput<KeyCode> and chaos samples
/// raw keys into RawActionQueue (actuated by the harness).
#[test]
#[serial]
fn autotest_finds_dead_left_key_without_adapter() {
    let _fixture_bug_guard = FixtureBugGuard::set(Some("dead_left_key"));
    // 60Hz over 6s: 360 samples, ~45 per key — well past the 10-sample
    // dead_verb threshold for the dead keys.
    let sc: bevy_swarm::scenario::Scenario = serde_json::from_str(
        r#"{"bot":{"type":"chaos","seed":1,"input_rate_hz":10},"duration_s":20.0,"invariants":[],"deny_ambiguities":false,"single_threaded":true}"#,
    ).unwrap();
    let config = AutotestConfig {
        num_seeds: 1,
        parallel: 1,
        base_scenario: sc,
        ..AutotestConfig::default()
    };
    let report = autotest(fixture_walker::WalkerGamePlugin, config).expect("autotest runs");
    assert!(
        report.failures.iter().any(|f| f.rule == "dead_verb"),
        "tier-0 chaos must find the dead left key: {:?}",
        report.failures
    );
}

/// FX4: clean walker passes tier-0 autotest.
#[test]
#[serial]
fn autotest_clean_walker_passes() {
    let sc: bevy_swarm::scenario::Scenario = serde_json::from_str(
        r#"{"bot":{"type":"chaos","seed":1,"input_rate_hz":10},"duration_s":20.0,"invariants":[],"deny_ambiguities":false,"single_threaded":true}"#,
    ).unwrap();
    let config = AutotestConfig {
        num_seeds: 1,
        parallel: 1,
        base_scenario: sc,
        ..AutotestConfig::default()
    };
    let report = autotest(fixture_walker::WalkerGamePlugin, config).expect("autotest runs");
    assert!(
        report.failures.is_empty(),
        "clean walker must pass tier-0: {:?}",
        report.failures
    );
}

/// FX4.3: a scenario rejected before running is counted as a FAILURE
/// by autotest (scenario_error rule, fingerprinted), not silently 0.
#[test]
fn autotest_counts_scenario_errors_as_failures() {
    // Planner bot without goals -> ScenarioError::Rejected at validate.
    let sc: bevy_swarm::scenario::Scenario = serde_json::from_str(
        r#"{"bot":{"type":"planner"},"duration_s":0.2,"invariants":[],"deny_ambiguities":false,"single_threaded":true}"#,
    )
    .unwrap();
    let config = AutotestConfig {
        num_seeds: 2,
        duration_s: 0.2,
        parallel: 1,
        base_scenario: sc,
        ..AutotestConfig::default()
    };
    let report = autotest(SimpleGamePlugin, config).expect("autotest runs");
    assert!(
        report.failures.iter().any(|f| f.rule == "scenario_error"),
        "scenario rejection must surface as failure: {:?}",
        report.failures
    );
    assert!(
        report.runs.iter().any(|r| r.status == "crash"),
        "rejected scenario runs are crash status: {:?}",
        report.runs
    );
}

/// le3.2: RAII guard that saves/restores FIXTURE_BUG on scope exit,
/// including panics mid-test (which would otherwise leak a set env
/// var into subsequent tests in this binary). Panics are caught via
/// Drop. Combined with #[serial], mutations are exception-safe and
/// serialized within the test binary.
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

// ---- RF8 (swarm-m2h.22): autotest false positives --------------------

#[derive(Resource, Default)]
struct Rf8Position {
    x: f32,
    y: f32,
}

#[derive(Clone, Default)]
struct ArrowsOnlyPlugin;

impl Plugin for ArrowsOnlyPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Rf8Position>().add_systems(
            Update,
            |keys: Res<ButtonInput<KeyCode>>, mut pos: ResMut<Rf8Position>| {
                if keys.pressed(KeyCode::ArrowUp) {
                    pos.y += 1.0;
                }
                if keys.pressed(KeyCode::ArrowDown) {
                    pos.y -= 1.0;
                }
                if keys.pressed(KeyCode::ArrowLeft) {
                    pos.x -= 1.0;
                }
                if keys.pressed(KeyCode::ArrowRight) {
                    pos.x += 1.0;
                }
            },
        );
    }
}

/// RF8 critical: an arrows-only clean game must PASS the default
/// autotest — unbound default-surface keys (WASD) are informational
/// unused_input, not dead_verb.
#[test]
#[serial]
fn rf8_arrows_only_passes_default_surface() {
    let config = AutotestConfig {
        num_seeds: 8,
        duration_s: 2.0,
        parallel: 2,
        ..AutotestConfig::default()
    };
    let report = autotest(ArrowsOnlyPlugin, config).expect("autotest runs");
    assert!(
        !report.failures.iter().any(|f| f.rule == "dead_verb"),
        "no dead_verb on unused WASD keys; failures: {:?}",
        report.failures.iter().map(|f| &f.rule).collect::<Vec<_>>()
    );
    for f in &report.failures {
        eprintln!("FAILURE {} {}", f.rule, f.fingerprint.0);
    }
    assert!(
        report.failures.is_empty(),
        "arrows-only game passes default autotest: {:#?}",
        report.failures
    );
}

/// RF8 moderate: a static/undrivable game must not report frozen_world
/// (Major) — the tier-0 surface was a guess, so the game is
/// undrivable-game territory (informational), not a gameplay bug.
#[test]
#[serial]
fn rf8_static_game_undrivable_not_frozen() {
    #[derive(Clone, Default)]
    struct StaticPlugin;
    impl Plugin for StaticPlugin {
        fn build(&self, _app: &mut App) {}
    }
    let config = AutotestConfig {
        num_seeds: 4,
        duration_s: 3.0,
        parallel: 2,
        ..AutotestConfig::default()
    };
    let report = autotest(StaticPlugin, config).expect("autotest runs");
    assert!(
        !report.failures.iter().any(|f| f.rule == "frozen_world"),
        "static game should not report frozen_world as Major"
    );
}

/// RF8 moderate: non-Clone plugins work via autotest_factory (no
/// Clone bound, game-type inference preserved).
#[test]
#[serial]
fn rf8_non_clone_plugin_via_factory() {
    use bevy_swarm::autotest::autotest_factory;

    struct NonClonePlugin {
        _data: std::sync::Mutex<Vec<u8>>,
    }
    impl Plugin for NonClonePlugin {
        fn build(&self, app: &mut App) {
            app.init_resource::<Rf8Position>().add_systems(
                Update,
                |keys: Res<ButtonInput<KeyCode>>, mut pos: ResMut<Rf8Position>| {
                    if keys.pressed(KeyCode::ArrowUp) {
                        pos.y += 1.0;
                    }
                },
            );
        }
    }

    let config = AutotestConfig {
        num_seeds: 2,
        duration_s: 1.0,
        parallel: 1,
        ..AutotestConfig::default()
    };
    let report = autotest_factory(
        || NonClonePlugin {
            _data: std::sync::Mutex::new(Vec::new()),
        },
        config,
    )
    .expect("autotest_factory runs");
    assert!(report.failures.is_empty(), "non-Clone plugin autotests");
}

/// RF8 moderate: coverage is serialized into report.json (was
/// #[serde(skip)]).
#[test]
#[serial]
fn rf8_coverage_serialized_in_report() {
    // Unique output dir so parallel/serial tests don't race.
    let config = AutotestConfig {
        num_seeds: 2,
        duration_s: 0.1,
        parallel: 1,
        output_dir: std::path::PathBuf::from("rf8-coverage-test"),
        ..AutotestConfig::default()
    };
    let report = autotest(ArrowsOnlyPlugin, config).expect("autotest runs");
    let json = serde_json::to_value(&report).expect("serialize report");
    assert!(
        json["coverage"].is_object(),
        "coverage serialized in report"
    );
    assert!(
        json["coverage"].get("intents_emitted").is_some(),
        "intents_emitted present in coverage"
    );
}

// ---------------------------------------------------------------------------
// Z18 (swarm-m2h.19): gate, regressions-first, schema, junit, per-seed.
// ---------------------------------------------------------------------------

/// A game that reliably fails: a dead_verb-free but crashing plugin is
/// overkill; use an always-violating invariant-less game via a bot on
/// an unreachable target. Simpler: a plugin whose system violates the
/// finite_transforms rule every frame.
struct FailingGamePlugin;

impl Plugin for FailingGamePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, |mut commands: Commands| {
            commands.spawn((
                bevy_swarm::contract::Gameplay,
                Name::new("bad"),
                Transform::default(),
            ));
        })
        .add_systems(
            Update,
            |mut q: Query<&mut Transform, With<bevy_swarm::contract::Gameplay>>| {
                for mut t in &mut q {
                    t.translation.x = f32::NAN;
                }
            },
        );
    }
}

fn z18_config(seeds: u64, dir_suffix: &str) -> AutotestConfig {
    AutotestConfig {
        num_seeds: seeds,
        duration_s: 0.05,
        parallel: 1,
        minimize_failures: false,
        max_failures: None,
        output_dir: std::path::PathBuf::from(format!(
            "bevy_swarm/runs-z18-{}-{}",
            dir_suffix,
            std::process::id()
        )),
        ..AutotestConfig::default()
    }
}

fn z18_runs_dir(cfg: &AutotestConfig) -> std::path::PathBuf {
    let target = std::env::var("CARGO_TARGET_DIR").unwrap_or_else(|_| "target".into());
    std::path::Path::new(&target).join(&cfg.output_dir)
}

/// Z18: isolate the persisted ledger (issues.json, regressions/) per
/// test. nextest runs each test in its OWN process, so #[serial] does
/// not prevent interference with the shared tests/swarm paths.
fn z18_isolate_state(suffix: &str) {
    let target = std::env::var("CARGO_TARGET_DIR").unwrap_or_else(|_| "target".into());
    let dir = std::path::Path::new(&target).join(format!("z18-state-{}", suffix));
    // Fresh state every time: stale issues.json/regressions/ from an
    // earlier run would flip classifications.
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create state dir");
    std::env::set_var("BEVY_SWARM_STATE_DIR", &dir);
}

/// Z18.F + T13: two consecutive autotests — the second classifies the
/// known fingerprint as `known` (gate.new empty) when the DB records it.
#[test]
#[serial]
fn z18_gate_second_run_knows_fingerprint() {
    use bevy_swarm::fingerprint::Fingerprint;
    use bevy_swarm::issues::{IssueDb, IssueRecord, IssueState};

    z18_isolate_state("gate");

    // First run: everything is new (empty DB). Record to DB as the
    // update mode would.
    let cfg = z18_config(1, "gate1");
    let first = autotest_with(|| FailingGamePlugin, cfg).expect("first autotest");
    assert!(
        !first.gate.classification.new.is_empty(),
        "first run must see NEW fingerprints: {:?}",
        first.gate.classification
    );

    // Populate the DB with the observed fingerprints (open).
    let mut db = IssueDb::default();
    let observed: Vec<(bevy_swarm::fingerprint::Fingerprint, u64, String, u64)> = first
        .failures
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
    db.record(&observed);
    db.save().expect("save DB");

    // Second run with the DB present: same fingerprint, now known.
    let cfg2 = z18_config(1, "gate2");
    let second = autotest_with(|| FailingGamePlugin, cfg2).expect("second autotest");
    assert!(
        second.gate.classification.new.is_empty(),
        "second run must classify the fingerprint as known: {:?}",
        second.gate.classification
    );
    assert!(second.gate.passed, "known-open must not fail the gate");

    // Restore (or remove) the DB so other tests see the pristine state.
    let _ = IssueRecord::default();
    let _ = IssueState::Open;
    let _ = Fingerprint(String::new());
}

/// Z18.E: a regressions-dir record replays FIRST (before seeds), and
/// its still_failing status appears in the report.
#[test]
#[serial]
fn z18_regressions_replayed_first() {
    use bevy_swarm::sweep::{write_regression, RegressionRecord};

    z18_isolate_state("reg");

    let cfg = z18_config(1, "reg");
    let report =
        autotest_with(|| FailingGamePlugin, cfg.clone()).expect("autotest for fingerprint");
    let fp = report
        .failures
        .first()
        .expect("failing game produces a failure fingerprint")
        .fingerprint
        .clone();

    // Persist a regression record for that fingerprint.
    let reg_dir = bevy_swarm::conventions::regressions_dir();
    let mut scen = cfg.base_scenario.clone();
    scen.duration_s = 0.05;
    write_regression(
        &RegressionRecord {
            schema_version: 1,
            fingerprint: fp,
            found_by_seeds: vec![7],
            status: "staging".into(),
            scenario: scen,
        },
        &reg_dir,
    )
    .expect("write regression record");

    // Re-run: the regression must be replayed and reported.
    let cfg2 = z18_config(1, "reg2");
    let report = autotest_with(|| FailingGamePlugin, cfg2).expect("second autotest");
    assert_eq!(
        report.regressions.len(),
        1,
        "regression record must be replayed"
    );
    assert_eq!(
        report.regressions[0].status, "still_failing",
        "the persisted regression must still fail"
    );
    // Cleanup: remove the record file so other tests are unaffected.
    for entry in std::fs::read_dir(&reg_dir).unwrap().flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with("regression_") && name.contains(&report.regressions[0].fingerprint.0) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Z18.B (amendment 2): the aggregate report.json validates against
/// the `autotest_report` definition in docs/report-schema.json.
#[test]
#[serial]
fn z18_autotest_report_matches_schema() {
    z18_isolate_state("schema");

    let cfg = z18_config(1, "schema");
    let report = autotest_with(|| FailingGamePlugin, cfg.clone()).expect("autotest runs");
    let run_dir = z18_runs_dir(&cfg).join(report.run_id.clone().expect("run id"));
    let json_text = std::fs::read_to_string(run_dir.join("report.json")).expect("read report.json");
    let value: serde_json::Value = serde_json::from_str(&json_text).expect("report parses");

    let schema: serde_json::Value =
        serde_json::from_str(include_str!("../docs/report-schema.json")).expect("schema parses");
    let wrapper = serde_json::json!({
        "$ref": "#/$defs/autotest_report",
        "$defs": schema["$defs"].clone()
    });
    let validator = jsonschema::validator_for(&wrapper).expect("schema compiles");
    let errors: Vec<String> = validator
        .iter_errors(&value)
        .map(|e| format!("{e} at {}", e.instance_path))
        .collect();
    assert!(
        errors.is_empty(),
        "autotest report does not match docs/report-schema.json#/$defs/autotest_report: {errors:#?}"
    );
}

/// Z18.B: per-seed PlaytestReports are written next to the aggregate.
#[test]
#[serial]
fn z18_per_seed_reports_written() {
    z18_isolate_state("seeds");

    let cfg = z18_config(2, "seeds");
    let cfg2 = cfg.clone();
    let report = autotest(SimpleGamePlugin, cfg).expect("autotest runs");
    let run_dir = z18_runs_dir(&cfg2).join(report.run_id.clone().expect("run id"));
    for seed in [1u64, 2u64] {
        let path = run_dir.join(format!("seed-{seed}.json"));
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        let v: serde_json::Value = serde_json::from_str(&text).expect("per-seed report parses");
        assert_eq!(
            v.get("schema_version").and_then(|s| s.as_u64()),
            Some(1),
            "per-seed report must be a PlaytestReport"
        );
    }
}

/// Z18 amendment (adversarial review): the junit.xml actually written
/// to the run dir parses with a STRICT parser and contains the
/// failing case.
#[test]
#[serial]
fn z18_junit_xml_strict_parse() {
    z18_isolate_state("junit");

    let cfg = z18_config(1, "junit");
    let cfg2 = cfg.clone();
    let report = autotest_with(|| FailingGamePlugin, cfg).expect("autotest runs");
    let run_dir = z18_runs_dir(&cfg2).join(report.run_id.clone().expect("run id"));
    let junit_text = std::fs::read_to_string(run_dir.join("junit.xml")).expect("junit.xml written");
    let doc = roxmltree::Document::parse(&junit_text)
        .unwrap_or_else(|e| panic!("junit.xml must parse with a strict parser: {e}"));
    let testsuites = doc.root_element();
    assert_eq!(testsuites.tag_name().name(), "testsuites");
    let suite = testsuites.first_element_child().expect("testsuite");
    let testcases: Vec<_> = suite.children().filter(|n| n.is_element()).collect();
    assert!(!testcases.is_empty(), "at least one testcase");
    // Failing game: at least one failure element present.
    let has_failure = testcases.iter().any(|tc| {
        tc.children()
            .any(|c| c.is_element() && c.tag_name().name() == "failure")
    });
    assert!(has_failure, "failing run must yield <failure> elements");
}
