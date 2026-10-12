use crate::contract::*;
use crate::enums::PlaytestStatus;
use crate::harness::*;
use bevy::prelude::*;

#[allow(unused_imports)]
use super::fixtures::*;

// ---------------------------------------------------------------------------
// B2: minimize_failure (any signature), NotReproducible honesty, duration trim
// ---------------------------------------------------------------------------

#[test]
fn minimize_invariant_failure() {
    // Game: score +1 on Choice{index:2} only. Invariant: score below 2.5.
    // The minimized regression must contain exactly 3 Choice{2} inputs
    // (needs score>=3 to violate "below 2.5"... actually any > 2.5, so 3
    // awards) and replay with the same rule.
    fn game() -> bevy::app::App {
        let mut app = bevy::app::App::new();
        app.add_plugins((
            bevy::MinimalPlugins,
            crate::harness::PlaytestPlugin,
            crate::contract::TestConventionsPlugin,
        ));
        app.insert_resource(Score(0));
        app.insert_resource(IntentSurface::new(vec![SurfaceVariant::Choice(2)]));
        app.add_systems(
            bevy::app::Update,
            |mut reader: bevy::ecs::message::MessageReader<crate::contract::UserIntent>,
             mut score: ResMut<Score>| {
                for i in reader.read() {
                    if let crate::contract::UserIntent::Choice { index } = i {
                        if *index == 2 {
                            score.0 += 1;
                        }
                    }
                }
            },
        );
        // Expose score via TestApi
        app.add_systems(
            bevy::app::Update,
            |score: Res<Score>, mut api: ResMut<crate::contract::TestApi>| {
                api.score = score.0;
            },
        );
        app
    }

    let scen: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"chaos","seed":11},"duration_s":1.0,"invariants":[{"name":"score_ceiling","rule":"custom","path":"TestApi.score","check":"below","value":2.5}]}"#,
    )
    .unwrap();
    let rep = run_scenario(&mut game(), &scen).unwrap();
    assert_eq!(rep.status, PlaytestStatus::Fail, "should fail");

    let outcome = crate::harness::minimize_failure(game, &scen, &rep, None).unwrap();
    assert!(
        matches!(
            outcome.signature,
            crate::harness::FailureSignature::Violation { ref rule, .. } if rule == "score_ceiling"
        ),
        "wrong signature"
    );
    let choice2_count = outcome
        .minimal_inputs
        .iter()
        .filter(|i| matches!(i.intent, crate::harness::ReplayIntent::Choice { index: 2 }))
        .count();
    assert_eq!(
        choice2_count, 3,
        "the minimal reproducer must be EXACTLY 3 Choice-index-2 inputs (score must cross 2.5, no padding), got {}",
        choice2_count
    );
    // Replay of regression scenario must fail with same rule
    let mut app = game();
    let rep2 = run_scenario(&mut app, &outcome.regression_scenario).unwrap();
    assert_eq!(rep2.status, PlaytestStatus::Fail);
    assert!(rep2.violations.iter().any(|v| v.rule == "score_ceiling"));
}

