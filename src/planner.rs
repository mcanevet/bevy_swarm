//! Planner bot (goal structures, aplib-inspired).
//!

//! A declarative tree of TestApi-predicate goals with combinators: a
//! scenario author writes WHAT to achieve, not WHEN to press. A
//! primitive goal declares the intents to emit while it is unachieved;
//! combinators control ordering, alternatives, and retries.

use crate::contract::{TestApi, TestApiResolve, TestFieldValue, UserIntent};
use crate::harness::{PlaytestState, ReplayIntent, ScenarioResource, Violations};
use bevy::ecs::world::World;
use serde::{Deserialize, Serialize};

fn default_any_max_s() -> f32 {
    10.0
}
fn default_repeat_max_s() -> f32 {
    10.0
}

/// Goal-structure (aplib-inspired): a declarative tree of TestApi-
/// predicate goals with combinators. A primitive goal declares the
/// intents to emit while it is unachieved; combinators control ordering,
/// alternatives, and retries. This is the high-level planner bot: a
/// scenario author writes WHAT to achieve, not WHEN to press.
#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum GoalNode {
    /// Achieve children in order.
    Seq {
        #[serde(default)]
        children: Vec<GoalNode>,
    },
    /// Achieve any one child: try in order; a child is abandoned after
    /// `max_s` seconds without achievement (default 10s), next child starts.
    Any {
        #[serde(default)]
        children: Vec<GoalNode>,
        #[serde(default = "default_any_max_s")]
        max_s: f32,
    },
    /// Repeat the child until achieved (bounded by max_s to fail loudly
    /// instead of looping forever).
    Repeat {
        child: Box<GoalNode>,
        #[serde(default = "default_repeat_max_s")]
        max_s: f32,
    },
    /// Leaf: a TestApi predicate plus intents to emit while pursuing it.
    /// Achieved when `path` satisfies `check` vs `value`.
    Primitive {
        path: String,
        /// "below" | "above" | "equals" (numeric), "equals" (text)
        check: String,
        value: serde_json::Value,
        #[serde(default)]
        emit: Vec<ReplayIntent>,
        /// Differential post-condition (aplib Hoare-triple style):
        /// when this goal is activated, snapshot `path`'s value; upon
        /// achievement, require the value changed by at least/at most
        /// `min_gain`/`max_gain` (numeric paths only).
        #[serde(default)]
        min_gain: Option<f64>,
        #[serde(default)]
        max_gain: Option<f64>,
    },
}

/// One runtime frame on the planner goal stack.
#[derive(Clone)]
struct PlannerEntry {
    goal: GoalNode,
    /// Combinators: index of the NEXT child to activate (pre-incremented
    /// at push, so the parent already points past the active child).
    child: usize,
    /// Post-condition snapshot (aplib S.before): value of the primitive's
    /// path captured when the goal was activated, checked at completion.
    snap: Option<f64>,
    /// Absolute deadline tick (Any alternatives / Repeat children only):
    /// when exceeded the child is abandoned.
    deadline: Option<u64>,
    /// Tick this entry was activated (for diagnostics).
    activated: u64,
}

/// Persistent planner state stored in PlaytestState across ticks. Keeping
/// the pursuit stack in state means: (a) progress survives ticks, (b) every
/// violation report can carry the full goal path (`seq[0]/prim(...)`).
#[derive(Default, Clone)]
pub struct PlannerStack(Vec<PlannerEntry>);

impl PlannerStack {
    /// True when no goal is active (either never started or fully done).
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    fn push(&mut self, goal: &GoalNode, api: &TestApi, frame: u64, deadline: Option<u64>) {
        let snap = match goal {
            GoalNode::Primitive {
                path,
                min_gain,
                max_gain,
                ..
            } if min_gain.is_some() || max_gain.is_some() => match api.resolve(path) {
                Some(TestFieldValue::Numeric(n)) => Some(n),
                _ => None,
            },
            _ => None,
        };
        self.0.push(PlannerEntry {
            goal: goal.clone(),
            child: 0,
            snap,
            deadline,
            activated: frame,
        });
    }

