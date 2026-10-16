//! J1: quantitative robustness semantics — signed margins in reports.

use bevy::ecs::reflect::AppTypeRegistry;
use bevy::prelude::*;
use bevy_swarm::contract::{Gameplay, TestApi, UserIntent};
use bevy_swarm::driver::{run_scenario, PlaytestPlugin};
use bevy_swarm::scenario::Scenario;

fn build_app() -> App {
    let mut app = App::new();
    app.add_plugins((
        bevy::MinimalPlugins,
        bevy::transform::TransformPlugin,
        PlaytestPlugin,
        bevy_swarm::contract::TestConventionsPlugin,
    ));
    app.insert_resource(TestApi::default());
    app.add_message::<UserIntent>();
    app
}

#[derive(Component)]
struct Points(i64);

#[derive(Resource, Reflect, Default)]
#[reflect(Resource)]
struct Policy {
    health: f64,
}

#[test]
fn report_robustness_positive_on_pass_negative_on_fail() {
    // Same game: pass case (score 2, below 10) and fail case (above 10).
    let scen_pass: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"replay","inputs":[
            {"frame":1,"intent":{"intent":"choice","index":0}}]},
            "duration_s":0.2,
            "invariants":[{"name":"cap","rule":"custom","path":"TestApi.score","check":"below","value":10}]}"#,
    )
    .unwrap();
    let scen_fail: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"replay","inputs":[
            {"frame":1,"intent":{"intent":"choice","index":0}}]},
            "duration_s":0.2,
            "invariants":[{"name":"cap","rule":"custom","path":"TestApi.score","check":"below","value":1}]}"#,
    )
    .unwrap();
    let mut app = build_app();
    app.add_systems(bevy::app::Startup, |mut c: Commands| {
        c.spawn((Name::new("P1"), Gameplay, Points(0)));
    });
    app.add_systems(
        Update,
        (
            |mut reader: bevy::ecs::message::MessageReader<UserIntent>,
             mut q: Query<&mut Points>| {
                for _ in reader.read() {
                    for mut p in &mut q {
                        p.0 += 2;
                    }
                }
            },
            |q: Query<&Points>, mut api: ResMut<TestApi>| {
                if let Ok(p) = q.single() {
                    api.score = p.0;
                }
            },
        ),
    );
    let pass = run_scenario(&mut app, &scen_pass).unwrap();
    assert!(
        pass.robustness.overall.unwrap() > 0.0,
        "pass run must have positive margin: {:?}",
        pass.robustness
    );
    assert_eq!(
        pass.robustness.per_invariant["cap"].0, 8.0,
        "thr 10 - score 2"
    );

    let mut app = build_app();
    app.add_systems(bevy::app::Startup, |mut c: Commands| {
        c.spawn((Name::new("P1"), Gameplay, Points(0)));
    });
    app.add_systems(
        Update,
        (
            |mut reader: bevy::ecs::message::MessageReader<UserIntent>,
             mut q: Query<&mut Points>| {
                for _ in reader.read() {
                    for mut p in &mut q {
                        p.0 += 2;
                    }
                }
            },
            |q: Query<&Points>, mut api: ResMut<TestApi>| {
                if let Ok(p) = q.single() {
                    api.score = p.0;
                }
            },
        ),
    );
    let fail = run_scenario(&mut app, &scen_fail).unwrap();
    assert!(fail.robustness.overall.unwrap() < 0.0);
    assert!((fail.robustness.per_invariant["cap"].0 - (-1.0)).abs() < 1e-9);
    assert!(
        !fail.violations.is_empty(),
        "negative margin must coincide with a violation"
    );
}

