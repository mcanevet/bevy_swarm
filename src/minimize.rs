//! Crash minimization: ddmin over recorded action logs.

use bevy::app::App;

use crate::driver::run_scenario;
use crate::scenario::*;

// ---------------------------------------------------------------------------
// Ddmin — Delta Debugging minimizer (Zeller's ddmin over action sequences).
// Record the full action stream, binary-search removals,
// keep a minimal reproducer. The result is a permanent regression scenario.
// ---------------------------------------------------------------------------

/// Minimize an action sequence that reproduces a failure. `reproduce`
/// receives candidate subsequences; must be deterministic and side-effect
/// isolated (caller is responsible for resetting world state between calls).
/// Returns the smallest found prefix-preserving subsequence that still fails.
pub fn ddmin_minimize<T: Clone>(actions: &[T], reproduce: impl Fn(&[T]) -> bool) -> Vec<T> {
    let mut cur: Vec<T> = actions.to_vec();
    if cur.is_empty() || !reproduce(&cur) {
        return cur; // not reproducible (or empty) — return as-is
    }
    let mut n = 2;
    while cur.len() > 1 {
        let chunk = (cur.len() as f64 / n as f64).ceil() as usize;
        let mut removed = false;
        let mut i = 0;
        while i < cur.len() {
            let end = (i + chunk).min(cur.len());
            let candidate: Vec<T> = cur
                .iter()
                .enumerate()
                .filter(|(j, _)| *j < i || *j >= end)
                .map(|(_, a)| a.clone())
                .collect();
            if reproduce(&candidate) {
                cur = candidate;
                removed = true;
                // restart chunking from the shorter sequence
                break;
            }
            i += chunk;
        }
        if removed {
            n = (n - 1).max(2); // removal worked — try finer chunks
        } else {
            if n >= cur.len() {
                break; // 1-progress: try simpler minimality (pairwise removal below)
            }
            n = (n * 2).min(cur.len());
        }
    }
    // Final 1-minimality pass: try removing each single remaining action.
    loop {
        let mut single_removed = false;
        for i in 0..cur.len() {
            let candidate: Vec<T> = cur
                .iter()
                .enumerate()
                .filter(|(j, _)| *j != i)
                .map(|(_, a)| a.clone())
                .collect();
            if reproduce(&candidate) {
                cur = candidate;
                single_removed = true;
                break;
            }
        }
        if !single_removed {
            break;
        }
    }
    cur
}

// ---------------------------------------------------------------------------
// Crash minimization (ddmin)
// ---------------------------------------------------------------------------
//
/// Outcome of a crash-minimization session.
#[derive(Debug, Clone)]
pub struct MinimizeOutcome {
    /// Minimal subsequence of actions still reproducing the crash.
    pub minimal_actions: Vec<TimedAction>,
    /// A ready-to-save regression scenario that replays the minimal
    /// action sequence (persist to assets/scenarios/regression_*.json).
    pub regression_scenario: Scenario,
}

/// Parse a logged action string back into a replayable intent. Handles
/// both the legacy per-bot formats ("move:..", "choice:idx=..") and the
/// canonical structured form: action "intent:\<variant\>" with a JSON
/// payload in `details` (written by the IntentAudited observer for ALL
/// writers — chaos/replay/pursuit/planner/game adapters alike). This is
/// what makes crashes found by ANY bot minimizable via ddmin.
pub fn action_to_replay_intent(entry: &TimedAction) -> Option<ReplayIntent> {
    let a = &entry.action;
    // Canonical structured form: intent:<variant> + JSON details.
    if let Some(variant) = a.strip_prefix("intent:") {
        return match variant {
            "wait" => Some(ReplayIntent::Wait),
            _ => {
                let d = entry.details.as_deref()?;
                let v: serde_json::Value = serde_json::from_str(d).ok()?;
                match variant {
                    "move" => {
                        let arr = v.get("dir")?.as_array()?;
                        let x = arr.first()?.as_f64()? as f32;
                        let y = arr.get(1)?.as_f64()? as f32;
                        Some(ReplayIntent::Move { dir: (x, y) })
                    }
                    "choice" => Some(ReplayIntent::Choice {
                        index: v.get("index")?.as_u64()? as usize,
                    }),
                    "axis" => Some(ReplayIntent::Axis {
                        name: v.get("name")?.as_str()?.to_string(),
                        value: v.get("value")?.as_f64()? as f32,
                    }),
                    "select" => Some(ReplayIntent::Select {
                        target: v.get("target")?.as_str()?.to_string(),
                    }),
                    _ => None,
                }
            }
        };
    }
    // Legacy per-bot string formats (kept for compatibility with old logs).
    if let Some(rest) = a.strip_prefix("choice:idx=") {
        return rest
            .parse::<usize>()
            .ok()
            .map(|index| ReplayIntent::Choice { index });
    }
    if let Some(rest) = a.strip_prefix("select:name=") {
        return Some(ReplayIntent::Select {
            target: rest.to_string(),
        });
    }
    if a == "wait" {
        return Some(ReplayIntent::Wait);
    }
    None
}

