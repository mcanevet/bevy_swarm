use crate::enums::PlaytestStatus;
use crate::harness::*;
use bevy::prelude::*;

#[allow(unused_imports)]
use super::fixtures::*;

// ---------------------------------------------------------------------------
// A1: deterministic simulated time + configurable tick rate
// ---------------------------------------------------------------------------

#[test]
fn scenario_defaults_to_60_tps_simulated() {
    let scen: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"chaos","seed":1},"duration_s":0.1,"invariants":[],"setup":{}}"#,
    )
    .unwrap();
    assert_eq!(scen.tps, 60);
    assert!(scen.simulated_time);
}

#[test]
fn scenario_tps_is_configurable() {
    let scen: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"chaos","seed":1},"duration_s":0.1,"tps":10,"simulated_time":false,"invariants":[],"setup":{}}"#,
    )
    .unwrap();
    assert_eq!(scen.tps, 10);
    assert!(!scen.simulated_time);
}

#[test]
fn scenario_rejects_zero_tps() {
    let res: Result<Scenario, _> = serde_json::from_str(
        r#"{"bot":{"type":"chaos","seed":1},"duration_s":0.1,"tps":0,"invariants":[],"setup":{}}"#,
    );
    let scen = res.unwrap();
    assert!(crate::harness::validate_scenario(&scen).is_err());
}

#[test]
fn scenario_rejects_nonpositive_duration() {
    let scen: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"chaos","seed":1},"duration_s":0.0,"invariants":[],"setup":{}}"#,
    )
    .unwrap();
    assert!(crate::harness::validate_scenario(&scen).is_err());
}

#[test]
fn simulated_time_advances_deterministically() {
    // Two runs with the same seed must produce identical elapsed simulated
    // time (Time advances by exactly 1/tps per update).
    let run = || {
        let scen: Scenario = serde_json::from_str(
            r#"{"bot":{"type":"chaos","seed":7},"duration_s":0.1,"tps":50,"invariants":[],"setup":{}}"#,
        )
        .unwrap();
        let mut app = build_app();
        run_scenario(&mut app, &scen).unwrap()
    };
    let r1 = run();
    let r2 = run();
    assert_eq!(r1.status, PlaytestStatus::Pass);
    // Reproducibility: same seed, same scenario → same intents emitted.
    assert_eq!(
        r1.coverage.intents_emitted, r2.coverage.intents_emitted,
        "same-seed runs must be deterministic under simulated time"
    );
}

#[test]
fn simulated_time_delta_is_one_over_tps() {
    use bevy::time::Time;
    // Bevy's first time_system run yields zero delta; deltas appear from
    // the second update on. Run 2 ticks (duration_s=0.1 * tps=20).
    let scen: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"chaos","seed":3},"duration_s":0.1,"tps":20,"invariants":[],"setup":{}}"#,
    )
    .unwrap();
    let mut app = build_app();
    let _rep = run_scenario(&mut app, &scen).unwrap();
    let time = app.world().resource::<Time>();
    assert!(
        (time.delta_secs_f64() - 0.05).abs() < 1e-6,
        "delta was {}s, expected exactly 0.05s under ManualDuration",
        time.delta_secs_f64()
    );
}

#[test]
fn frame_time_anomaly_is_opt_in() {
    // No FrameTimeAnomaly invariant armed -> no violations even under heavy
    // wall-clock jitter (a turn-based-ish world doing nothing).
    let scen: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"replay","inputs":[]},"duration_s":0.2,"invariants":[]}"#,
    )
    .unwrap();
    let mut app = build_app();
    let rep = run_scenario(&mut app, &scen).unwrap();
    assert_eq!(rep.status, PlaytestStatus::Pass, "{:?}", rep.violations);
}

#[test]
fn frame_time_anomaly_fires_when_armed() {
    // Arm FrameTimeAnomaly with an absurdly low sensitivity threshold.
    // Force an anomaly by making a system slow, then assert a violation.
    use bevy::app::Update;
    // Needs >60 observed samples (FRAME_TIME_ANOMALY_MIN_SAMPLES) before
    // anomalies can fire, so run 1.5s @ 60tps = 90 ticks and stall late.
    let scen: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"replay","inputs":[]},"duration_s":1.5,"invariants":[{"name":"fast_frames","rule":"frame_time_anomaly"}]}"#,
    )
    .unwrap();
    let mut app = build_app();
    // Stall ~40ms at frames 70-74: far above the sub-ms running mean.
    app.add_systems(Update, |mut frame: Local<i32>| {
        *frame += 1;
        if (70..75).contains(&*frame) {
            std::thread::sleep(std::time::Duration::from_millis(40));
        }
    });
    let rep = run_scenario(&mut app, &scen).unwrap();
    assert!(
        rep.violations
            .iter()
            .any(|v| v.rule == "frame_time_anomaly" && v.target == "fast_frames"),
        "expected frame_time_anomaly violation reported under its dedicated rule, got: {:?}",
        rep.violations
    );
}
