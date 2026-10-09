//! Fixture runner CLI: run a fixture with a scenario and write the report

use bevy_swarm::contract::{IntentSurface, SurfaceVariant};
use bevy_swarm::conventions;
use bevy_swarm::driver::PlaytestPlugin;
use bevy_swarm::scenario::Scenario;
use fixture_runner::{
    headless_app, write_report, RegressionPairAdapterPlugin, SpawnerAdapterPlugin,
    SpinnerAdapterPlugin, TurnBasedAdapterPlugin, WalkerAdapterPlugin,
};
use std::env;

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let fixture = args
        .first()
        .expect("usage: fixture-runner <fixture> [scenario] [--out <path>]")
        .clone();
    // FX2: optional --out <path> — unique per-case report destination,
    // so parallel invocations never race on a shared path.
    let mut out_path: Option<std::path::PathBuf> = None;
    let mut positional: Vec<String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--out" {
            out_path = Some(
                args.get(i + 1)
                    .expect("--out requires a path argument")
                    .into(),
            );
            i += 2;
        } else {
            positional.push(args[i].clone());
            i += 1;
        }
    }
    let scenario_json = positional.get(1).cloned().unwrap_or_else(|| {
        r#"{"bot":{"type":"chaos","seed":42},"duration_s":1.0,"invariants":[],"setup":{}}"#
            .to_string()
    });

    let scenario: Scenario = serde_json::from_str(&scenario_json).expect("parse scenario");

    // FX12 (E1): build the app via a factory closure, then run through
    // InProcess (ScenarioRunner trait) — the binary now exercises the
    // same execution path as matrix/sweep consumers.
    let build_app = || {
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
            "turn_based" => {
                app.insert_resource(IntentSurface::new(vec![
                    SurfaceVariant::Choice(3),
                    SurfaceVariant::Wait,
                ]));
                app.add_plugins(TurnBasedAdapterPlugin);
                app.add_plugins(fixture_turn_based::TurnBasedGamePlugin);
            }
            "regression_pair" => {
                app.insert_resource(IntentSurface::new(vec![SurfaceVariant::Wait]));
                app.add_plugins(RegressionPairAdapterPlugin);
                app.add_plugins(fixture_regression_pair::RegressionPairGamePlugin);
            }
            _ => panic!("unknown fixture: {}", fixture),
        }
        app
    };

    let runner = bevy_swarm::branch::InProcess { factory: build_app };
    let report = bevy_swarm::branch::ScenarioRunner::run(&runner, &scenario).expect("run scenario");
    let scenario_hash = serde_json::to_string(&scenario)
        .map(|s| {
            let mut h = 0xcbf29ce484222325u64;
            for b in s.bytes() {
                h = h.wrapping_mul(0x100000001b3).wrapping_add(b as u64);
            }
            h
        })
        .unwrap_or(0);
    let run_id = conventions::run_id(scenario_hash, scenario.bot.seed);
    let path = write_report(&report, &run_id);
    // FX2: --out overrides the report destination (unique per-case
    // path for the acceptance harness). Errors are FATAL — a swallowed
    // copy error meant stale-report reads downstream.
    if let Some(out) = out_path {
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)
                .unwrap_or_else(|e| panic!("create out dir {}: {}", parent.display(), e));
        }
        std::fs::write(
            &out,
            serde_json::to_string_pretty(&report).expect("serialize"),
        )
        .unwrap_or_else(|e| panic!("write {}: {}", out.display(), e));
    }
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