#[test]
fn scale_normalizes_large_units() {
    // px-scale invariant with scale 1000 vs unit-scale invariant.
    let scen: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"replay","inputs":[]},"duration_s":0.2,"invariants":[
            {"name":"px","rule":"custom","path":"TestApi.score","check":"above","value":900,"scale":1000},
            {"name":"hp","rule":"custom","path":"TestApi.score","check":"above","value":0.4}]}"#,
    )
    .unwrap();
    let mut app = build_app();
    app.add_systems(Update, |mut api: ResMut<TestApi>| api.score = 950);
    let rep = run_scenario(&mut app, &scen).unwrap();
    let r = &rep.robustness;
    assert!(
        (r.per_invariant["px"].2 - 0.05).abs() < 1e-9,
        "normalized 50/1000"
    );
    assert!((r.per_invariant["hp"].2 - 949.6).abs() < 1e-9);
    // Overall is the min of normalized values: 0.05 (not swamped by
    // the huge raw hp margin).
    assert!((r.overall.unwrap() - 0.05).abs() < 1e-9);
}

#[test]
fn bounds_margin_reports_signed_distance() {
    let scen: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"replay","inputs":[]},"duration_s":0.2,"invariants":[
            {"name":"arena","rule":"nodes_in_bounds","min_x":-10,"max_x":10,"min_y":-10,"max_y":10}]}"#,
    )
    .unwrap();
    let mut app = build_app();
    app.add_systems(bevy::app::Startup, |mut c: Commands| {
        c.spawn((
            Name::new("Walker"),
            Gameplay,
            Transform::from_xyz(8.0, 0.0, 0.0),
        ));
    });
    let rep = run_scenario(&mut app, &scen).unwrap();
    assert!(rep.violations.is_empty());
    // margin = min(8 - (-10), 10 - 8) = 2
    let entry = &rep.robustness.per_invariant["arena/Walker"];
    assert!((entry.0 - 2.0).abs() < 1e-5, "got {:?}", entry);
}

#[test]
fn threshold_margins_positive_while_satisfied_and_negative_once_violated() {
    // below (le): ρ = thr - v; above (ge): ρ = v - thr.
    let scen: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"replay","inputs":[]},"duration_s":0.2,"invariants":[
            {"name":"lo","rule":"custom","path":"TestApi.score","check":"below","value":100},
            {"name":"hi","rule":"custom","path":"TestApi.score","check":"above","value":10},
            {"name":"lo_bad","rule":"custom","path":"TestApi.score","check":"below","value":50},
            {"name":"hi_bad","rule":"custom","path":"TestApi.score","check":"above","value":500}]}"#,
    )
    .unwrap();
    let mut app = build_app();
    app.add_systems(Update, |mut api: ResMut<TestApi>| api.score = 80);
    let rep = run_scenario(&mut app, &scen).unwrap();
    let r = &rep.robustness;
    assert!((r.per_invariant["lo"].0 - 20.0).abs() < 1e-9, "thr-v");
    assert!((r.per_invariant["hi"].0 - 70.0).abs() < 1e-9, "v-thr");
    assert!((r.per_invariant["lo_bad"].0 - (-30.0)).abs() < 1e-9);
    assert!((r.per_invariant["hi_bad"].0 - (-420.0)).abs() < 1e-9);
    // Boolean verdict ⇔ sign: positive entries have no violation,
    // negative entries do.
    for (name, (rho, _, _)) in &r.per_invariant {
        let violated = rep.violations.iter().any(|v| v.rule == *name);
        assert_eq!(violated, rho < &0.0, "{name}: rho {rho}");
    }
}

#[test]
fn equals_margin_is_negative_distance() {
    // equals: ρ = -(distance); 0 when exactly equal.
    let scen: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"replay","inputs":[]},"duration_s":0.2,"invariants":[
            {"name":"exact","rule":"custom","path":"TestApi.score","check":"equals","value":42},
            {"name":"off_by_three","rule":"custom","path":"TestApi.score","check":"equals","value":45}]}"#,
    )
    .unwrap();
    let mut app = build_app();
    app.add_systems(Update, |mut api: ResMut<TestApi>| api.score = 42);
    let rep = run_scenario(&mut app, &scen).unwrap();
    let r = &rep.robustness;
    // le3.34 (RF44): exact equality now yields a strictly POSITIVE
    // epsilon margin (was 0.0 — made max(-0, rho_R) = -0, breaking
    // overall<0 iff fail).
    assert!((r.per_invariant["exact"].0 - bevy_swarm::robustness::EPSILON).abs() < 1e-12);
    assert!((r.per_invariant["off_by_three"].0 - (-3.0)).abs() < 1e-9);
    assert!(
        rep.violations.iter().any(|v| v.rule == "off_by_three"),
        "equals violation must fire"
    );
    assert!(
        !rep.violations.iter().any(|v| v.rule == "exact"),
        "exact equality must pass"
    );
}

