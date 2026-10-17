//! swarm-dt0: Time<Real> priming — frame 1 must advance a full
//! manual virtual delta, uniform with frames 2..N.

use bevy::prelude::*;
use bevy_playtest::contract::{IntentSurface, SurfaceVariant, TestConventionsPlugin};
use bevy_playtest::driver::PlaytestPlugin;
use bevy_playtest::prelude::PlaytestStatus;
use bevy_playtest::scenario::Scenario;

#[derive(Resource, Default)]
struct Deltas(Vec<f32>);

fn build_game() -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        bevy::transform::TransformPlugin,
        TestConventionsPlugin,
        PlaytestPlugin,
    ));
    app.insert_resource(IntentSurface::new(vec![
        SurfaceVariant::Wait,
        SurfaceVariant::Choice(1),
    ]));
    app.insert_resource(Deltas::default());
    app.add_systems(Update, |time: Res<Time<Virtual>>, mut d: ResMut<Deltas>| {
        d.0.push(time.delta().as_secs_f32());
    });
    app
}

fn scenario_tps(tps: u32) -> Scenario {
    serde_json::from_str(&format!(
        r#"{{"bot":{{"type":"chaos","seed":1}},"duration_s":0.3,"tps":{tps},"simulated_time":true,"invariants":[]}}"#
    ))
    .unwrap()
}

#[test]
fn frame1_delta_equals_configured_virtual_dt() {
    let mut app = build_game();
    let report = bevy_playtest::driver::run_scenario(&mut app, &scenario_tps(20)).unwrap();
    assert_eq!(report.status, PlaytestStatus::Pass);
    let deltas = app.world().resource::<Deltas>().0.clone();
    assert!(deltas.len() >= 3, "need a few frames, got {}", deltas.len());
    let expected = 1.0f32 / 20.0;
    // THE regression: an unprimed Time<Real> gives frame 1 a
    // zero/near-zero delta.
    assert!(
        (deltas[0] - expected).abs() < 1e-3,
        "frame 1 delta {} != virtual dt {expected}",
        deltas[0]
    );
    // Uniformity: every frame advances the full virtual dt.
    for (i, d) in deltas.iter().enumerate() {
        assert!(
            (d - expected).abs() < 1e-3,
            "frame {} delta {} != virtual dt {expected}",
            i + 1,
            d
        );
    }
}

#[test]
fn scenario_validation_rejects_tps_below_max_delta_clamp() {
    // 1/tps > 250ms (tps < 4): Time<Virtual> would silently clamp
    // the manual delta — must be rejected at load time, not silently
    // distorted.
    let err = bevy_playtest::scenario::validate_scenario(&scenario_tps(2)).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("max_delta"),
        "expected max_delta rejection, got: {msg}"
    );
}