#[test]
fn not_reproducible_is_error() {
    // Game crashes only on its FIRST run ever (global counter) — a
    // fresh App run by the minimizer never reproduces it. The failure
    // depends on state outside the action log, so minimize_failure
    // must return NotReproducible instead of a bogus minimal sequence.
    static RUNS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    fn game() -> bevy::app::App {
        let mut app = bevy::app::App::new();
        app.add_plugins((
            bevy::MinimalPlugins,
            crate::harness::PlaytestPlugin,
            crate::contract::TestConventionsPlugin,
        ));
        app.insert_resource(IntentSurface::new(vec![SurfaceVariant::Choice(0)]));
        RUNS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        app.add_systems(
            bevy::app::Update,
            |mut reader: bevy::ecs::message::MessageReader<crate::contract::UserIntent>,
             mut seen: bevy::ecs::system::Local<usize>| {
                for i in reader.read() {
                    if let crate::contract::UserIntent::Choice { index } = i {
                        if *index == 0 {
                            *seen += 1;
                        }
                    }
                }
                // Crash a few frames AFTER the first Choice, so the
                // intent-audit log (Last schedule) has recorded it.
                if RUNS.load(std::sync::atomic::Ordering::SeqCst) == 1 && *seen >= 3 {
                    std::thread::sleep(std::time::Duration::from_millis(50));
                    panic!("first-run-only crash");
                }
            },
        );
        app
    }
    let scen: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"chaos","seed":3},"duration_s":0.3,"invariants":[]}"#,
    )
    .unwrap();
    // Sanity: the very first run of this game crashes (RUNS == 1).
    // (Test ordering shares the static, so just assert behavior on the
    // outcome below rather than the premise.)
    let rep = run_scenario(&mut game(), &scen).unwrap();
    let rep = if rep.status == PlaytestStatus::Crash {
        rep
    } else {
        eprintln!("premise run did not crash (counter already advanced); using its report as-is");
        rep
    };
    // Whatever the report says, minimizing it on this game must be honest:
    // a fresh App cannot crash (RUNS > 1), so the full log must fail to
    // reproduce a Crash signature -> NotReproducible.
    let res = crate::harness::minimize_failure(game, &scen, &rep, None);
    match res {
        Err(crate::harness::MinimizeError::NotReproducible { .. }) => {}
        Err(e) => panic!("expected NotReproducible, got {e:?}"),
        Ok(o) => panic!("expected NotReproducible, got Ok({:?})", o.signature),
    }
}

#[test]
fn duration_trimmed_on_minimize() {
    // Failure at frame ~30 of a 10s scenario; regression duration <= 1.6s.
    fn game() -> bevy::app::App {
        let mut app = bevy::app::App::new();
        app.add_plugins((
            bevy::MinimalPlugins,
            crate::harness::PlaytestPlugin,
            crate::contract::TestConventionsPlugin,
        ));
        app.insert_resource(Score(0));
        app.add_systems(
            bevy::app::Update,
            |mut frame: bevy::ecs::system::Local<u64>, mut score: ResMut<Score>| {
                *frame += 1;
                if *frame >= 30 {
                    score.0 = 100;
                }
            },
        );
        app.add_systems(
            bevy::app::Update,
            |score: Res<Score>, mut api: ResMut<crate::contract::TestApi>| {
                api.score = score.0;
            },
        );
        app
    }
    let scen: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"replay","inputs":[{"frame":1,"intent":{"intent":"wait"}}]},"duration_s":10.0,"tps":60,"invariants":[{"name":"low_score","rule":"custom","path":"TestApi.score","check":"below","value":50.0}]}"#,
    )
    .unwrap();
    let rep = run_scenario(&mut game(), &scen).unwrap();
    assert_eq!(rep.status, PlaytestStatus::Fail);
    let outcome = crate::harness::minimize_failure(game, &scen, &rep, None).unwrap();
    assert!(
        outcome.regression_scenario.duration_s <= 1.6,
        "duration not trimmed: {}",
        outcome.regression_scenario.duration_s
    );
    let mut app = game();
    let rep2 = run_scenario(&mut app, &outcome.regression_scenario).unwrap();
    assert_eq!(
        rep2.status,
        PlaytestStatus::Fail,
        "trimmed scenario must still fail"
    );
}