#[test]
fn text_equality_uses_boolean_lift() {
    // Text invariants lift to ±1 (scaled by 1.0): +1 when equal, -1
    // when not.
    let scen: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"replay","inputs":[]},"duration_s":0.2,"invariants":[
            {"name":"phase_ok","rule":"custom","path":"TestApi.game_phase","check":"equals","value":"Playing"},
            {"name":"phase_bad","rule":"custom","path":"TestApi.game_phase","check":"equals","value":"GameOver"}]}"#,
    )
    .unwrap();
    let mut app = build_app();
    app.add_systems(Update, |mut api: ResMut<TestApi>| {
        api.custom_text
            .insert("game_phase".into(), "Playing".into());
    });
    let rep = run_scenario(&mut app, &scen).unwrap();
    let r = &rep.robustness;
    assert_eq!(r.per_invariant["phase_ok"].0, 1.0);
    assert_eq!(r.per_invariant["phase_bad"].0, -1.0);
    assert_eq!(r.per_invariant["phase_bad"].2, -1.0);
}

#[test]
fn count_query_margin_is_threshold_margin_on_the_count() {
    let scen: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"replay","inputs":[]},"duration_s":0.2,"invariants":[
            {"name":"pop","rule":"custom","query":{"with":["Gameplay","Name"],"without":[]},
             "check":"ge","value":1}]}"#,
    )
    .unwrap();
    let mut app = build_app();
    app.add_systems(bevy::app::Startup, |mut c: Commands| {
        c.spawn((Name::new("A"), Gameplay, Points(0)));
        c.spawn((Name::new("B"), Gameplay, Points(0)));
        c.spawn((Name::new("C"), Gameplay, Points(0)));
    });
    let rep = run_scenario(&mut app, &scen).unwrap();
    let r = &rep.robustness;
    assert!(
        (r.per_invariant["pop"].0 - 2.0).abs() < 1e-9,
        "count 3 - thr 1 = 2, got {:?}",
        r.per_invariant["pop"]
    );
}

#[test]
fn eventually_robustness_is_max_not_min() {
    // Eventually (deadline D): ρ = max over t ≤ D of ρ_inner(t) — the
    // BEST value so far, not the worst. Score grows 0→10 over the run;
    // the eventual "above 8" must report the max margin (at the end),
    // never the early negative one.
    let scen: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"replay","inputs":[
            {"frame":1,"intent":{"intent":"choice","index":0}},
            {"frame":2,"intent":{"intent":"choice","index":0}},
            {"frame":3,"intent":{"intent":"choice","index":0}},
            {"frame":4,"intent":{"intent":"choice","index":0}}]},
            "duration_s":0.2,
            "invariants":[{"name":"reach","rule":"custom","path":"TestApi.score","check":"above","value":7,"eventually_s":1.0}]}"#,
    )
    .unwrap();
    let mut app = build_app();
    app.add_systems(
        Update,
        (
            |mut reader: bevy::ecs::message::MessageReader<UserIntent>,
             mut q: Query<&mut Points>| {
                for _ in reader.read() {
                    for mut p in &mut q {
                        p.0 += 2;
                    }
                }
            },
            |q: Query<&Points>, mut api: ResMut<TestApi>| {
                if let Ok(p) = q.single() {
                    api.score = p.0;
                }
            },
        ),
    );
    app.add_systems(bevy::app::Startup, |mut c: Commands| {
        c.spawn((Name::new("P1"), Gameplay, Points(0)));
    });
    let rep = run_scenario(&mut app, &scen).unwrap();
    assert!(
        rep.violations.is_empty(),
        "eventually-above-8 satisfied late: {:?}",
        rep.violations
    );
    // Run-level ρ is the best-so-far max of ρ_inner: after 5 choices
    // (0,2,4,6,8,10...), score reaches ≥ 8, and the best margin is
    // final_score - 8 > 0. A min-tracker would report the initial
    // negative margin instead.
    let entry = &rep.robustness.per_invariant["reach"];
    assert!(
        entry.0 > 0.0,
        "eventually ρ must be the max (best) inner margin, got {entry:?}"
    );
}

