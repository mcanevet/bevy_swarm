use bevy::prelude::*;

#[allow(unused_imports)]
use super::fixtures::*;

// ---------------------------------------------------------------------------
// Ddmin minimizer tests
// ---------------------------------------------------------------------------

#[test]
fn ddmin_minimizes_to_single_action() {
    // Crash depends only on action 3 (value 42).
    let actions: Vec<i32> = vec![1, 2, 42, 4, 5, 6];
    let result = crate::harness::ddmin_minimize(&actions, |c| c.contains(&42));
    assert_eq!(result, vec![42]);
}

#[test]
fn ddmin_minimizes_multi_dependency() {
    // Crash requires BOTH 7 and 42.
    let actions: Vec<i32> = vec![1, 7, 3, 42, 5, 6, 8];
    let result = crate::harness::ddmin_minimize(&actions, |c| c.contains(&7) && c.contains(&42));
    assert_eq!(result, vec![7, 42]);
}

#[test]
fn ddmin_returns_as_is_when_not_reproducible() {
    let actions: Vec<i32> = vec![1, 2, 3];
    let result = crate::harness::ddmin_minimize(&actions, |_| false);
    assert_eq!(result, vec![1, 2, 3]);
}

#[test]
fn ddmin_preserves_order() {
    // Crash requires 42 AFTER 7 (order-dependent).
    let actions: Vec<i32> = vec![7, 1, 2, 42, 3];
    let result = crate::harness::ddmin_minimize(&actions, |c| {
        let pos7 = c.iter().position(|&x| x == 7);
        let pos42 = c.iter().position(|&x| x == 42);
        matches!((pos7, pos42), (Some(a), Some(b)) if a < b)
    });
    assert_eq!(result, vec![7, 42]);
}

#[test]
fn ddmin_empty_sequence() {
    let result: Vec<i32> = crate::harness::ddmin_minimize(&[], |_| true);
    assert!(result.is_empty());
}

#[test]
fn ddmin_real_action_log() {
    use crate::contract::{ActionLog, ActionSource};
    let mut log = ActionLog::default();
    for i in 0..10 {
        log.record(
            i * 10,
            0.0,
            ActionSource::BotScenario("test".into()),
            format!("choice:idx={}", i % 2),
            None,
        );
    }
    let timed = crate::harness::action_log_to_timed_actions(&log.entries);
    assert_eq!(timed.len(), 10);
    assert_eq!(timed[0].frame, 0);
    assert_eq!(timed[0].action, "choice:idx=0");
    // Conversion back to replay intents
    let replayable: Vec<_> = timed
        .iter()
        .filter_map(crate::harness::action_to_replay_intent)
        .collect();
    assert_eq!(replayable.len(), 10);
}
