//! Runtime state resources: PlaytestState, metrics, coverage, violations.

use bevy::ecs::resource::Resource;
use bevy::ecs::world::World;
use std::collections::{HashMap, HashSet};

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
    /// kind/rule/location/normalized detail. Informational (wall-clock
    /// frame-time) rules are exempt.
    pub fingerprint: Option<crate::fingerprint::Fingerprint>,
    /// Which fingerprint scheme produced `fingerprint` (frozen at 1
    /// for v0.2; a documented migration bumps this).
    pub fingerprint_scheme: u32,
}

#[derive(Resource, Clone, Debug, Default)]
pub struct Violations {
    pub entries: HashMap<(String, String), ViolationEntry>,
    /// Latest bot intent in flight (set by bot systems each fire, read
    /// when reporting) — a violation carries the input that produced it.
    context: String,
}

impl Violations {
    pub fn set_context(&mut self, ctx: impl Into<String>) {
        self.context = ctx.into();
    }

    pub fn report(&mut self, rule: &str, target: &str, detail: String, frame: u64) {
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
    /// scenarios; never hand out the live map. Fingerprints (T1) are
    /// computed HERE so every report consumer sees them without the
    /// oracle systems needing the normalizer.
    pub fn snapshot(&self) -> Vec<ViolationEntry> {
        let mut v: Vec<ViolationEntry> = self.entries.values().cloned().collect();
        // FX5: total order (first_frame, rule, target) for deterministic
        // fingerprinting across runs. HashMap tie-breaking was random.
        v.sort_by(|a, b| {
            (a.first_frame, &a.rule, &a.target).cmp(&(b.first_frame, &b.rule, &b.target))
        });
        for e in &mut v {
            // Informational rules (wall-clock frame-time) are NOT
            // fingerprint-gated (U1 rule 9).
            if Self::INFORMATIONAL_RULES.contains(&e.rule.as_str()) {
                e.fingerprint = None;
                e.fingerprint_scheme = 1;
            } else {
                // Strip "[intent: ...]" prefix from detail before
                // fingerprinting — that prefix varies by seed. The
                // closing bracket is the first "] " boundary, not the
                // string end.
                let raw_detail = e
                    .detail
                    .strip_prefix("[intent: ")
                    .and_then(|rest| rest.find("] "))
                    .map(|end| &e.detail["[intent: ".len() + end + 2..])
                    .unwrap_or(e.detail.as_str());
                let norm = crate::fingerprint::default_normalizer();
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

    /// Wall-clock/informational rules excluded from fingerprint gating.
    pub const INFORMATIONAL_RULES: &[&str] = &[
        "frame_time_p99",
        "frame_time_worst",
        "entropy_unattributed_draws",
    ];
}

// ---------------------------------------------------------------------------
// Runtime state resources
// ---------------------------------------------------------------------------

#[derive(Resource)]
pub struct PlaytestState {
    pub frame: u64,
    pub tps: u64,
    pub metrics: Metrics,
    pub delta_windows: HashMap<String, std::collections::VecDeque<(u64, f64)>>, // rule -> samples
    pub(crate) warned_paths: HashSet<String>,
    pub coverage: Coverage,
    /// Execution-time oracle state (Welford per-frame stats).
    pub frame_timing: FrameTimingStats,
    /// Differential invariant history: path → last observed numeric value.
    /// Used by no_decrease/no_increase checks.
    pub(crate) api_history: HashMap<String, f64>,
    /// Eventually-mode state: rule → (deadline_s, satisfied_at_frame).
    /// Tracks whether an eventually-rule has been satisfied; deadline
    /// expiration without satisfaction triggers a violation.
    pub(crate) eventually_state: HashMap<String, (f32, Option<u64>)>,
    /// synthetic_pointer bot: queued press/release halves of click
    /// gestures awaiting their scheduled frames.
    pub(crate) pending_gestures: std::collections::VecDeque<crate::bots::PendingGesture>,
    /// synthetic_pointer actionability gates: gestures whose click must
    /// be verified against the hover map one frame after release.
    pub(crate) pending_actionability_checks:
        std::collections::VecDeque<crate::bots::PendingGesture>,
    /// synthetic_keyboard bot: key names whose release is due next frame.
    pub(crate) pending_key_releases: Vec<(String, u64)>,
    /// Frozen-world oracle: consecutive frames with zero Gameplay-entity
    /// Transform mutations (see `frozen_world_oracle_system`). Reports a
    /// soft-lock suspicion after 2s of silence in real-time games.
    pub frozen_frames: u64,
    /// Last frame where any liveness signal was observed (I4).
    pub last_alive_frame: u64,
    /// Last frame where an intent/action was consumed (I4 turn-based mode).
    pub last_intent_frame: Option<u64>,
    /// Z6: archetype count at the last gameplay-inference pass.
    pub inference_last_archetype_len: usize,
    /// FX1: change tick snapshotted at each frame START (tick_counter,
    /// before any schedule runs). Liveness windows compare against the
    /// tick recorded N FRAMES ago — the raw tick counter advances per
    /// system run, not per frame, so `now - k` spans microseconds.
    pub(crate) frame_start_ticks: std::collections::VecDeque<u32>,
    /// FX1: pending effect-window snapshots: (frame, tick) taken at
    /// action application time; the effect oracle consumes them.
    pub(crate) effect_window_snapshots: Vec<(u64, u32)>,
    /// Readiness gate: frames spent waiting for `GameReady` before
    /// scenario duration began accruing (UE IsReady analog).
    pub pre_ready_frames: u64,
    /// Planner bot persistent goal stack (empty = inactive/complete).
    pub planner: crate::planner::PlannerStack,
}

#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct Metrics {
    pub input_count: u64,
    pub frame_count: u64,
    pub frame_ms_p99: f64,
    pub worst_frame_ms: f64,
    pub crash_detected: bool,
}

/// Coverage metrics: counts of intent variants exercised, TestApi paths
/// read, and reset kinds invoked. Exported in PlaytestReport for
/// incremental-test selection (cf. SMART's 94% branch coverage).
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct Coverage {
    /// Map of intent variant name → count emitted by bots
    pub intents_emitted: HashMap<String, u64>,
    /// Set of TestApi paths resolved by custom invariants
    pub test_api_paths_read: HashSet<String>,
    /// Set of reset hook kinds invoked during setup
    pub resets_invoked: HashSet<String>,
    /// For chaos bot: which SurfaceVariant indices were sampled
    pub chaos_surface_indices: HashSet<u64>,
}

/// Code-aware coverage (CA² pattern): which SCHEDULED SYSTEMS actually
/// executed during a scenario run. Bevy's schedules are inspectable —
/// each system's `last_run` tick advances when it runs, so comparing
/// pre/post-run snapshots gives exact execution sets without
/// instrumentation. Systems that never ran are coverage gaps the chaos
/// bot (or the game author) should target next.
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct SystemCoverage {
    /// Fully-qualified "schedule::system_name" for every system that
    /// EXECUTED during the run.
    pub executed: Vec<String>,
    /// Every system REGISTERED in the app's schedules (executed or not).
    pub registered: Vec<String>,
}

impl SystemCoverage {
    /// View excluding this harness's own systems (they run every frame
    /// and would inflate fraction()); the bevy_ engine's systems are
    /// kept for engine-level debugging. Raw lists stay on self.
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
        let exec: std::collections::HashSet<&str> =
            self.executed.iter().map(|s| s.as_str()).collect();
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

/// Snapshot every scheduled system's identity + last_run tick, keyed by
/// "schedule::system". Schedules initialize lazily on first run OR via
/// app.finish(); this function initializes any that have not run yet
/// (see initialize() call below). run_scenario finishes plugin building
/// before the first snapshot.
pub fn snapshot_systems(world: &mut World) -> HashMap<String, u32> {
    let mut out = HashMap::new();
    // Take Schedules out of the world so each schedule can borrow the
    // world for lazy initialization (schedules init on first run).
    let Some(mut schedules) = world
        .remove_resource::<bevy::ecs::schedule::Schedules>()
        .map(|mut s| std::mem::take(&mut s))
    else {
        return out;
    };
    // Store each system's AGE: how far its last_run tick lags the world
    // tick, as a wrapping distance. Raw ticks wrap and get clamped by
    // check_change_ticks, so absolute comparisons lie; a shrinking age
    // ("ran more recently") is wrap-safe.
    let world_tick = world.change_tick().get();
    for (label, sched) in schedules.iter_mut() {
        let key = format!("{:?}", label);
        let _ = sched.initialize(world);
        if let Ok(systems) = sched.systems() {
            for (_id, system) in systems {
                let name = format!("{}::{}", key, system.name());
                let age = world_tick.wrapping_sub(system.get_last_run().get());
                out.insert(name, age);
            }
        }
    }
    world.insert_resource(schedules);
    out
}

/// Compute code-aware system coverage for a completed run: executed =
/// systems whose last_run tick ADVANCED past the pre-run snapshot.
/// A `before` snapshot of None-entry means the system was added
/// mid-run (still counts as executed).
/// Harness module paths: systems defined in these bevy_swarm modules
/// are the harness's own (they run every frame or under run conditions
/// and would inflate fraction()). Test-fixture or game systems defined
/// elsewhere in the crate (e.g. bevy_swarm::verify:: under cfg(test))
/// are NOT excluded.
const HARNESS_MODULES: &[&str] = &[
    "bevy_swarm::bots::",
    "bevy_swarm::oracles::",
    "bevy_swarm::planner::",
    "bevy_swarm::state::",
    "bevy_swarm::driver::",
    "bevy_swarm::scenario::",
    "bevy_swarm::contract::",
    "bevy_swarm::diagnostics::",
];

pub fn is_harness_system(name: &str) -> bool {
    HARNESS_MODULES.iter().any(|m| name.contains(m))
}

pub fn system_coverage(before: &HashMap<String, u32>, world: &mut World) -> SystemCoverage {
    let after = snapshot_systems(world);
    // Harness-owned systems are excluded: bots/oracles run every frame
    // and would inflate fraction(), or show up as spurious "unexecuted"
    // gaps when gated by run conditions. Engine systems (bevy_) are
    // kept; use SystemCoverage::game_only to drop them too.
    let after: HashMap<String, u32> = after
        .into_iter()
        .filter(|(name, _)| !is_harness_system(name))
        .collect();
    let mut cov = SystemCoverage {
        registered: after.keys().cloned().collect(),
        ..Default::default()
    };
    for (name, age) in &after {
        match before.get(name) {
            // Same-or-older age = did not run during the scenario.
            Some(prev) if age >= prev => {}
            _ => cov.executed.push(name.clone()),
        }
    }
    cov.registered.sort();
    cov.executed.sort();
    cov
}

impl PlaytestState {
    /// Construct a fresh playtest state for a run at `tps` ticks per
    /// second with the given RNG seed. Public entry point for callers
    /// driving the harness manually (e.g. calibration runs).
    pub fn new(tps: u64, _seed: u64) -> Self {
        // seed moved into ChoiceStream (J0); kept for API compat.
        PlaytestState {
            frame: 0,
            tps,
            metrics: Metrics::default(),
            delta_windows: HashMap::default(),
            warned_paths: HashSet::default(),
            coverage: Coverage::default(),
            frame_timing: FrameTimingStats::default(),
            api_history: HashMap::default(),
            eventually_state: HashMap::default(),
            frozen_frames: 0,
            last_alive_frame: 0,
            last_intent_frame: None,
            inference_last_archetype_len: 0,
            frame_start_ticks: Default::default(),
            effect_window_snapshots: Vec::new(),
            pre_ready_frames: 0,
            planner: crate::planner::PlannerStack::default(),
            pending_gestures: std::collections::VecDeque::new(),
            pending_actionability_checks: std::collections::VecDeque::new(),
            pending_key_releases: Vec::new(),
        }
    }

    pub fn elapsed_s(&self) -> f32 {
        self.frame as f32 / self.tps.max(1) as f32
    }
}

// ---------------------------------------------------------------------------

/// Relative-deviation gate: an anomalous frame must ALSO be this factor
/// above the mean. Guards against zero-variance histories (perfectly
/// steady frames would make 3σ hair-trigger) and ties the oracle to
/// human-meaningful slowdowns, not microsecond jitter.
const FRAME_TIME_ANOMALY_RELATIVE: f64 = 1.5;

/// Welford online stats for frame durations (ms), plus a bounded ring
/// buffer of recent samples backing a TRUE rolling p99 (nearest-rank).
#[derive(Clone, Debug)]
pub struct FrameTimingStats {
    mean_ms: f64,
    m2: f64,
    count: u64,
    /// Worst observed frame (ms)
    pub worst_ms: f64,
    /// Number of anomaly verdicts (each logged frame is one tick —
    /// dedup happens via Violations).
    pub anomalies: u64,
    /// Ring buffer of the last `ring_cap()` frame samples (ms),
    /// for rolling-quantile estimates.
    ring: std::collections::VecDeque<f64>,
}

const FRAME_TIMING_RING_CAP: usize = 600; // ten seconds @ 60 TPS

impl Default for FrameTimingStats {
    fn default() -> Self {
        Self {
            mean_ms: 0.0,
            m2: 0.0,
            count: 0,
            worst_ms: 0.0,
            anomalies: 0,
            ring: std::collections::VecDeque::with_capacity(FRAME_TIMING_RING_CAP),
        }
    }
}

impl FrameTimingStats {
    pub fn observe(&mut self, ms: f64) {
        self.count += 1;
        let delta = ms - self.mean_ms;
        self.mean_ms += delta / self.count as f64;
        self.m2 += delta * (ms - self.mean_ms);
        self.worst_ms = self.worst_ms.max(ms);
        if self.ring.len() == FRAME_TIMING_RING_CAP {
            self.ring.pop_front();
        }
        self.ring.push_back(ms);
    }

    /// TRUE rolling p99 (nearest-rank) over the last `ring_cap()`
    /// samples. Requires at least 100 samples; below that, returns
    /// None (a p99 of a handful of samples is noise).
    pub fn p99_ms(&self) -> Option<f64> {
        if self.ring.len() < 100 {
            return None;
        }
        let mut sorted: Vec<f64> = self.ring.iter().copied().collect();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let rank = ((sorted.len() as f64) * 0.99).ceil() as usize;
        sorted.get(rank.saturating_sub(1)).copied()
    }

    pub fn is_anomalous(&self, ms: f64) -> bool {
        if self.count < FRAME_TIME_ANOMALY_MIN_SAMPLES {
            return false;
        }
        let stddev = (self.m2 / self.count as f64).sqrt();
        let above_sigma = ms > self.mean_ms + FRAME_TIME_ANOMALY_SIGMA * stddev;
        let above_relative = ms > self.mean_ms * FRAME_TIME_ANOMALY_RELATIVE;
        above_sigma && above_relative
    }

    pub fn mean_ms(&self) -> f64 {
        self.mean_ms
    }
}

const FRAME_TIME_ANOMALY_MIN_SAMPLES: u64 = 60; // one second @ 60 TPS
pub(crate) const FRAME_TIME_ANOMALY_SIGMA: f64 = 3.0;