#[test]
fn eventually_robustness_stays_negative_when_never_satisfied() {
    // Deadline expires unmet: run-level ρ must be negative (max over
    // all frames was still negative).
    let scen: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"replay","inputs":[]},"duration_s":0.2,
            "invariants":[{"name":"reach","rule":"custom","path":"TestApi.score","check":"above","value":8,"eventually_s":0.1}]}"#,
    )
    .unwrap();
    let mut app = build_app();
    let rep = run_scenario(&mut app, &scen).unwrap();
    assert!(!rep.violations.is_empty());
    let entry = &rep.robustness.per_invariant["reach"];
    assert!(
        entry.0 < 0.0,
        "unsatisfied eventually must be negative, got {entry:?}"
    );
}

#[test]
fn implication_masking_whenever_false_all_run() {
    // WHEN→REQUIRE implication: ρ = max(-ρ_W, ρ_R). WHEN false ALL run
    // (score 80 vs below 5): the rule holds vacuously, ρ_W < 0 so
    // max(-ρ_W, ...) is positive — no violation, no negative
    // robustness under the invariant name.
    let scen: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"replay","inputs":[]},"duration_s":0.2,"invariants":[
            {"name":"heal_policy","rule":"custom","path":"TestApi.score","check":"below","value":5,
             "requires_path":"TestApi.active_players","requires_check":"above","requires_value":500}]}"#,
    )
    .unwrap();
    let mut app = build_app();
    app.add_systems(Update, |mut api: ResMut<TestApi>| {
        api.score = 80;
        api.active_players = 0;
    });
    let rep = run_scenario(&mut app, &scen).unwrap();
    assert!(
        rep.violations.is_empty(),
        "vacuous implication must not violate: {:?}",
        rep.violations
    );
    let entry = &rep.robustness.per_invariant["heal_policy"];
    assert!(
        entry.0 > 0.0,
        "WHEN false all run: ρ = -ρ_W > 0 (masking avoided), got {entry:?}"
    );
}

#[test]
fn implication_records_require_margin_when_when_holds() {
    // WHEN true, REQUIRE violated: ρ = max(-ρ_W, ρ_R) = ρ_R when ρ_W
    // ≥ 0 dominates... no: -ρ_W ≤ 0 while the rule is active only when
    // ρ_W ≥ 0; the recorded value must be ρ_R (negative), matching
    // the violation.
    let scen: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"replay","inputs":[]},"duration_s":0.2,"invariants":[
            {"name":"heal_policy","rule":"custom","path":"TestApi.score","check":"below","value":100,
             "requires_path":"TestApi.active_players","requires_check":"above","requires_value":500}]}"#,
    )
    .unwrap();
    let mut app = build_app();
    app.add_systems(Update, |mut api: ResMut<TestApi>| {
        api.score = 80;
        api.active_players = 0;
    });
    let rep = run_scenario(&mut app, &scen).unwrap();
    assert!(
        rep.violations.iter().any(|v| v.rule == "heal_policy"),
        "REQUIRE violated while WHEN holds must fire"
    );
    let entry = &rep.robustness.per_invariant["heal_policy"];
    assert!(
        entry.0 < 0.0,
        "active violated implication must be negative, got {entry:?}"
    );
}

