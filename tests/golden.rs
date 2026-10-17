//! Z11: golden trajectory tests — record, check, divergence detection,
//! update-env approval workflow.

use bevy::prelude::*;
use bevy_playtest::contract::TestApi;
use bevy_playtest::golden::{check_golden, record_golden, GoldenOutcome, GoldenSet, Tolerance};
use bevy_playtest::headless::headless_app;

/// Damage formula flag: version A (original) vs B (changed logic).
/// FX9: per-app resource — a global static toggled mid-run by other
/// tests made this suite flaky under parallel execution.
#[derive(Resource, Clone, Copy)]
struct DamageV2(bool);

#[derive(Resource, Default)]
struct EnemyDamage(u32);

fn damage_system(mut dmg: ResMut<EnemyDamage>, v2: Option<Res<DamageV2>>) {
    // Deterministic damage: 1 per frame in v1, 2 in v2 (logic change).
    let inc = if v2.is_some_and(|f| f.0) { 2 } else { 1 };
    dmg.0 += inc;
}

fn sync_damage_to_api(mut api: ResMut<TestApi>, dmg: Res<EnemyDamage>) {
    if let Some(entry) = api.custom_numeric.get_mut("Damage.Accumulated") {
        *entry = dmg.0 as f64;
    }
}

#[derive(Clone, Default)]
struct GamePlugin;

impl Plugin for GamePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<EnemyDamage>();
        app.add_systems(Update, damage_system);
        app.add_systems(Last, sync_damage_to_api);
        app.add_plugins(bevy_playtest::driver::PlaytestPlugin);
        app.insert_resource(TestApi {
            score: 0,
            active_players: 0,
            custom_numeric: [(String::from("Damage.Accumulated"), 0.0)]
                .into_iter()
                .collect(),
            custom_text: Default::default(),
        });
    }
}

fn game_v1() -> App {
    headless_app(GamePlugin)()
}

fn game_v2() -> App {
    let mut app = headless_app(GamePlugin)();
    app.insert_resource(DamageV2(true));
    app
}

fn scenario(seed: u64) -> bevy_playtest::scenario::Scenario {
    serde_json::from_str(&format!(
        r#"{{"bot":{{"type":"chaos","seed":{}}},"duration_s":0.05}}"#,
        seed
    ))
    .unwrap()
}

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("bevy_playtest_golden_{tag}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn golden_same_on_unchanged_game() {
    let dir = temp_dir("same");
    let scen = scenario(42);

    // Cross-check determinism: two fresh runs of the same scenario
    let d = |app: &mut App| {
        app.insert_resource(bevy_playtest::determinism::StateTrace::default());
        let rep = bevy_playtest::driver::run_scenario(app, &scen).unwrap();
        rep.state_trace
            .iter()
            .flatten()
            .map(|fd| format!("{:016x}", fd.hash))
            .collect::<Vec<_>>()
    };
    let mut a1 = headless_app(GamePlugin)();
    let d1 = d(&mut a1);
    let mut a2 = headless_app(GamePlugin)();
    let d2 = d(&mut a2);
    eprintln!("run1: {:?}", d1);
    eprintln!("run2: {:?}", d2);
    eprintln!("identical: {}", d1 == d2);
    record_golden(game_v1, std::slice::from_ref(&scen), &dir).unwrap();

    let set = GoldenSet {
        scenarios: vec![("scenario-0-42".to_string(), scen.clone())],
        dir: dir.clone(),
    };
    let report = check_golden(game_v1, &set, &Tolerance::default()).unwrap();
    assert!(
        report.all_same(),
        "unchanged game must be Same: {:?}",
        report.per_scenario
    );
}