    /// Human-readable goal path for diagnostics, e.g.
    /// `seq[1]/any[0]/prim(TestApi.score)`.
    fn trace(&self) -> String {
        self.0
            .iter()
            .map(|e| match &e.goal {
                GoalNode::Seq { .. } => format!("seq[{}]", e.child),
                GoalNode::Any { .. } => format!("any[{}]", e.child),
                GoalNode::Repeat { .. } => "repeat".into(),
                GoalNode::Primitive { path, .. } => format!("prim({})", path),
            })
            .collect::<Vec<_>>()
            .join("/")
    }

    /// True when the goal tree is fully satisfied (stack drained).
    pub fn done(&self) -> bool {
        self.0.is_empty()
    }

    /// Deepest unfinished primitive's (path, check, value) for reports.
    fn deepest_unfinished_primitive(&self) -> Option<(String, String, String)> {
        self.0.iter().rev().find_map(|e| match &e.goal {
            GoalNode::Primitive {
                path, check, value, ..
            } => Some((path.clone(), check.clone(), value.to_string())),
            _ => None,
        })
    }
}

/// Pop a COMPLETED goal, cascade success upward: a finished child
/// completes its Any/Repeat parent immediately; a Seq parent keeps
/// going (its own child index already advanced). Post-conditions
/// (min_gain/max_gain vs snapshot) are checked per popped primitive.
fn pop_success(stack: &mut PlannerStack, api: &TestApi, violations: &mut Violations, frame: u64) {
    while let Some(entry) = stack.0.pop() {
        check_postcondition(&entry, api, violations, frame, &stack.trace());
        match stack.0.last() {
            Some(parent)
                if matches!(parent.goal, GoalNode::Any { .. } | GoalNode::Repeat { .. }) =>
            {
                continue; // success propagates through Any/Repeat
            }
            _ => break, // Seq absorbs child completion
        }
    }
}

/// Differential post-condition (aplib Hoare-triple style): the primitive's
/// path must have changed by at least/at most min_gain/max_gain between
/// goal activation and completion.
fn check_postcondition(
    entry: &PlannerEntry,
    api: &TestApi,
    violations: &mut Violations,
    frame: u64,
    trace: &str,
) {
    let GoalNode::Primitive {
        path,
        min_gain,
        max_gain,
        ..
    } = &entry.goal
    else {
        return;
    };
    let (Some(TestFieldValue::Numeric(curr)), Some(start)) = (api.resolve(path), entry.snap) else {
        return;
    };
    let delta = curr - start;
    if let Some(min_g) = min_gain {
        if delta < *min_g {
            violations.report(
                "planner_postcondition",
                path,
                format!(
                    "[goal {}] achieved but {} changed by {:.3}, min_gain {} — effect weaker than the contract",
                    trace, path, delta, min_g
                ),
                frame,
            );
        }
    }
    if let Some(max_g) = max_gain {
        if delta > *max_g {
            violations.report(
                "planner_postcondition",
                path,
                format!(
                    "[goal {}] achieved but {} changed by {:.3}, max_gain {} — effect stronger than the contract",
                    trace, path, delta, max_g
                ),
                frame,
            );
        }
    }
}

/// Convert a ReplayIntent to a UserIntent. Select intents resolve by
/// Name against live Gameplay entities (stable across resets).
fn replay_intent_to_user(ri: &ReplayIntent) -> Option<UserIntent> {
    match ri {
        ReplayIntent::Move { dir } => Some(UserIntent::Move {
            dir: bevy::math::Vec2::new(dir.0, dir.1),
        }),
        ReplayIntent::Choice { index } => Some(UserIntent::Choice { index: *index }),
        ReplayIntent::Axis { name, value } => Some(UserIntent::Axis {
            name: name.clone(),
            value: *value,
        }),
        // Planner primitives name targets symbolically via TestApi
        // predicates, not entity ids — Select is resolved at the game layer.
        ReplayIntent::Select { .. } | ReplayIntent::Wait => Some(UserIntent::Wait),
    }
}