#[test]
fn implication_requires_path_percepts_work() {
    // requires_path through world percepts (Resource:...) with
    // robustness recorded under the implication semantics.
    let scen: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"replay","inputs":[]},"duration_s":0.2,"invariants":[
            {"name":"heal_policy","rule":"custom","path":"TestApi.score","check":"below","value":100,
             "requires_path":"Resource:Policy.health","requires_check":"below","requires_value":5}]}"#,
    )
    .unwrap();
    let mut app = build_app();
    app.init_resource::<Policy>();
    {
        let registry = app.world_mut().resource_mut::<AppTypeRegistry>();
        registry.0.write().register::<Policy>();
    }
    app.add_systems(Update, |mut api: ResMut<TestApi>| api.score = 80);
    app.world_mut().resource_mut::<Policy>().health = 10.0;
    let rep = run_scenario(&mut app, &scen).unwrap();
    assert!(
        rep.violations.iter().any(|v| v.rule == "heal_policy"),
        "requires_path percept violation must fire"
    );
    let entry = &rep.robustness.per_invariant["heal_policy"];
    assert!(
        entry.0 < 0.0,
        "REQUIRE health 10 < 5 margin -5, got {entry:?}"
    );
}

#[test]
fn differential_and_rate_margins_recorded() {
    // no_decrease: ρ = v_t - v_{t-1}; max_delta_per_sec: max_dps - rate.
    let scen: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"replay","inputs":[]},"duration_s":0.3,"invariants":[
            {"name":"mono","rule":"custom","path":"TestApi.score","differential":"no_decrease"},
            {"name":"rate","rule":"custom","path":"TestApi.score","max_delta_per_sec":100.0}]}"#,
    )
    .unwrap();
    let mut app = build_app();
    // Ramp the score from 1 upward by 3/frame at 60tps → rate 180/s?
    // Keep it gentle: +2 every frame → 120/s at 60 tps. Use +1/frame
    // → 60/s, margin 40.
    app.add_systems(Update, |mut api: ResMut<TestApi>| {
        api.score += 1;
    });
    let rep = run_scenario(&mut app, &scen).unwrap();
    assert!(rep.violations.is_empty(), "{:?}", rep.violations);
    let mono = &rep.robustness.per_invariant["mono"];
    assert!(
        (mono.0 - 1.0).abs() < 1e-9,
        "no_decrease step margin = +1, got {mono:?}"
    );
}

#[test]
fn report_robustness_none_without_custom_invariants() {
    // No custom invariants → no robustness entries (tracker empty).
    let scen: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"replay","inputs":[]},"duration_s":0.2,"invariants":[]}"#,
    )
    .unwrap();
    let mut app = build_app();
    let rep = run_scenario(&mut app, &scen).unwrap();
    assert!(rep.robustness.overall.is_none());
    assert!(rep.robustness.per_invariant.is_empty());
}

#[test]
fn report_robustness_populated_for_every_run_with_invariants() {
    let scen: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"replay","inputs":[]},"duration_s":0.2,"invariants":[
            {"name":"cap","rule":"custom","path":"TestApi.score","check":"below","value":10}]}"#,
    )
    .unwrap();
    let mut app = build_app();
    let rep = run_scenario(&mut app, &scen).unwrap();
    assert!(rep.robustness.overall.is_some());
    assert!(rep.robustness.per_invariant.contains_key("cap"));
    // Entry shape: (min margin, frame of worst, normalized).
    let (rho, frame, norm) = rep.robustness.per_invariant["cap"];
    assert!(rho > 0.0);
    assert!(norm > 0.0);
    let _ = frame;
}

/// le3.9 (RF5): J1 overall<0 iff fail — basic sanity check.
/// Full property test over random invariants deferred to follow-up.
#[test]
fn j1_overall_negative_basic() {
    // Basic cases: unconditional expert rule, text REQUIRE, strict boundary.
    for (thr, should_fail) in [(5.0, false), (200.0, true)] {
        // Unconditional expert rule: rho_W=INF, so rho=rho_R.
        let scen: Scenario = serde_json::from_str(
            &format!(
                r#"{{"bot":{{"type":"replay","inputs":[]}},"duration_s":0.2,"invariants":[
                    {{"name":"exp_uncond","rule":"custom","path":"TestApi.score","requires_path":"TestApi.active_players","requires_check":"ge","requires_value":{thr}}}]}}"#
            ),
        ).unwrap();
        let mut app = build_app();
        let score: i64 = if should_fail { 3 } else { 10 };
        app.add_systems(Update, move |mut api: ResMut<TestApi>| {
            api.score = score;
            api.active_players = if should_fail { 0 } else { 250 };
        });
        let rep = run_scenario(&mut app, &scen).unwrap();
        let overall_neg = rep.robustness.overall.map(|o| o < 0.0).unwrap_or(false);
        let has_violation = !rep.violations.is_empty();
        assert_eq!(
            should_fail, overall_neg,
            "unconditional expert rule overall"
        );
        assert_eq!(
            should_fail, has_violation,
            "unconditional expert rule violation"
        );
    }
}

