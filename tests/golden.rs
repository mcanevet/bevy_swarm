//! Z11: golden trajectory tests — record, check, divergence detection,
//! update-env approval workflow.

use bevy::prelude::*;
use bevy_swarm::contract::TestApi;
use bevy_swarm::golden::{check_golden, record_golden, GoldenOutcome, GoldenSet, Tolerance};
use bevy_swarm::headless::headless_app;

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
        app.add_plugins(bevy_swarm::driver::PlaytestPlugin);
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

fn scenario(seed: u64) -> bevy_swarm::scenario::Scenario {
    serde_json::from_str(&format!(
        r#"{{"bot":{{"type":"chaos","seed":{}}},"duration_s":0.05}}"#,
        seed
    ))
    .unwrap()
}

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("bevy_swarm_golden_{tag}"));
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
        app.insert_resource(bevy_swarm::determinism::StateTrace::default());
        let rep = bevy_swarm::driver::run_scenario(app, &scen).unwrap();
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
fn golden_update_env_rewrites() {
    // FX9: convention is BEVY_SWARM_UPDATE=golden, not
    // BEVY_SWARM_UPDATE_GOLDEN.
    unsafe { std::env::set_var("BEVY_SWARM_UPDATE", "golden") };
    assert!(bevy_swarm::golden::update_golden_enabled());
    unsafe { std::env::remove_var("BEVY_SWARM_UPDATE") };
    assert!(!bevy_swarm::golden::update_golden_enabled());
}
