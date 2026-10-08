//! J1: quantitative robustness semantics — signed margins in reports.

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