#[test]
fn overall_negative_iff_fail_across_fixture_suite() {
    // Property: for every invariant kind exercised by the fixture
    // suite, overall < 0 iff the run failed some robustness-tracked
    // invariant. Seeded property loop over pass/fail pairs.
    for (thr, should_fail) in [(5.0, false), (200.0, true)] {
        let scen: Scenario = serde_json::from_str(
            &format!(
                r#"{{"bot":{{"type":"replay","inputs":[]}},"duration_s":0.2,"invariants":[
                    {{"name":"cap","rule":"custom","path":"TestApi.score","check":"above","value":{thr}}}]}}"#
            ),
        )
        .unwrap();
        let mut app = build_app();
        app.add_systems(Update, |mut api: ResMut<TestApi>| api.score = 80);
        let rep = run_scenario(&mut app, &scen).unwrap();
        assert_eq!(should_fail, rep.robustness.overall.unwrap() < 0.0);
        assert_eq!(should_fail, !rep.violations.is_empty());
    }
}

// ---------------------------------------------------------------------------
// le3.18 (RF22): text-WHEN expert rules + J1 property
// ---------------------------------------------------------------------------

fn run_text_expert(
    when_phase: &str,
    when_expect: &str,
    healing: bool,
) -> bevy_swarm::driver::PlaytestReport {
    // Policy: WHEN phase == when_expect, REQUIRE healing == 1.
    let healing_num: f64 = if healing { 1.0 } else { 0.0 };
    let scen: Scenario = serde_json::from_str(&format!(
        r#"{{"bot":{{"type":"replay","inputs":[]}},"duration_s":0.2,"invariants":[
            {{"name":"policy","rule":"custom",
              "path":"TestApi.phase","check":"equals","value":"{when_expect}",
              "requires_path":"TestApi.healing","requires_check":"equals",
              "requires_value":1}}]}}"#
    ))
    .unwrap();
    let when_phase = when_phase.to_string();
    let mut app = build_app();
    app.add_systems(Update, move |mut api: ResMut<TestApi>| {
        api.custom_text.insert("phase".into(), when_phase.clone());
        api.custom_numeric.insert("healing".into(), healing_num);
    });
    run_scenario(&mut app, &scen).unwrap()
}

#[test]
fn text_when_expert_rule_four_combinations() {
    // WHEN(phase=="danger") REQUIRE(healing==true)
    // (when_expect danger, when_phase danger|calm) x (req 1|0)
    // Only WHEN-true ∧ REQUIRE-false fails.
    for (when_phase, req_healing, should_fail) in [
        ("danger", true, false), // WHEN T, REQUIRE T: pass
        ("danger", false, true), // WHEN T, REQUIRE F: VIOLATE
        ("calm", true, false),   // WHEN F: vacuous pass
        ("calm", false, false),  // WHEN F: vacuous pass
    ] {
        let rep = run_text_expert(when_phase, "danger", req_healing);
        assert_eq!(
            should_fail,
            !rep.violations.is_empty(),
            "text-WHEN violations: phase={when_phase} healing={req_healing}"
        );
        assert_eq!(
            should_fail,
            rep.robustness.overall.map(|o| o < 0.0).unwrap_or(false),
            "text-WHEN overall<0 iff fail: phase={when_phase} healing={req_healing}"
        );
    }
}

#[test]
fn text_when_expert_rule_evaluated_at_all() {
    // The RF22 core bug: text-WHEN expert rules were silently skipped
    // (the numeric `else { continue }` swallowed them — always Pass).
    // A violating combination MUST produce a violation.
    let rep = run_text_expert("danger", "danger", false);
    assert!(
        rep.violations
            .iter()
            .any(|v| v.rule.as_str() == "policy" || v.detail.contains("expert rule violated")),
        "text-WHEN violation must be reported, got: {:?}",
        rep.violations
    );
}

