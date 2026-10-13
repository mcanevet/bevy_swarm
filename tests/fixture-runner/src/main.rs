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
    let mut scaffold: Option<String> = None;
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
        } else if args[i] == "--scaffold" {
            scaffold = Some(
                args.get(i + 1)
                    .expect("--scaffold requires an argument (adapter|none)")
                    .clone(),
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
    // Z18 (swarm-m2h.19): scaffold "none" — the zero-contract path.
    // The app is the fixture's PLAIN game plugin: no adapter, no
    // IntentSurface, no TestConventionsPlugin. Z18 (swarm-m2h.19):
    // run through bevy_swarm::autotest — headless_app_factory adds
    // PlaytestPlugin and Z6 GameTypes inference automatically. The
    // run dir receives report.json (AutotestReport), one
    // seed-<n>.json PlaytestReport per seed, and junit.xml.
    if scaffold.as_deref() == Some("none") || fixture == "lone" {
        let seeds: Vec<u64> = vec![scenario.bot.seed];
        let base = scenario;
        let config = bevy_swarm::AutotestConfig {
            seeds: Some(seeds),
            duration_s: base.duration_s,
            parallel: 1,
            minimize_failures: false,
            max_failures: None,
            base_scenario: base,
            ..bevy_swarm::AutotestConfig::default()
        };
        macro_rules! plain_autotest {
            ($plugin:expr) => {{
                let report = bevy_swarm::autotest_with(|| $plugin, config)
                    .expect("autotest (scaffold none)");
                if let Some(out) = &out_path {
                    if let Some(parent) = out.parent() {
                        std::fs::create_dir_all(parent)
                            .unwrap_or_else(|e| panic!("create out dir {}: {e}", parent.display()));
                    }
                    // Aggregate report at --out; per-seed PlaytestReports
                    // next to it so acceptance can check every seed.
                    std::fs::write(
                        out,
                        serde_json::to_string_pretty(&report).expect("serialize autotest report"),
                    )
                    .unwrap_or_else(|e| panic!("write {}: {e}", out.display()));
                    let target_dir =
                        std::env::var("CARGO_TARGET_DIR").unwrap_or_else(|_| "target".into());
                    let run_dir = std::path::Path::new(&target_dir)
                        .join("bevy_swarm/runs")
                        .join(report.run_id.clone().expect("run id"));
                    let mut entries: Vec<_> = std::fs::read_dir(&run_dir)
                        .expect("run dir exists")
                        .flatten()
                        .map(|e| e.path())
                        .filter(|p| {
                            p.file_name()
                                .map(|n| n.to_string_lossy().starts_with("seed-"))
                                .unwrap_or(false)
                        })
                        .collect();
                    entries.sort();
                    for p in entries {
                        let dest = out
                            .parent()
                            .expect("out parent")
                            .join(p.file_name().expect("file name"));
                        std::fs::copy(&p, &dest).unwrap_or_else(|e| {
                            panic!("copy {} -> {}: {e}", p.display(), dest.display())
                        });
                    }
                }
                println!("Status: {}", if report.passed() { "pass" } else { "fail" });
                println!(
                    "Runs: {} ({} failing)",
                    report.budget.seeds_run,
                    report.runs.iter().filter(|r| r.status != "pass").count()
                );
                return;
            }};
        }
        match fixture.as_str() {
            "lone" => plain_autotest!(fixture_lone::LoneGamePlugin),
            "walker" => plain_autotest!(fixture_walker::WalkerGamePlugin),
            "spinner" => plain_autotest!(fixture_spinner::SpinnerGamePlugin),
            "spawner" => plain_autotest!(fixture_spawner::SpawnerGamePlugin),
            "turn_based" => plain_autotest!(fixture_turn_based::TurnBasedGamePlugin),
            "regression_pair" => {
                plain_autotest!(fixture_regression_pair::RegressionPairGamePlugin)
            }
            _ => panic!("unknown fixture: {}", fixture),
        }
    }

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
    emit_report(&report, &scenario, out_path);
}

/// Shared report-writing tail for both scaffold paths: canonical run
/// dir, --out override, runs/last pointer, status summary.
fn emit_report(
    report: &bevy_swarm::driver::PlaytestReport,
    scenario: &Scenario,
    out_path: Option<std::path::PathBuf>,
) {
    let scenario_hash = serde_json::to_string(scenario)
        .map(|s| {
            let mut h = 0xcbf29ce484222325u64;
            for b in s.bytes() {
                h = h.wrapping_mul(0x100000001b3).wrapping_add(b as u64);
            }
            h
        })
        .unwrap_or(0);
    let run_id = conventions::run_id(scenario_hash, scenario.bot.seed);
    let path = write_report(report, &run_id);
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
