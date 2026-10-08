//! I4: change-tick liveness — card games stay alive, turn-based
//! softlocks are caught, turn-based idling is legitimate.

use bevy::prelude::*;
use bevy_swarm::contract::{Gameplay, TestApi, TestConventionsPlugin, TurnBased, UserIntent};
use bevy_swarm::driver::{run_scenario, PlaytestPlugin};
use bevy_swarm::scenario::Scenario;

fn build_app() -> App {
    let mut app = App::new();
    app.add_plugins((
        bevy::MinimalPlugins,
        bevy::transform::TransformPlugin,
        PlaytestPlugin,
        TestConventionsPlugin,
    ));
    app
}

#[derive(Component)]
struct Hand(u32);

#[test]
fn card_game_not_frozen() {
    // A game mutating a Hand component (no Transform) must stay alive:
    // liveness is any Gameplay component change, not just Transforms.
    let scenario: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"replay","inputs":[]},"duration_s":2.2,"invariants":[]}"#,
    )
    .unwrap();
    let mut app = build_app();
    app.add_systems(bevy::app::Startup, |mut commands: Commands| {
        commands.spawn((Name::new("Dealer"), Gameplay, Hand(0)));
    });
    app.add_systems(Update, |mut q: Query<&mut Hand>| {
        // Deal a card every frame: state churn, no Transform involved.
        for mut h in &mut q {
            h.0 += 1;
        }
    });
    let rep = run_scenario(&mut app, &scenario).unwrap();
    assert_eq!(
        rep.status,
        bevy_swarm::enums::PlaytestStatus::Pass,
        "a card game mutating Hand must not be flagged frozen: {:?}",
        rep.violations
    );
}

#[test]
fn turn_based_softlock_detected() {
    // The game consumes Choice intents, but after turn 3 its
    // turn-advance system is buggy (never advances again). The intent
    // was consumed, no liveness follows → stuck_after_intent.
    let scenario: Scenario = serde_json::from_str(
        r#"{
            "bot": {"type": "replay", "inputs": [
                {"frame": 30, "intent": {"intent": "choice", "index": 1}},
                {"frame": 60, "intent": {"intent": "choice", "index": 1}}
            ]},
            "duration_s": 4.0,
            "invariants": [],
            "liveness": {"mode": "after_intent", "timeout_s": 1.0}
        }"#,
    )
    .unwrap();
    let mut app = build_app();
    app.insert_resource(TestApi::default());
    app.add_message::<UserIntent>();
    app.add_systems(bevy::app::Startup, |mut commands: Commands| {
        commands.spawn((Name::new("Board"), Gameplay, TurnOnMarks(0)));
        commands.spawn(TurnBased);
    });
    app.add_systems(
        Update,
        (
            |mut reader: bevy::ecs::message::MessageReader<UserIntent>| {
                // Intents ARE consumed (drained) ...
                for _ in reader.read() {}
            },
            // ... and the turn advances only up to turn 3 (bug).
            |mut marks: Query<&mut TurnOnMarks>| {
                for mut m in &mut marks {
                    if m.0 < 3 {
                        m.0 += 1;
                    }
                }
            },
        ),
    );
    let rep = run_scenario(&mut app, &scenario).unwrap();
    assert!(
        rep.violations.iter().any(|v| v.rule == "stuck_after_intent"),
        "a turn-based game whose state stops changing after a consumed intent must trip stuck_after_intent, got: {:?}",
        rep.violations
    );
    assert!(!rep.violations.iter().any(|v| v.rule == "frozen_world"));
}

#[test]
fn turn_based_idle_ok() {
    // No intents at all for the whole run: idle waiting is legitimate
    // for a turn-based game — no liveness violation of any kind.
    let scenario: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"replay","inputs":[]},"duration_s":3.0,"invariants":[]}"#,
    )
    .unwrap();
    let mut app = build_app();
    app.add_systems(bevy::app::Startup, |mut commands: Commands| {
        commands.spawn((Name::new("Board"), Gameplay, Hand(0)));
        commands.spawn(TurnBased);
    });
    let rep = run_scenario(&mut app, &scenario).unwrap();
    assert_eq!(
        rep.status,
        bevy_swarm::enums::PlaytestStatus::Pass,
        "idle turn-based worlds are legitimately paused: {:?}",
        rep.violations
    );
}

#[test]
fn liveness_off_disables_oracle() {
    // mode "off" explicitly disables liveness; a static real-time world
    // must not trip frozen_world.
    let scenario: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"replay","inputs":[]},"duration_s":2.2,"invariants":[],
            "liveness": {"mode": "off"}}"#,
    )
    .unwrap();
    let mut app = build_app();
    app.add_systems(bevy::app::Startup, |mut commands: Commands| {
        commands.spawn((Name::new("Ball"), Gameplay, Transform::default()));
    });
    let rep = run_scenario(&mut app, &scenario).unwrap();
    assert!(
        !rep.violations.iter().any(|v| v.rule == "frozen_world"),
        "liveness off must disable the oracle: {:?}",
        rep.violations
    );
}

#[derive(Component)]
struct TurnOnMarks(u32);
