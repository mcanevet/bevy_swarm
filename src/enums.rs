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
    #[default]
    Curious,
    Aggressive,
    Idle,
}

impl std::fmt::Display for Persona {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckOp {
    Below,
    Above,
    Equals,
}

impl std::fmt::Display for CheckOp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CheckOp::Below => write!(f, "below"),
            CheckOp::Above => write!(f, "above"),
            CheckOp::Equals => write!(f, "equals"),
        }
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