/// Minimize a crashing scenario via ddmin over its recorded action log.
/// `app_builder` must build a FRESH App with the game's plugins (same
/// construction as the original run — `run_branch_matrix` pattern).
/// Returns the minimal reproducer and a ready-to-persist regression
/// scenario; `None` if the crash is not reproducible from the log alone
/// (e.g., it depends on wall-clock or unseeded randomness).
pub fn minimize_crash(
    app_builder: impl Fn() -> App,
    scenario: &Scenario,
    action_log: &[crate::contract::ActionEntry],
) -> Option<MinimizeOutcome> {
    let timed = action_log_to_timed_actions(action_log);
    // Only intent-bearing actions are replayable.
    let replayable: Vec<TimedAction> = timed
        .iter()
        .filter(|ta| action_to_replay_intent(ta).is_some())
        .cloned()
        .collect();
    if replayable.is_empty() {
        return None;
    }

    let reproduce = |candidate: &[TimedAction]| -> bool {
        let mut app = app_builder();
        let inputs: Vec<ReplayInput> = candidate
            .iter()
            .filter_map(|ta| {
                action_to_replay_intent(ta).map(|intent| ReplayInput {
                    frame: ta.frame,
                    intent,
                })
            })
            .collect();
        let replay_scenario = Scenario {
            tps: scenario.tps,
            simulated_time: scenario.simulated_time,
            setup: scenario.setup.clone(),
            duration_s: scenario.duration_s,
            bot: BotConfig {
                bot_type: crate::enums::BotType::Replay,
                seed: scenario.bot.seed,
                input_rate_hz: scenario.bot.input_rate_hz,
                agent_target: None,
                target: None,
                deadzone: scenario.bot.deadzone,
                goals: None,
                inputs,
                pointer_clicks: vec![],
                key_presses: vec![],
                persona: None,
            },
            invariants: scenario.invariants.clone(),
        };
        matches!(run_scenario(&mut app, &replay_scenario), Ok(r) if r.status == crate::enums::PlaytestStatus::Crash)
    };

    let minimal = ddmin_minimize(&replayable, reproduce);
    let inputs: Vec<ReplayInput> = minimal
        .iter()
        .filter_map(|ta| {
            action_to_replay_intent(ta).map(|intent| ReplayInput {
                frame: ta.frame,
                intent,
            })
        })
        .collect();
    let regression_scenario = Scenario {
        tps: scenario.tps,
        simulated_time: scenario.simulated_time,
        setup: scenario.setup.clone(),
        duration_s: scenario.duration_s,
        bot: BotConfig {
            bot_type: crate::enums::BotType::Replay,
            seed: scenario.bot.seed,
            input_rate_hz: scenario.bot.input_rate_hz,
            agent_target: None,
            target: None,
            deadzone: scenario.bot.deadzone,
            goals: None,
            inputs,
            pointer_clicks: vec![],
            key_presses: vec![],
            persona: None,
        },
        invariants: scenario.invariants.clone(),
    };
    Some(MinimizeOutcome {
        minimal_actions: minimal,
        regression_scenario,
    })
}

/// Serialize a regression scenario to pretty JSON for persistence.
pub fn regression_scenario_json(scenario: &Scenario) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(scenario)
}
