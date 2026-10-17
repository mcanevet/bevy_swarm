//! Data half of state.rs + PlaytestReport from driver.rs. Engine
//! bookkeeping (`#[derive(Resource)]`, World/Schedules probing) is
//! stripped.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};

// ---------------------------------------------------------------------------
// Violations — dedup by (rule, target), ordered by first_frame
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default, serde::Serialize, PartialEq)]
pub struct ViolationEntry {
    pub rule: String,
    pub target: String,
    pub first_frame: u64,
    pub last_frame: u64,
    pub count: u64,
    /// Detail + intent context at FIRST occurrence (reproduction anchor).
    pub detail: String,
    /// Detail at the most recent occurrence (trend/debug).
    pub last_detail: String,
    /// Stable failure identity (T1): computed at snapshot time from
    /// kind/rule/location/normalized detail.
    pub fingerprint: Option<crate::fingerprint::Fingerprint>,
    pub fingerprint_scheme: u32,
}

/// (Was `#[derive(Resource)]` — engine binding dropped.)
#[derive(Clone, Debug, Default)]
pub struct Violations {
    pub entries: BTreeMap<(String, String), ViolationEntry>,
    /// Latest bot intent in flight (set by bot systems each fire, read
    /// when reporting) — a violation carries the input that produced it.
    context: String,
}

impl Violations {
    pub fn set_context(&mut self, ctx: impl Into<String>) {
        self.context = ctx.into();
    }

    pub fn report(&mut self, rule: crate::rules::Rule, target: &str, detail: String, frame: u64) {
        self.report_named(rule.name(), target, detail, frame);
    }

    /// Report under a dynamic, per-scenario rule name (typed oracles /
    /// user-named invariants — not part of the `Rule` registry).
    pub fn report_named(&mut self, rule: &str, target: &str, detail: String, frame: u64) {
        let key = (rule.to_string(), target.to_string());
        let prefixed = format!("[intent: {}] {}", self.context, detail);
        let entry = self.entries.entry(key).or_insert_with(|| ViolationEntry {
            rule: rule.to_string(),
            target: target.to_string(),
            first_frame: frame,
            last_frame: frame,
            count: 0,
            detail: prefixed.clone(),
            last_detail: prefixed.clone(),
            fingerprint: None,
            fingerprint_scheme: 1,
        });
        entry.count += 1;
        entry.last_frame = frame;
        // FIRST occurrence's detail stays (reproduction anchor); only
        // the trend copy updates.
        entry.last_detail = prefixed;
    }

    /// Snapshot sorted by first_frame — callers store reports across
    /// scenarios; never hand out the live map.
    pub fn snapshot(&self) -> Vec<ViolationEntry> {
        let mut v: Vec<ViolationEntry> = self.entries.values().cloned().collect();
        // FX5: total order (first_frame, rule, target) for deterministic
        // fingerprinting across runs.
        v.sort_by(|a, b| {
            (a.first_frame, &a.rule, &a.target).cmp(&(b.first_frame, &b.rule, &b.target))
        });
        for e in &mut v {
            // Informational rules (wall-clock frame-time) are NOT
            // fingerprint-gated.
            if Self::INFORMATIONAL_RULES.iter().any(|r| r.name() == e.rule) {
                e.fingerprint = None;
                e.fingerprint_scheme = 1;
            } else {
                // Strip "[intent: ...]" prefix from detail before
                // fingerprinting.
                let raw_detail = e
                    .detail
                    .strip_prefix("[intent: ")
                    .and_then(|rest| rest.find("] "))
                    .map(|end| &e.detail["[intent: ".len() + end + 2..])
                    .unwrap_or(e.detail.as_str());
                let norm = crate::fingerprint::config_normalizer();
                e.fingerprint = Some(norm.fingerprint(
                    "violation",
                    &e.rule,
                    std::slice::from_ref(&e.target),
                    raw_detail,
                ));
                e.fingerprint_scheme = 1;
            }
        }
        v
    }
}