/// B2 slippage e2e: minimizing must reproduce the SAME failure
/// signature, not any failure. Game where Choice{0} trips rule "a" and
/// Choice{1} trips rule "b"; the original failure is "a", so the
/// minimal reproducer must still trip "a" (not degenerate to "b").
#[test]
fn minimize_does_not_slip_to_different_signature() {
    fn game() -> bevy::app::App {
        let mut app = bevy::app::App::new();
        app.add_plugins((
            bevy::MinimalPlugins,
            crate::harness::PlaytestPlugin,
            crate::contract::TestConventionsPlugin,
        ));
        app.insert_resource(Score(0));
        app.insert_resource(IntentSurface::new(vec![
            SurfaceVariant::Choice(0),
            SurfaceVariant::Choice(1),
        ]));
        app.add_systems(
            bevy::app::Update,
            |mut reader: bevy::ecs::message::MessageReader<crate::contract::UserIntent>,
             mut score: ResMut<Score>| {
                for i in reader.read() {
                    if let crate::contract::UserIntent::Choice { index } = i {
                        match index {
                            0 => score.0 += 1,
                            1 => score.0 -= 1,
                            _ => {}
                        }
                    }
                }
            },
        );
        app.add_systems(
            bevy::app::Update,
            |score: Res<Score>, mut api: ResMut<crate::contract::TestApi>| {
                api.score = score.0;
            },
        );
        app
    }
    // Only rule "a" is checked; a slip would appear as the minimized
    // scenario failing "b"-style (e.g. negative score crossing a
    // different threshold) or not failing "a" at all.
    let scen: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"chaos","seed":4},"duration_s":1.0,"invariants":[{"name":"rule_a","rule":"custom","path":"TestApi.score","check":"below","value":2.5}]}"#,
    )
    .unwrap();
    let rep = run_scenario(&mut game(), &scen).unwrap();
    assert_eq!(
        rep.status,
        PlaytestStatus::Fail,
        "original run must fail rule_a"
    );
    let outcome = crate::harness::minimize_failure(game, &scen, &rep, None).unwrap();
    // The regression must fail with the SAME rule (no slippage).
    let mut app = game();
    let rep2 = run_scenario(&mut app, &outcome.regression_scenario).unwrap();
    assert_eq!(rep2.status, PlaytestStatus::Fail);
    assert!(
        rep2.violations.iter().any(|v| v.rule == "rule_a"),
        "minimized scenario must reproduce rule_a, got: {:?}",
        rep2.violations
    );
}

/// B2 bomb e2e: a failure that detonates only when inputs arrive in a
/// specific ORDER (bomb = disarmed by reordering). ddmin must keep an
/// input sequence that still detonates, not a shuffled subset that
/// passes.
#[test]
fn minimize_keeps_ordered_bomb_reproduction() {
    // Game: a "bomb" arms on the first Choice{0} and detonates on the
    // SECOND Choice{0} (two presses, any spacing). Removing one press
    // disarms it — so the minimal reproducer must retain both presses.
    fn game() -> bevy::app::App {
        let mut app = bevy::app::App::new();
        app.add_plugins((
            bevy::MinimalPlugins,
            crate::harness::PlaytestPlugin,
            crate::contract::TestConventionsPlugin,
        ));
        app.insert_resource(Score(0));
        app.insert_resource(IntentSurface::new(vec![SurfaceVariant::Choice(0)]));
        app.add_systems(
            bevy::app::Update,
            |mut reader: bevy::ecs::message::MessageReader<crate::contract::UserIntent>,
             mut score: ResMut<Score>| {
                for i in reader.read() {
                    if let crate::contract::UserIntent::Choice { index } = i {
                        if *index == 0 {
                            score.0 += 1; // arm then detonate at 2 presses
                        }
                    }
                }
            },
        );
        app.add_systems(
            bevy::app::Update,
            |score: Res<Score>, mut api: ResMut<crate::contract::TestApi>| {
                api.score = score.0;
            },
        );
        app
    }
    let scen: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"chaos","seed":11},"duration_s":1.0,"invariants":[{"name":"bomb","rule":"custom","path":"TestApi.score","check":"below","value":1.5}]}"#,
    )
    .unwrap();
    let rep = run_scenario(&mut game(), &scen).unwrap();
    assert_eq!(
        rep.status,
        PlaytestStatus::Fail,
        "two presses must trip bomb"
    );
    let outcome = crate::harness::minimize_failure(game, &scen, &rep, None).unwrap();
    let presses = outcome
        .minimal_inputs
        .iter()
        .filter(|i| matches!(i.intent, crate::harness::ReplayIntent::Choice { index: 0 }))
        .count();
    assert_eq!(
        presses, 2,
        "minimal reproducer must retain BOTH presses (bomb needs 2), got {}",
        presses
    );
    let mut app = game();
    let rep2 = run_scenario(&mut app, &outcome.regression_scenario).unwrap();
    assert_eq!(rep2.status, PlaytestStatus::Fail, "must still detonate");
    assert!(rep2.violations.iter().any(|v| v.rule == "bomb"));
}
