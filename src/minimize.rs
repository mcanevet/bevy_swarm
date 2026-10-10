//! Crash minimization: ddmin over recorded action logs.

use bevy::app::App;

use crate::driver::{run_scenario, PlaytestReport};
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
/// Parse a logged action string back into a replayable intent. Handles
/// both the legacy per-bot formats ("move:..", "choice:idx=..") and the
/// canonical structured form: action "intent:\<variant\>" with a JSON
/// payload in `details` (written by intent_audit_log_system for ALL
/// intent writers — chaos/replay/pursuit/planner alike). This is what
/// makes failures found by ANY bot minimizable via ddmin.
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
                    "select" => {
                        // Prefer StableId (works for unnamed entities, I1);
                        // fall back to Name.
                        let target = if let Some(sid) = v.get("stable_id").and_then(|s| s.as_u64())
                        {
                            SelectTarget::Stable { stable_id: sid }
                        } else {
                            SelectTarget::Name(v.get("target")?.as_str()?.to_string())
                        };
                        Some(ReplayIntent::Select { target })
                    }
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
            target: SelectTarget::Name(rest.to_string()),
        });
    }
    if let Some(rest) = a.strip_prefix("select:stable=") {
        return rest
            .parse::<u64>()
            .ok()
            .map(|stable_id| ReplayIntent::Select {
                target: SelectTarget::Stable { stable_id },
            });
    }
    if a == "wait" {
        return Some(ReplayIntent::Wait);
    }
    None
}

/// What "still failing" means during shrinking. Matching the ORIGINAL
/// failure (not just any failure) prevents slippage to a different bug.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum FailureSignature {
    Crash,
    /// A violation with this rule (and target, if Some) is present.
    Violation {
        rule: String,
        target: Option<String>,
    },
}

impl FailureSignature {
    /// The primary failure of a report: Crash if crashed, else the
    /// earliest violation (`violations` is sorted by first_frame).
    pub fn of(report: &PlaytestReport) -> Option<Self> {
        if report.status == crate::enums::PlaytestStatus::Crash {
            return Some(Self::Crash);
        }
        report.violations.first().map(|v| Self::Violation {
            rule: v.rule.clone(),
            target: if v.target.is_empty() {
                None
            } else {
                Some(v.target.clone())
            },
        })
    }

    /// Does this report exhibit the same failure?
    pub fn matches(&self, report: &PlaytestReport) -> bool {
        match self {
            Self::Crash => report.status == crate::enums::PlaytestStatus::Crash,
            Self::Violation { rule, target } => report.violations.iter().any(|v| {
                v.rule == *rule
                    && match target {
                        Some(t) => v.target == *t,
                        None => true,
                    }
            }),
        }
    }
}

/// Errors from failure minimization.
#[derive(Debug)]
pub enum MinimizeError {
    /// The log contains no replayable intents (see report.unreplayable_actions).
    NoReplayableActions,
    /// Replaying the full log does not reproduce the signature. The
    /// failure depends on something outside the action log (wall clock,
    /// unseeded RNG, game state not covered by resets).
    NotReproducible {
        signature: FailureSignature,
    },
    Scenario(ScenarioError),
}

impl std::fmt::Display for MinimizeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoReplayableActions => write!(f, "no replayable actions in the log"),
            Self::NotReproducible { .. } => write!(
                f,
                "full action log does not reproduce the failure — it depends on \
                 something outside the log (wall clock, unseeded RNG, state not \
                 covered by resets)"
            ),
            Self::Scenario(e) => write!(f, "{}", e),
        }
    }
}
impl std::error::Error for MinimizeError {}

/// Outcome of a failure-minimization session.
#[derive(Debug, Clone)]
pub struct MinimizeOutcome {
    pub signature: FailureSignature,
    /// Minimal subsequence of replay inputs still reproducing the failure.
    /// May legitimately be EMPTY ("this bug needs no player input").
    pub minimal_inputs: Vec<ReplayInput>,
    /// A ready-to-save regression scenario that replays the minimal input
    /// sequence at the trimmed duration.
    pub regression_scenario: Scenario,
    /// Cost accounting: how many replay runs the minimization performed.
    pub replays_run: usize,
}