impl Violations {
    /// Wall-clock/informational rules excluded from fingerprint gating.
    /// le3.10 (RF6): kept in parity with Rule::is_informational().
    pub const INFORMATIONAL_RULES: &[crate::rules::Rule] = &[
        crate::rules::Rule::FrameTimeAnomaly,
        crate::rules::Rule::FrameTimeP99Below,
        crate::rules::Rule::FpsFloor,
        crate::rules::Rule::DeadWidget,
        crate::rules::Rule::PerfBudgetExceeded,
        crate::rules::Rule::EntropyUnattributedDraws,
        crate::rules::Rule::BevySuppressed,
        crate::rules::Rule::UnusedInput,
        crate::rules::Rule::UndrivableGame,
        crate::rules::Rule::BevyWarning,
        crate::rules::Rule::LogWarn,
        crate::rules::Rule::NothingOnScreen,
    ];
}

// ---------------------------------------------------------------------------
// Report data structs
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct Metrics {
    pub input_count: u64,
    pub frame_count: u64,
    pub frame_ms_p99: f64,
    pub worst_frame_ms: f64,
    pub crash_detected: bool,
}

/// Coverage metrics: counts of intent variants exercised, TestApi paths
/// read, and reset kinds invoked.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Coverage {
    pub intents_emitted: HashMap<String, u64>,
    pub test_api_paths_read: HashSet<String>,
    pub resets_invoked: HashSet<String>,
    /// For chaos bot: which SurfaceVariant indices were sampled
    pub chaos_surface_indices: HashSet<u64>,
}

/// Code-aware coverage (CA² pattern): which SCHEDULED SYSTEMS actually
/// executed during a scenario run.
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct SystemCoverage {
    pub executed: Vec<String>,
    pub registered: Vec<String>,
}

impl SystemCoverage {
    pub fn game_only(&self) -> SystemCoverage {
        SystemCoverage {
            executed: self
                .executed
                .iter()
                .filter(|n| !is_harness_system(n))
                .cloned()
                .collect(),
            registered: self
                .registered
                .iter()
                .filter(|n| !is_harness_system(n))
                .cloned()
                .collect(),
        }
    }

    /// Registered-but-never-executed systems — the coverage gaps.
    pub fn unexecuted(&self) -> Vec<String> {
        let exec: HashSet<&str> = self.executed.iter().map(|s| s.as_str()).collect();
        self.registered
            .iter()
            .filter(|s| !exec.contains(&s.as_str()))
            .cloned()
            .collect()
    }

    /// Fraction of registered systems that executed, 0.0..1.0.
    pub fn fraction(&self) -> f64 {
        if self.registered.is_empty() {
            return 1.0;
        }
        self.executed.len() as f64 / self.registered.len() as f64
    }
}

/// Harness-owned systems are excluded from game coverage.
pub fn is_harness_system(name: &str) -> bool {
    name.contains("bevy_playtest::") && !name.contains("bevy_playtest::verify::")
}

/// Published run report (bevy-free half of driver::PlaytestReport).
#[derive(Clone, Debug, serde::Serialize)]
pub struct PlaytestReport {
    pub schema_version: u32,
    pub status: crate::enums::PlaytestStatus,
    pub violations: Vec<ViolationEntry>,
    pub metrics: Metrics,
    pub coverage: Coverage,
    pub frame_count: u64,
    pub tps: u64,
    pub simulated_duration_s: f64,
    pub wall_time_s: f64,
    pub pre_ready_frames: u64,
    pub error: Option<String>,
    /// AUDIT: every injected input and cheat invocation.
    pub action_log: Vec<ActionEntry>,
    pub game_version: Option<String>,
    pub cheat_count: usize,
    pub unreplayable_actions: usize,
    pub warnings: Vec<String>,
    pub system_coverage: SystemCoverage,
    /// Per-frame state digests (A4), present only when a StateTrace
    /// resource was inserted by the caller.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_trace: Option<Vec<FrameDigest>>,
    /// I5: per-action effect rates.
    pub action_effect_rate: std::collections::HashMap<String, crate::effects::EffectRate>,
    /// J0: every bot random choice this run.
    pub choices: Vec<crate::choice::Choice>,
    /// J1: signed robustness margins (STL-style).
    pub robustness: crate::robustness::RobustnessSummary,
    /// Z4: input surface discovered by differential probing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub discovered_surface: Option<crate::discover::DiscoveredSurface>,
    /// Z1: tier + what actually ran (oracle families, actuators).
    pub scope: CheckedScope,
    /// Z19/Z20: which harness mode produced this report.
    pub harness_mode: String,
}