/// Coverage-variant name for an emitted replay intent.
fn replay_ctx(ri: &ReplayIntent) -> String {
    match ri {
        ReplayIntent::Move { dir } => format!("move:x={:.2},y={:.2}", dir.0, dir.1),
        ReplayIntent::Choice { index } => format!("choice:idx={}", index),
        ReplayIntent::Axis { name, value } => format!("axis:{}={:.2}", name, value),
        ReplayIntent::Select { target } => format!("select:name={}", target),
        ReplayIntent::Wait => "wait".into(),
    }
}

/// Check whether a primitive goal's TestApi path satisfies its check.
fn primitive_achieved(api: &TestApi, path: &str, check: &str, value: &serde_json::Value) -> bool {
    match api.resolve(path) {
        Some(TestFieldValue::Numeric(n)) => {
            if let Some(thr) = value.as_f64() {
                match check {
                    "below" => n < thr,
                    "above" => n > thr,
                    "equals" => (n - thr).abs() <= f64::EPSILON,
                    _ => false,
                }
            } else {
                false
            }
        }
        Some(TestFieldValue::Text(s)) => {
            matches!(value, serde_json::Value::String(expect) if check == "equals" && *s == *expect)
        }
        None => false,
    }
}

/// Planner bot system: drives the game toward goal-tree satisfaction.
/// Stack-machine evaluation with persistent state across ticks, goal-path
/// traces in violation reports, deadline-bounded Any alternatives /
/// Repeat attempts, and differential post-conditions on primitives.
/// Exclusive system: needs `&mut World` to resolve Select intents by
/// Name against live Gameplay entities (entity-id-free, reset-stable).
pub fn planner_bot_system(world: &mut World) {
    // Use scoped borrows so each section ends its mutable world borrow.
    let is_planner =
        world.resource::<ScenarioResource>().0.bot.bot_type == crate::enums::BotType::Planner;
    if !is_planner {
        return;
    }
    let (planner_stack, frame, tps) = {
        let state = world.resource::<PlaytestState>();
        (state.planner.clone(), state.frame, state.tps)
    };
    let Some(goal_root) = world.resource::<ScenarioResource>().0.bot.goals.clone() else {
        world.resource_mut::<Violations>().report(
            "planner_bot_config",
            "",
            "planner bot requires `goals` (goal structure)".to_string(),
            frame,
        );
        return;
    };

    // planner_stack copied out; written back before returning. First
    // tick initializes with the root.
    let mut planner = planner_stack;
    if planner.is_empty() {
        let api = world.resource::<TestApi>().clone();
        planner.push(&goal_root, &api, frame, None);
    }

    // Intent to emit this tick (at most one): (intent, variant ctx, trace).
    let mut emitted: Option<(UserIntent, String, String)> = None;
    let api = world.resource::<TestApi>().clone();
    let mut violations = world.resource_mut::<Violations>();
    let mut goals_done: u64 = 0;

    loop {
        // Deadline sweep: abandon expired Any alternatives / Repeat
        // attempts. Parent combinators pre-advance their child index at
        // push, so popping the child moves straight to the next one.
        if let Some(top) = planner.0.last() {
            if let Some(dl) = top.deadline {
                if frame > dl {
                    planner.0.pop();
                    if let Some(parent) = planner.0.last() {
                        if matches!(parent.goal, GoalNode::Repeat { .. }) {
                            let tr = planner.trace();
                            violations.report(
                                "planner_goal_timeout",
                                &tr,
                                "Repeat exceeded max_s without achieving the goal — scenario stuck"
                                    .to_string(),
                                frame,
                            );
                            planner.0.pop();
                        }
                    }
                    continue;
                }
            }
        }

        let Some(entry) = planner.0.last() else { break };
        match &entry.goal {
            GoalNode::Seq { children } => {
                if entry.child >= children.len() {
                    pop_success(&mut planner, &api, &mut violations, frame);
                } else {
                    let child = children[entry.child].clone();
                    planner.0.last_mut().unwrap().child += 1;
                    planner.push(&child, &api, frame, None);
                }
            }
            GoalNode::Any { children, max_s } => {
                if entry.child >= children.len() {
                    let tr = planner.trace();
                    violations.report(
                        "planner_any_exhausted",
                        &tr,
                        format!(
                            "any goal exhausted all {} alternatives without success",
                            children.len()
                        ),
                        frame,
                    );
                    break;
                }
                let child = children[entry.child].clone();
                let dl = frame + (max_s * tps as f32) as u64;
                planner.0.last_mut().unwrap().child += 1;
                planner.push(&child, &api, frame, Some(dl));
            }
            GoalNode::Repeat { child, max_s } => {
                // Push the child once (child == 0); afterwards this frame
                // sits under the active child until it completes or the
                // deadline sweep abandons it.
                if entry.child == 0 {
                    let c = child.as_ref().clone();
                    let dl = frame + (max_s * tps as f32) as u64;
                    planner.0.last_mut().unwrap().child = 1;
                    planner.push(&c, &api, frame, Some(dl));
                } else {
                    break; // child is in flight on the stack above us
                }
            }
            GoalNode::Primitive {
                path,
                check,
                value,
                emit,
                ..
            } => {
                if primitive_achieved(&api, path, check, value) {
                    pop_success(&mut planner, &api, &mut violations, frame);
                    goals_done += 1;
                } else {
                    // Unachieved: emit pursuit intents (one per tick).
                    if !emit.is_empty() {
                        let idx = (frame as usize) % emit.len();
                        let ri = &emit[idx];
                        if let Some(intent) = replay_intent_to_user(ri) {
                            let ctx = replay_ctx(ri);
                            emitted = Some((intent, ctx, planner.trace()));
                        }
                    }
                    break;
                }
            }
        }
        if emitted.is_some() {
            break;
        }
    }

    // Write results back into state (single mutable borrow region).
    // Write results back into state (single mutable borrow region).
    {
        let state = &mut world.resource_mut::<PlaytestState>();
        if goals_done > 0 {
            state
                .coverage
                .intents_emitted
                .entry("goal_primitive_done".into())
                .and_modify(|c| *c += goals_done)
                .or_insert(goals_done);
        }
        if let Some((intent, ctx, trace)) = &emitted {
            state
                .coverage
                .intents_emitted
                .entry(ctx.clone())
                .and_modify(|c| *c += 1)
                .or_insert(1);
            state.planner = planner;
            world
                .resource_mut::<Violations>()
                .set_context(format!("planner:{} [goal {}]", ctx, trace));
            world.write_message(intent.clone());
        } else {
            state.planner = planner;
        }
    }
}

/// Planner end-of-run check: if the goal stack is not EMPTY, the top
/// unfinished primitive never achieved its condition — a goal that
/// was pursued but not reached. This catches bugs whose symptom is
/// "progress stalls", which invariants and post-conditions cannot
/// see (post-conditions only fire on ACHIEVED goals).
pub fn planner_unfinished_check(
    planner: &PlannerStack,
    state: &PlaytestState,
) -> Option<crate::harness::ViolationEntry> {
    let top = planner.0.last()?;
    let trace = planner.trace();
    let (ppath, pcheck, pvalue) = planner.deepest_unfinished_primitive().unwrap_or_default();
    Some(crate::harness::ViolationEntry {
        rule: "planner_goal_unfinished".into(),
        target: ppath.clone(),
        first_frame: top.activated,
        last_frame: state.frame,
        count: 1,
        detail: format!(
            "[goal {}] run ended with goal unfinished: {} {} {} never became true — pursued but never achieved",
            trace, ppath, pcheck, pvalue
        ),
    })
}