#[test]
fn golden_detects_logic_change() {
    let dir = temp_dir("change");
    let scen = scenario(7);

    record_golden(game_v1, std::slice::from_ref(&scen), &dir).unwrap();

    // Changed damage formula: same inputs, different outcome.
    let set = GoldenSet {
        scenarios: vec![("scenario-0-7".to_string(), scen.clone())],
        dir: dir.clone(),
    };
    let report = check_golden(game_v2, &set, &Tolerance::default()).unwrap();
    assert!(!report.all_same(), "logic change must diverge");

    let (_, outcome) = &report.per_scenario[0];
    match outcome {
        GoldenOutcome::Diverged { first_frame, .. } => {
            assert!(
                *first_frame < 3,
                "divergence should be early, got {first_frame}"
            )
        }
        other => panic!("expected Diverged, got {other:?}"),
    }
}

#[test]
fn golden_missing_is_not_same() {
    let dir = temp_dir("missing");
    let set = GoldenSet {
        scenarios: vec![("never-recorded".to_string(), scenario(1))],
        dir: dir.clone(),
    };
    let report = check_golden(game_v1, &set, &Tolerance::default()).unwrap();
    assert!(!report.all_same());
    assert!(matches!(report.per_scenario[0].1, GoldenOutcome::Missing));
}

#[test]
#[serial_test::serial]
fn golden_update_env_rewrites() {
    // FX9: convention is BEVY_PLAYTEST_UPDATE=golden, not
    // BEVY_PLAYTEST_UPDATE_GOLDEN.
    unsafe { std::env::set_var("BEVY_PLAYTEST_UPDATE", "golden") };
    assert!(bevy_playtest::golden::update_golden_enabled());
    unsafe { std::env::remove_var("BEVY_PLAYTEST_UPDATE") };
    assert!(!bevy_playtest::golden::update_golden_enabled());
}

#[test]
fn golden_path_sanitization_rejects_traversal() {
    // H1: scenario.name is embedded into golden-set file names.
    // Reject anything outside [A-Za-z0-9_-] to prevent path traversal.
    let bad_names = vec![
        "../../etc/passwd",
        "..\\windows\\system32",
        "foo/bar",
        "foo\\bar",
        "foo.bar",
        "foo bar",
        "",
    ];
    for name in bad_names {
        let mut scen = scenario(42);
        scen.name = Some(name.to_string());
        let err = bevy_playtest::scenario::validate_scenario(&scen)
            .expect_err("must reject invalid name");
        assert!(
            err.to_string().contains("invalid"),
            "rejected for wrong reason: {err}"
        );
    }

    // Valid names pass.
    let good_names = vec!["clean", "bug_dead_left_key", "test_123", "a"];
    for name in good_names {
        let mut scen = scenario(42);
        scen.name = Some(name.to_string());
        bevy_playtest::scenario::validate_scenario(&scen).expect("must accept valid name");
    }
}

#[test]
fn scenario_duration_tps_caps_reject_dos() {
    // H2: tps/duration combos that imply enormous tick counts must be
    // rejected at validation time.
    let reject = |tps: u32, dur: f64| {
        let scen: bevy_playtest::scenario::Scenario = serde_json::from_str(&format!(
            r#"{{"bot":{{"type":"chaos","seed":1}},"tps":{tps},"duration_s":{dur}}}"#
        ))
        .unwrap();
        bevy_playtest::scenario::validate_scenario(&scen).expect_err("must reject");
    };

    reject(1_001, 1.0); // tps over cap
    reject(1_000_000, 1.0); // tps way over cap
    reject(60, 3_600.1); // duration over cap
    reject(4294967295, 3.4e38); // the original hostile scenario
    reject(5000, 2000.0); // over both tps cap and 10M tick cap
    reject(100, 200_000.0); // 20M ticks — over tick cap only

    // Boundary: 1000 tps x 3600 s = 3.6M ticks is accepted.
    let ok: bevy_playtest::scenario::Scenario =
        serde_json::from_str(r#"{"bot":{"type":"chaos","seed":1},"tps":1000,"duration_s":3600.0}"#)
            .unwrap();
    bevy_playtest::scenario::validate_scenario(&ok).expect("boundary combo must be accepted");
}
