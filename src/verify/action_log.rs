use crate::harness::*;

#[allow(unused_imports)]
use super::fixtures::*;

#[test]
fn action_log_structured_payloads_roundtrip() {
    // Verify that the structured action-log format (intent:<variant> + JSON)
    // roundtrips through action_to_replay_intent for all variants.
    use crate::harness::{action_to_replay_intent, TimedAction};

    let move_action = TimedAction {
        frame: 10,
        source: "test".into(),
        action: "intent:move".into(),
        details: Some(r#"{"dir":[1.5,-2.3]}"#.into()),
    };
    assert_eq!(
        action_to_replay_intent(&move_action),
        Some(ReplayIntent::Move { dir: (1.5, -2.3) })
    );

    let choice_action = TimedAction {
        frame: 20,
        source: "test".into(),
        action: "intent:choice".into(),
        details: Some(r#"{"index":42}"#.into()),
    };
    assert_eq!(
        action_to_replay_intent(&choice_action),
        Some(ReplayIntent::Choice { index: 42 })
    );

    let axis_action = TimedAction {
        frame: 30,
        source: "test".into(),
        action: "intent:axis".into(),
        details: Some(r#"{"name":"throttle","value":0.75}"#.into()),
    };
    assert_eq!(
        action_to_replay_intent(&axis_action),
        Some(ReplayIntent::Axis {
            name: "throttle".into(),
            value: 0.75
        })
    );

    let select_action = TimedAction {
        frame: 40,
        source: "test".into(),
        action: "intent:select".into(),
        details: Some(r#"{"target":"end_turn"}"#.into()),
    };
    assert_eq!(
        action_to_replay_intent(&select_action),
        Some(ReplayIntent::Select {
            target: crate::scenario::SelectTarget::Name("end_turn".into())
        })
    );

    let wait_action = TimedAction {
        frame: 50,
        source: "test".into(),
        action: "intent:wait".into(),
        details: None,
    };
    assert_eq!(
        action_to_replay_intent(&wait_action),
        Some(ReplayIntent::Wait)
    );
}