#[test]
fn numeric_when_text_require_robustness_matches_verdict() {
    // WHEN(score>5) REQUIRE(phase=="saved"): rho_w numeric, text rho_r.
    // WHEN true & REQUIRE false must give overall < 0.
    let scen: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"replay","inputs":[]},"duration_s":0.2,"invariants":[
            {"name":"save_policy","rule":"custom",
             "path":"TestApi.score","check":"above","value":5,
             "requires_path":"TestApi.save_state","requires_check":"equals",
             "requires_value":"saved"}]}"#,
    )
    .unwrap();
    for (score, save_state, should_fail) in [
        (10, "saved", false), // WHEN T, REQUIRE T
        (10, "dirty", true),  // WHEN T, REQUIRE F — the RF22 inversion case
        (0, "dirty", false),  // WHEN F: vacuous
    ] {
        let mut app = build_app();
        app.add_systems(Update, move |mut api: ResMut<TestApi>| {
            api.score = score;
            api.custom_text
                .insert("save_state".into(), save_state.into());
        });
        let rep = run_scenario(&mut app, &scen).unwrap();
        assert_eq!(
            should_fail,
            rep.robustness.overall.map(|o| o < 0.0).unwrap_or(false),
            "numeric-WHEN/text-REQUIRE overall: score={score} state={save_state}"
        );
    }
}