/// Build a replay-bot scenario from typed inputs, preserving the original
/// scenario's timing/config fields and setup (cheats stay in setup.cheats —
/// they are NOT minimized, the cheat scheduler replays them verbatim).
fn replay_scenario(scenario: &Scenario, inputs: Vec<ReplayInput>, duration_s: f32) -> Scenario {
    Scenario {
        name: scenario.name.clone(),
        tps: scenario.tps,
        simulated_time: scenario.simulated_time,
        single_threaded: scenario.single_threaded,
        deny_ambiguities: scenario.deny_ambiguities,
        setup: scenario.setup.clone(),
        duration_s,
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
            raw_surface: None,
            persona: None,
            policy: None,
            choices: None,
            continuation: crate::choice::Continuation::default(),
        },
        invariants: scenario.invariants.clone(),
        oracles: scenario.oracles.clone(),
        liveness: scenario.liveness.clone(),
    }
}

/// First frame at which the signature's failure occurs (Crash: the
/// report's frame count; Violation: first matching violation frame).
fn first_failure_frame(report: &PlaytestReport, sig: &FailureSignature) -> u64 {
    match sig {
        FailureSignature::Crash => report.frame_count,
        FailureSignature::Violation { rule, target } => report
            .violations
            .iter()
            .find(|v| {
                v.rule == *rule
                    && match target {
                        Some(t) => v.target == *t,
                        None => true,
                    }
            })
            .map(|v| v.first_frame)
            .unwrap_or(0),
    }
}

/// Shrink ANY failure (crash or invariant violation) found by a prior run.
///
/// 1. Honesty check: the full action log must reproduce the failure
///    signature, else `Err(NotReproducible)` (never a bogus "minimal" log).
/// 2. ddmin over the typed replay inputs.
/// 3. Duration trim: failure frame + 1s margin, if it still reproduces.
///
/// An empty `minimal_inputs` is a valid result: the failure reproduces
/// with no player input at all.
pub fn minimize_failure(
    app_builder: impl Fn() -> App,
    scenario: &Scenario,
    report: &PlaytestReport,
    signature: Option<FailureSignature>,
) -> Result<MinimizeOutcome, MinimizeError> {
    let sig = signature.or_else(|| FailureSignature::of(report)).ok_or(
        MinimizeError::NotReproducible {
            signature: FailureSignature::Crash,
        },
    )?;
    let inputs = replayable_inputs(&report.action_log);
    if inputs.is_empty() {
        return Err(MinimizeError::NoReplayableActions);
    }
    let replays = std::cell::Cell::new(0usize);
    let reproduce = |cand: &[ReplayInput], dur: f32| {
        replays.set(replays.get() + 1);
        let mut app = app_builder();
        let rs = replay_scenario(scenario, cand.to_vec(), dur);
        matches!(run_scenario(&mut app, &rs), Ok(r) if sig.matches(&r))
    };
    // 1. Honesty check: the FULL log must reproduce.
    if !reproduce(&inputs, scenario.duration_s) {
        return Err(MinimizeError::NotReproducible { signature: sig });
    }
    // 2. ddmin over inputs at the original duration.
    let minimal = ddmin_minimize(&inputs, |c| reproduce(c, scenario.duration_s));
    // 3. Duration trim: failure frame + 1s margin.
    let fail_frame = first_failure_frame(report, &sig);
    let trimmed = ((fail_frame as f32 / scenario.tps as f32) + 1.0).min(scenario.duration_s);
    let duration = if reproduce(&minimal, trimmed) {
        trimmed
    } else {
        scenario.duration_s
    };
    Ok(MinimizeOutcome {
        regression_scenario: replay_scenario(scenario, minimal.clone(), duration),
        signature: sig,
        minimal_inputs: minimal,
        replays_run: replays.get(),
    })
}

/// Extract the typed replay inputs from an action log.
pub fn replayable_inputs(action_log: &[crate::contract::ActionEntry]) -> Vec<ReplayInput> {
    action_log_to_timed_actions(action_log)
        .iter()
        .filter_map(|ta| {
            action_to_replay_intent(ta).map(|intent| ReplayInput {
                frame: ta.frame,
                intent,
            })
        })
        .collect()
}

/// Serialize a regression scenario to pretty JSON for persistence.
pub fn regression_scenario_json(scenario: &Scenario) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(scenario)
}