impl PlaytestReport {
    /// An empty Crash-status report.
    pub fn crashed_empty() -> Self {
        Self {
            schema_version: 1,
            status: crate::enums::PlaytestStatus::Crash,
            violations: vec![],
            metrics: Default::default(),
            coverage: Default::default(),
            frame_count: 0,
            tps: 0,
            simulated_duration_s: 0.0,
            wall_time_s: 0.0,
            pre_ready_frames: 0,
            error: None,
            action_log: vec![],
            game_version: None,
            cheat_count: 0,
            unreplayable_actions: 0,
            warnings: vec![],
            system_coverage: Default::default(),
            state_trace: None,
            action_effect_rate: Default::default(),
            choices: vec![],
            robustness: Default::default(),
            discovered_surface: None,
            scope: Default::default(),
            harness_mode: "in_process".to_string(),
        }
    }

    /// A scenario rejected before running gets a Crash-status report
    /// WITH a scenario_error violation.
    pub fn scenario_error(msg: &str) -> Self {
        let mut rep = Self::crashed_empty();
        rep.error = Some(msg.to_string());
        let scheme = crate::fingerprint::default_normalizer();
        let fp = scheme.fingerprint(
            "crash",
            crate::rules::Rule::ScenarioError.name(),
            &[],
            msg,
        );
        rep.violations = vec![ViolationEntry {
            rule: crate::rules::Rule::ScenarioError.to_string(),
            target: "scenario".to_string(),
            first_frame: 0,
            last_frame: 0,
            count: 1,
            detail: msg.to_string(),
            last_detail: msg.to_string(),
            fingerprint: Some(fp),
            fingerprint_scheme: 1,
        }];
        rep
    }
}

// ---------------------------------------------------------------------------
// DSL half of contract.rs (bevy-free)
// ---------------------------------------------------------------------------

/// Contract depth detected for this run.
#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Serialize,
    Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum ContractTier {
    #[default]
    Zero,
    Observable,
    Semantic,
}

impl std::fmt::Display for ContractTier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ContractTier::Zero => write!(f, "zero"),
            ContractTier::Observable => write!(f, "observable"),
            ContractTier::Semantic => write!(f, "semantic"),
        }
    }
}

/// Scope of what was actually checked during a run.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CheckedScope {
    pub tier: Option<ContractTier>,
    pub active: Vec<String>,
    pub inactive: Vec<(String, String)>,
    pub actuators: Vec<String>,
}

impl CheckedScope {
    /// Union of several run scopes (Z18 autotest aggregation).
    pub fn union(scopes: impl IntoIterator<Item = CheckedScope>) -> CheckedScope {
        let mut out = CheckedScope::default();
        for s in scopes {
            if out.tier.unwrap_or(ContractTier::Zero) < s.tier.unwrap_or(ContractTier::Zero) {
                out.tier = s.tier;
            }
            for a in s.active {
                if !out.active.contains(&a) {
                    out.active.push(a);
                }
            }
            for (fam, why) in s.inactive {
                if !out.inactive.iter().any(|(f, _)| *f == fam) {
                    out.inactive.push((fam, why));
                }
            }
            for a in s.actuators {
                if !out.actuators.contains(&a) {
                    out.actuators.push(a);
                }
            }
        }
        out
    }
}

/// Audit-log entry (contract.rs DSL half).
#[derive(Clone, Debug, serde::Serialize)]
pub struct ActionEntry {
    pub frame: u64,
    pub elapsed_ms: f64,
    pub source: ActionSource,
    pub action: String,
    pub details: Option<String>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub enum ActionSource {
    BotScenario(String),
    Agent,
    Manual,
}

/// Per-frame state digest (determinism.rs data half).
#[derive(Clone, Debug, serde::Serialize)]
pub struct FrameDigest {
    pub frame: u64,
    pub hash: u64,
}
