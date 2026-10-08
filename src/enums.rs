//! Type-safe enums replacing stringly-typed fields.
//!
//! All enums use `#[serde(rename_all = "...")]` so JSON serialization
//! is identical to the former strings (breaking-free for users).

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Bot types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BotType {
    Chaos,
    Pursuit,
    Replay,
    SyntheticPointer,
    SyntheticKeyboard,
    Planner,
    /// I3: user-registered typed bot policy (BotConfig.policy names it).
    Custom,
}

impl std::fmt::Display for BotType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BotType::Chaos => write!(f, "chaos"),
            BotType::Pursuit => write!(f, "pursuit"),
            BotType::Replay => write!(f, "replay"),
            BotType::SyntheticPointer => write!(f, "synthetic_pointer"),
            BotType::SyntheticKeyboard => write!(f, "synthetic_keyboard"),
            BotType::Planner => write!(f, "planner"),
            BotType::Custom => write!(f, "custom"),
        }
    }
}

impl From<BotType> for String {
    fn from(t: BotType) -> Self {
        t.to_string()
    }
}

// ---------------------------------------------------------------------------
// Persona configurations
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Persona {
    /// Default: uniform sampling over the declared surface.
    #[default]
    Uniform,
    /// Reserved for F1 (coverage-guided curiosity); currently behaves
    /// as uniform.
    Curious,
    Aggressive,
    Idle,
}

impl std::fmt::Display for Persona {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Persona::Uniform => write!(f, "uniform"),
            Persona::Curious => write!(f, "curious"),
            Persona::Aggressive => write!(f, "aggressive"),
            Persona::Idle => write!(f, "idle"),
        }
    }
}

// ---------------------------------------------------------------------------
// Invariant rule types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvariantRule {
    NodesInBounds,
    FrameTimeP99Below,
    FpsFloor,
    Custom,
    FrameTimeAnomaly,
}

impl std::fmt::Display for InvariantRule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InvariantRule::NodesInBounds => write!(f, "nodes_in_bounds"),
            InvariantRule::FrameTimeP99Below => write!(f, "frame_time_p99_below"),
            InvariantRule::FpsFloor => write!(f, "fps_floor"),
            InvariantRule::Custom => write!(f, "custom"),
            InvariantRule::FrameTimeAnomaly => write!(f, "frame_time_anomaly"),
        }
    }
}

// ---------------------------------------------------------------------------
// Check operators
// ---------------------------------------------------------------------------

/// Comparison operators with ONE semantics everywhere.
/// Wire format: `lt`, `le`, `gt`, `ge`, `equals`, `ne` (plus the legacy
/// aliases `below` = `le`, `above` = `ge`, `eq` = `equals`; all inclusive
/// boundaries).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckOp {
    Lt,
    #[serde(alias = "below")]
    Le,
    Gt,
    #[serde(alias = "above")]
    Ge,
    #[serde(alias = "eq")]
    Equals,
    Ne,
}

/// Relative+absolute tolerance so integer-valued counters compare
/// exactly and large floats don't need bit-equality.
fn approx_eq(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9 * a.abs().max(b.abs()).max(1.0)
}

impl CheckOp {
    /// Single source of truth for numeric comparison.
    pub fn holds(self, cur: f64, thr: f64) -> bool {
        match self {
            CheckOp::Lt => cur < thr,
            CheckOp::Le => cur <= thr,
            CheckOp::Gt => cur > thr,
            CheckOp::Ge => cur >= thr,
            CheckOp::Equals => approx_eq(cur, thr),
            CheckOp::Ne => !approx_eq(cur, thr),
        }
    }
    /// Text comparison: only Equals/Ne are meaningful; others -> None
    /// (validate_scenario rejects them at load time).
    pub fn holds_text(self, cur: &str, expect: &str) -> Option<bool> {
        match self {
            CheckOp::Equals => Some(cur == expect),
            CheckOp::Ne => Some(cur != expect),
            _ => None,
        }
    }
    pub fn symbol(self) -> &'static str {
        match self {
            CheckOp::Lt => "<",
            CheckOp::Le => "<=",
            CheckOp::Gt => ">",
            CheckOp::Ge => ">=",
            CheckOp::Equals => "==",
            CheckOp::Ne => "!=",
        }
    }
}

impl std::fmt::Display for CheckOp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.symbol())
    }
}

// ---------------------------------------------------------------------------
// Differential invariants
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DifferentialOp {
    NoDecrease,
    NoIncrease,
}

impl std::fmt::Display for DifferentialOp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DifferentialOp::NoDecrease => write!(f, "no_decrease"),
            DifferentialOp::NoIncrease => write!(f, "no_increase"),
        }
    }
}

// ---------------------------------------------------------------------------
// Playtest report status
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlaytestStatus {
    Pass,
    Fail,
    Crash,
}

impl std::fmt::Display for PlaytestStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlaytestStatus::Pass => write!(f, "pass"),
            PlaytestStatus::Fail => write!(f, "fail"),
            PlaytestStatus::Crash => write!(f, "crash"),
        }
    }
}

impl From<PlaytestStatus> for String {
    fn from(s: PlaytestStatus) -> Self {
        s.to_string()
    }
}

// ---------------------------------------------------------------------------
// Pointer buttons
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum PointerButton {
    #[default]
    Primary,
    Secondary,
    Middle,
}

impl std::fmt::Display for PointerButton {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PointerButton::Primary => write!(f, "primary"),
            PointerButton::Secondary => write!(f, "secondary"),
            PointerButton::Middle => write!(f, "middle"),
        }
    }
}
