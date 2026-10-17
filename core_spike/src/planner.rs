//! Planner bot, bevy-free data half (SPIKE).
//!
//! What moved: GoalNode (full declarative tree), goal_tree_needs_resolver,
//! PlannerStack (full stack machine incl. pop_success/check_postcondition),
//! and the harness-internal bookkeeping data.
//! What stayed behind (stubbed with todo!()): the two World-probing
//! runtimes — replay_intent_to_user and planner_bot_system — which
//! require bevy World/resource access and UserIntent construction.
//!
//! TestFieldValue/Violations come from local replacements.
#![allow(dead_code)]

use serde::{Deserialize, Serialize};

use crate::core_state::Violations;

fn default_any_max_s() -> f32 {
    10.0
}
fn default_repeat_max_s() -> f32 {
    10.0
}

/// A resolved TestApi field value — numeric or textual.
/// (Engine half in contract.rs uses reflection; this is the value side.)
#[derive(Debug, Clone, PartialEq)]
pub enum TestFieldValue {
    Numeric(f64),
    Text(String),
}

impl TestFieldValue {
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            TestFieldValue::Numeric(n) => Some(*n),
            _ => None,
        }
    }

    pub fn as_text(&self) -> Option<&str> {
        match self {
            TestFieldValue::Text(s) => Some(s),
            _ => None,
        }
    }
}

/// The trait a game's TestApi resource implements.
pub trait TestApiResolve {
    fn resolve(&self, path: &str) -> Option<TestFieldValue>;
}

/// Goal-structure (aplib-inspired): a declarative tree of TestApi-
/// predicate goals with combinators.
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum GoalNode {
    /// All children in order.
    Seq {
        #[serde(default)]
        children: Vec<GoalNode>,
    },
    /// First achievable alternative; alternatives have deadlines.
    Any {
        #[serde(default)]
        children: Vec<GoalNode>,
        #[serde(default = "default_any_max_s")]
        max_s: f32,
    },
    /// Retry child until it succeeds or max_s elapses.
    Repeat {
        child: Box<GoalNode>,
        #[serde(default = "default_repeat_max_s")]
        max_s: f32,
    },
    /// Leaf (I3): a registered typed goal predicate.
    Predicate {
        name: String,
        #[serde(default)]
        emit: Vec<ReplayIntent>,
    },
    /// Leaf: a TestApi predicate plus intents to emit while pursuing it.
    Primitive {
        path: String,
        /// "below" | "above" | "equals" (numeric), "equals" (text)
        check: crate::enums::CheckOp,
        value: serde_json::Value,
        #[serde(default)]
        emit: Vec<ReplayIntent>,
        #[serde(default)]
        min_gain: Option<f64>,
        #[serde(default)]
        max_gain: Option<f64>,
    },
}

pub fn goal_tree_needs_resolver(node: &GoalNode) -> bool {
    match node {
        GoalNode::Seq { children } => children.iter().any(goal_tree_needs_resolver),
        GoalNode::Any { children, .. } => children.iter().any(goal_tree_needs_resolver),
        GoalNode::Repeat { child, .. } => goal_tree_needs_resolver(child),
        GoalNode::Predicate { .. } => false,
        GoalNode::Primitive { path, .. } => path.starts_with("TestApi."),
    }
}

// ---------------------------------------------------------------------------
// Stack machine (data only — the runtime driver stays behind)
// ---------------------------------------------------------------------------

/// One runtime frame on the planner goal stack.
#[derive(Clone)]
struct PlannerEntry {
    goal: GoalNode,
    child: usize,
    /// Post-condition snapshot (aplib S.before).
    snap: Option<f64>,
    /// Absolute deadline tick.
    deadline: Option<u64>,
    activated: u64,
    emit_cursor: usize,
}

/// Persistent planner state.
#[derive(Default, Clone)]
pub struct PlannerStack(Vec<PlannerEntry>);

impl PlannerStack {
    /// True when no goal is active.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    fn push(
        &mut self,
        goal: &GoalNode,
        resolve: &dyn Fn(&str) -> Option<TestFieldValue>,
        frame: u64,
        deadline: Option<u64>,
    ) {
        let snap = match goal {
            GoalNode::Primitive {
                path,
                min_gain,
                max_gain,
                ..
            } if min_gain.is_some() || max_gain.is_some() => match resolve(path) {
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
            emit_cursor: 0,
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
                GoalNode::Predicate { name, .. } => format!("pred({})", name),
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
            } => Some((path.clone(), check.to_string(), value.to_string())),
            _ => None,
        })
    }
}

/// Pop a COMPLETED goal, cascade success upward: a finished child
/// completes its Any/Repeat parent immediately; a Seq parent keeps
/// going. Post-conditions are checked per popped primitive.
#[allow(dead_code)]
fn pop_success(
    stack: &mut PlannerStack,
    resolve: &dyn Fn(&str) -> Option<TestFieldValue>,
    violations: &mut Violations,
    frame: u64,
) {
    while let Some(entry) = stack.0.pop() {
        check_postcondition(&entry, resolve, violations, frame, &stack.trace());
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

/// Differential post-condition (aplib Hoare-triple style).
#[allow(dead_code)]
fn check_postcondition(
    entry: &PlannerEntry,
    resolve: &dyn Fn(&str) -> Option<TestFieldValue>,
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
    let (Some(TestFieldValue::Numeric(curr)), Some(start)) = (resolve(path), entry.snap) else {
        return;
    };
    let delta = curr - start;
    if let Some(min_g) = min_gain {
        if delta < *min_g {
            violations.report(
                crate::rules::Rule::PlannerPostcondition,
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
                crate::rules::Rule::PlannerPostcondition,
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

/// Convert a ReplayIntent to a UserIntent. (SPIKE STUB — needs bevy World
/// + Vec2 + identity resolution.)
pub fn replay_intent_to_user(_ri: &ReplayIntent, _world: ()) -> Option<()> {
    todo!("bevy-bound: identity resolution against the live World")
}

/// Planner bot system: drives the game toward goal-tree satisfaction.
/// (SPIKE STUB — needs &mut bevy World, resources, TypedRegistry.)
pub fn planner_bot_system(_world: ()) {
    todo!("bevy-bound: exclusive system over &mut World")
}

/// ReplayIntent type alias — full definition lives in scenario.rs
/// (bevy-free) and is re-exported here for the planner tree.
pub use crate::scenario::ReplayIntent;