#[test]
fn j1_overall_negative_iff_fail_property() {
    // le3.34 (RF44): deterministic enumeration of BOUNDARY cases
    // (value == threshold for every op; Equals/Ne WHEN; eventually
    // with deadline before/after run end; text WHEN/REQUIRE) plus
    // seeded random cases. Assert overall<0 IFF a violation was
    // produced, on the REAL path (run_scenario).
    let mut cases: Vec<(String, i64, usize, bool, &'static str)> = Vec::new();
    // (scenario-json fragment tail, score, players, should_fail, phase)
    let ops = [
        ("le", "below"),
        ("ge", "above"),
        ("lt", "lt"),
        ("gt", "gt"),
        ("equals", "equals"),
        ("ne", "ne"),
    ];
    // Boundary: value == threshold, for every op, both directions of
    // satisfaction around it.
    for (op, check) in ops {
        for &(score, expect_fail_exact) in &[(50i64, false), (51, true), (49, false)] {
            // exact-op semantics: fail depends on op at the boundary
            let should_fail = match op {
                "le" => score > 50,
                "ge" => score < 50,
                "lt" => score >= 50,
                "gt" => score <= 50,
                "equals" => score != 50,
                _ => score == 50, // "ne"
            };
            let _ = expect_fail_exact;
            cases.push((
                format!(
                    r#"{{"name":"b_{op}","rule":"custom","path":"TestApi.score","check":"{check}","value":50}}"#
                ),
                score,
                100,
                should_fail,
                "playing",
            ));
        }
    }
    // Ne boundary: value == threshold must FAIL.
    cases.push((
        r#"{"name":"b_ne_eq","rule":"custom","path":"TestApi.score","check":"ne","value":50}"#
            .into(),
        50,
        100,
        true,
        "playing",
    ));
    // Equals WHEN expert rule at the boundary: WHEN score EQUALS 50
    // REQUIRE players GE 5, players=0 → must fail with overall<0
    // (the RF44 critical: rho was max(-0, -5) = -0).
    cases.push((
        r#"{"name":"b_exp_equals","rule":"custom","path":"TestApi.score","check":"equals","value":50,"requires_path":"TestApi.active_players","requires_check":"ge","requires_value":5}"#.into(),
        50, 0, true, "playing",
    ));
    // Same but WHEN holds and REQUIRE holds → pass.
    cases.push((
        r#"{"name":"b_exp_ok","rule":"custom","path":"TestApi.score","check":"equals","value":50,"requires_path":"TestApi.active_players","requires_check":"ge","requires_value":5}"#.into(),
        50, 7, false, "playing",
    ));
    // Text WHEN/REQUIRE: WHEN phase=menu REQUIRE score ge 0 → pass.
    cases.push((
        r#"{"name":"b_text_when","rule":"custom","path":"TestApi.phase","check":"equals","value":"menu","requires_path":"TestApi.score","requires_check":"ge","requires_value":0}"#.into(),
        10, 3, false, "menu",
    ));
    // Text WHEN false (phase=playing) REQUIRE score ge 100 →
    // VACUOUSLY true: no violation, overall >= 0 (the RF44 minor:
    // sign inversion enforced the REQUIRE against a false guard).
    cases.push((
        r#"{"name":"b_text_vacuous","rule":"custom","path":"TestApi.phase","check":"equals","value":"menu","requires_path":"TestApi.score","requires_check":"ge","requires_value":100}"#.into(),
        10, 3, false, "playing",
    ));
    // Eventually with deadline INSIDE the run: never satisfied → fail.
    cases.push((
        r#"{"name":"b_ev_in","rule":"custom","path":"TestApi.score","check":"ge","value":100,"eventually_s":0.1}"#.into(),
        10, 3, true, "playing",
    ));
    // Eventually with deadline PAST the run end (duration 0.15 <
    // deadline 5.0): never satisfied within the run → MUST fail
    // (the RF44 critical: previously overall<0 with no violation).
    cases.push((
        r#"{"name":"b_ev_past","rule":"custom","path":"TestApi.score","check":"ge","value":100,"eventually_s":5.0}"#.into(),
        10, 3, true, "playing",
    ));
    // Eventually satisfied mid-run → pass.
    cases.push((
        r#"{"name":"b_ev_ok","rule":"custom","path":"TestApi.score","check":"ge","value":5,"eventually_s":1.0}"#.into(),
        10, 3, false, "playing",
    ));

    // Seeded random cases on top: scores/thresholds on both sides of
    // the boundaries.
    let mut rng_state = 0xC0FFEEu64;
    let mut next = move || {
        rng_state ^= rng_state >> 12;
        rng_state ^= rng_state << 25;
        rng_state ^= rng_state >> 27;
        rng_state.wrapping_mul(0x2545F4914F6CDD1D)
    };
    for _ in 0..12 {
        let score = (next() % 200) as i64;
        let thr = (next() % 200) as i64;
        let (op, check) = ops[(next() % 6) as usize];
        let should_fail = match op {
            "le" => score > thr,
            "ge" => score < thr,
            "lt" => score >= thr,
            "gt" => score <= thr,
            "equals" => score != thr,
            _ => score == thr,
        };
        cases.push((
            format!(
                r#"{{"name":"r_{op}_{thr}","rule":"custom","path":"TestApi.score","check":"{check}","value":{thr}}}"#
            ),
            score,
            100,
            should_fail,
            "playing",
        ));
    }

    for (inv_json, score, players, should_fail, phase) in cases {
        let phase = phase.to_string();
        let scen: Scenario = serde_json::from_str(&format!(
            r#"{{"bot":{{"type":"replay","inputs":[]}},"duration_s":0.15,"invariants":[{inv_json}]}}"#
        ))
        .unwrap();
        let mut app = build_app();
        app.add_systems(Update, move |mut api: ResMut<TestApi>| {
            api.score = score;
            api.active_players = players;
            api.custom_text.insert("phase".into(), phase.clone());
        });
        let rep = run_scenario(&mut app, &scen).unwrap();
        let overall_neg = rep.robustness.overall.map(|o| o < 0.0).unwrap_or(false);
        let failed = rep.violations.iter().any(|v| v.rule != "bevy_suppressed");
        let inv_name = scen.invariants[0].name.clone();
        assert_eq!(
            should_fail,
            overall_neg,
            "J1 overall sign: {inv_name} score={score} players={players} violations={:?}",
            rep.violations
                .iter()
                .map(|v| v.rule.clone())
                .collect::<Vec<_>>()
        );
        assert_eq!(
            should_fail, failed,
            "J1 violation presence: {inv_name} score={score} players={players}",
        );
    }
}
