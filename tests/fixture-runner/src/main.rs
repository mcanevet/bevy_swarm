//! Fixture runner CLI: run a fixture with a scenario and write the report

use fixture_runner::{headless_app, SpinnerAdapterPlugin, WalkerAdapterPlugin, SpawnerAdapterPlugin, write_report};
use bevy_swarm::scenario::Scenario;
use bevy_swarm::driver::{PlaytestPlugin, run_scenario};
use std::env;
use bevy_swarm::conventions;
use bevy_swarm::contract::{IntentSurface, SurfaceVariant};

fn main() {
    let fixture = env::args().nth(1).expect("usage: fixture-runner <spinner|walker|spawner>");
    let scenario_json = env::args().nth(2).unwrap_or_else(|| r#"{"bot":{"type":"chaos","seed":42},"duration_s":1.0,"invariants":[],"setup":{}}"#.to_string());
    
    let scenario: Scenario = serde_json::from_str(&scenario_json).expect("parse scenario");
    
    let mut app = headless_app();
    app.add_plugins(bevy_swarm::contract::TestConventionsPlugin);
    app.add_plugins(PlaytestPlugin);
    // Insert AFTER TestConventionsPlugin so it persists
    
    match fixture.as_str() {
        "spinner" => {
            app.insert_resource(IntentSurface::new(vec![SurfaceVariant::Wait]));
            app.add_plugins(SpinnerAdapterPlugin);
            app.add_plugins(fixture_spinner::SpinnerGamePlugin);
        }
        "walker" => {
            app.insert_resource(IntentSurface::new(vec![
                SurfaceVariant::Move,
                SurfaceVariant::Wait,
            ]));
            app.add_plugins(WalkerAdapterPlugin);
            app.add_plugins(fixture_walker::WalkerGamePlugin);
        }
        "spawner" => {
            app.insert_resource(IntentSurface::new(vec![SurfaceVariant::Wait]));
            app.add_plugins(SpawnerAdapterPlugin);
            app.add_plugins(fixture_spawner::SpawnerGamePlugin);
        }
        _ => panic!("unknown fixture: {}", fixture),
    }
    
    let report = run_scenario(&mut app, &scenario).expect("run scenario");
    let scenario_hash = serde_json::to_string(&scenario)
        .map(|s| {
            let mut h = 0xcbf29ce484222325u64;
            for b in s.bytes() { h = h.wrapping_mul(0x100000001b3).wrapping_add(b as u64); }
            h
        })
        .unwrap_or(0);
    let run_id = conventions::run_id(scenario_hash, scenario.bot.seed);
    let path = write_report(&report, &run_id);
    // runs/last convention: copy of the latest run dir (conventions §2)
    let last = bevy_swarm::conventions::runs_dir().join("last");
    let _ = std::fs::remove_dir_all(&last);
    if let Some(parent) = last.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::create_dir_all(&last);
    let _ = std::fs::copy(&path, last.join("report.json"));
    println!("Report written to {}", path.display());
    println!("Status: {:?}", report.status);
    if !report.violations.is_empty() {
        println!("Violations: {}", report.violations.len());
        for v in &report.violations {
            println!("  - {}: {}", v.rule, v.detail);
        }
    }
}
