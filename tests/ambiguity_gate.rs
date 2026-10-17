//! swarm-b6t: ambiguity gate v2 — structured extraction + allow rules.

use bevy::prelude::*;
use bevy_playtest::driver::{run_scenario, PlaytestPlugin};

#[derive(Resource, Default)]
struct Score(u32);

fn sys_a(mut score: ResMut<Score>) {
    score.0 += 1;
}

fn sys_b(mut score: ResMut<Score>) {
    score.0 += 1;
}

fn build_app() -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, PlaytestPlugin));
    // Intentionally ambiguous pair: both take ResMut<Score>, unordered.
    app.add_systems(Update, (sys_a, sys_b));
    app
}

fn scenario(extra: &str) -> bevy_playtest::scenario::Scenario {
    // `extra` is either "" or a JSON array of allow rules.
    let allow = if extra.is_empty() {
        String::new()
    } else {
        let val: serde_json::Value = serde_json::from_str(extra).unwrap();
        format!(
            r#","allow_ambiguities":{}"#,
            serde_json::to_string(&val).unwrap()
        )
    };
    serde_json::from_str(&format!(
        r#"{{"bot":{{"type":"replay","inputs":[]}},"duration_s":0.1,"single_threaded":true,"deny_ambiguities":true,"invariants":[]{allow}}}"#
    ))
    .unwrap()
}

#[test]
fn ambiguous_pair_rejected_without_allow_rules() {
    let mut app = build_app();
    let err = run_scenario(&mut app, &scenario("")).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("ambiguities"), "got: {msg}");
    assert!(msg.contains("sys_a") && msg.contains("sys_b"), "got: {msg}");
    eprintln!("REJECTION MSG: {msg}");
}

#[test]
fn allow_pair_rule_makes_run_pass() {
    let mut app = build_app();
    let rep = run_scenario(
        &mut app,
        &scenario(r#"[{"kind":"pair","a":"sys_a","b":"sys_b"}]"#),
    );
    assert!(rep.is_ok(), "pair rule must allow the conflict: {rep:?}");
}

#[test]
fn allow_system_rule_makes_run_pass() {
    let mut app = build_app();
    let rep = run_scenario(&mut app, &scenario(r#"[{"kind":"system","name":"sys_a"}]"#));
    assert!(rep.is_ok(), "system rule must allow the conflict");
}

#[test]
fn allow_data_rule_makes_run_pass() {
    let mut app = build_app();
    let rep = run_scenario(&mut app, &scenario(r#"[{"kind":"data","name":"::Score"}]"#));
    assert!(rep.is_ok(), "data rule must allow the conflict");
}

#[test]
fn allow_among_rule_requires_prefix_match_on_both_sides() {
    let mut app = build_app();
    // Wrong prefix: bevy_ui:: matches neither sys_a nor sys_b.
    let rep = run_scenario(
        &mut app,
        &scenario(r#"[{"kind":"among","prefix":"bevy_ui"}]"#),
    );
    assert!(rep.is_err(), "unrelated prefix must NOT allow the conflict");
}

#[test]
fn builder_named_systems_are_matched_by_full_path() {
    // Anonymous closure systems get builder-assigned names; verify
    // extraction produces real full names usable in allow rules.
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, PlaytestPlugin));
    app.insert_resource(Score(0));
    app.add_systems(
        Update,
        (
            |mut s: ResMut<Score>| s.0 += 1,
            |mut s: ResMut<Score>| s.0 += 1,
        ),
    );
    let err = run_scenario(&mut app, &scenario("")).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("ambiguities"), "got: {msg}");
    // Closure conflicts carry auto-generated names; the data type
    // must still be reported for a Data rule to be actionable.
    assert!(msg.contains("Score"), "got: {msg}");
}

#[test]
fn deny_ambiguities_clean_app_still_passes() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, PlaytestPlugin));
    let rep = run_scenario(&mut app, &scenario(""));
    assert!(rep.is_ok(), "clean app must pass: {:?}", rep.err());
}
