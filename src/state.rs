//! Runtime state resources: PlaytestState, metrics, coverage, violations.

use bevy::ecs::resource::Resource;
use bevy::ecs::world::World;
use std::collections::BTreeMap;
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

    /// Wall-clock/informational rules excluded from fingerprint gating.
    pub const INFORMATIONAL_RULES: &[&str] = &[
        "frame_time_p99",
        "frame_time_worst",
        "frame_time_anomaly",
        "frame_time_p99_below",
        "fps_floor",
        "dead_widget",
        "perf_budget_exceeded",
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
    /// FX12 (I4): one-shot — liveness.mode was off this run.
    pub(crate) warned_liveness_off: bool,
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
    /// Consecutive liveness-sample quanta with NO state change after
    /// an intent (turn-based stuck streak). Unlike last_intent_frame,
    /// a continuous stream of IGNORED intents keeps accumulating this
    /// counter — a swallow-everything softlock is detected even while
    /// the bot keeps trying.
    pub stuck_after_intent_quanta: u64,
    /// Z6: archetype count at the last gameplay-inference pass.
    pub inference_last_archetype_len: usize,
    /// Entity count at last gameplay-inference pass. Retrigger also
    /// when entities join EXISTING archetypes (spawn into an already
    /// seen archetype does not grow the archetype count).
    pub inference_last_entity_len: u32,
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
/// Snapshot of system last-run ages plus the world tick it was taken
/// at. 716.32: ages from different snapshots are only comparable via
/// their snapshot ticks — comparing raw ages misclassifies bursty
/// systems that ran once early in a long scenario.
pub struct SystemSnapshot {
    pub ages: HashMap<String, u32>,
    pub tick: u32,
}

/// Take a snapshot; convenience wrapper preserving the old signature.
pub fn snapshot_systems(world: &mut World) -> HashMap<String, u32> {
    snapshot_systems_with_tick(world).ages
}

pub fn snapshot_systems_with_tick(world: &mut World) -> SystemSnapshot {
    let mut out = HashMap::new();
    // Take Schedules out of the world so each schedule can borrow the
    // world for lazy initialization (schedules init on first run).
    let Some(mut schedules) = world
        .remove_resource::<bevy::ecs::schedule::Schedules>()
        .map(|mut s| std::mem::take(&mut s))
    else {
        return SystemSnapshot {
            ages: out,
            tick: world.change_tick().get(),
        };
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
    SystemSnapshot {
        ages: out,
        tick: world_tick,
    }
}

/// Compute code-aware system coverage for a completed run: executed =
/// systems whose last_run tick ADVANCED past the pre-run snapshot.
/// A `before` snapshot of None-entry means the system was added
/// mid-run (still counts as executed).
/// FX12 (C3): harness systems are excluded from game coverage. The
/// original hardcoded list of 8 modules silently went stale — newer
/// harness modules (liveness, effects, raw_input, identity, sinks,
/// game_types) inflated game coverage. Matching the whole `bevy_swarm`
/// crate prefix is exhaustive by construction: every harness system
/// lives there, and games/fxtures define their systems elsewhere.
/// bevy_swarm::verify:: under cfg(test) is exempted explicitly so
/// coverage of TEST-game systems is still measurable in unit tests.
pub fn is_harness_system(name: &str) -> bool {
    name.contains("bevy_swarm::") && !name.contains("bevy_swarm::verify::")
}

pub fn system_coverage(before: &HashMap<String, u32>, world: &mut World) -> SystemCoverage {
    let snap = snapshot_systems_with_tick(world);
    // Legacy call sites have no before-tick: fall back to age comparison
    // with the after-tick span correction unavailable. Build a snapshot
    // with a plausible tick: 716.32 — without tick_before the sound
    // comparison is impossible; approximate with span = max observed
    // age delta. For legacy paths (unit tests) we use tick 0 semantics:
    // ages compared directly (old behavior).
    coverage_from(before, &snap.ages, None)
}

/// 716.32: sound system coverage. Compares LAST-RUN TICKS, not raw
/// ages: a system that ran once shortly before the pre-run snapshot
/// (prev age 1-2) and once EARLY in a long scenario has a large
/// after-age that is >= prev — raw age comparison misclassified it as
/// "did not run". Converting both ages to last_run_tick
/// (tick_snapshot - age) and comparing those tick values directly
/// classifies bursty systems correctly.
pub fn system_coverage_from_snapshots(
    before: &SystemSnapshot,
    world: &mut World,
) -> SystemCoverage {
    let after = snapshot_systems_with_tick(world);
    coverage_from(&before.ages, &after.ages, Some((before.tick, after.tick)))
}

fn coverage_from(
    before: &HashMap<String, u32>,
    after: &HashMap<String, u32>,
    ticks: Option<(u32, u32)>,
) -> SystemCoverage {
    // Harness-owned systems are excluded: bots/oracles run every frame
    // and would inflate fraction(), or show up as spurious "unexecuted"
    // gaps when gated by run conditions. Engine systems (bevy_) are
    // kept; use SystemCoverage::game_only to drop them too.
    let after: HashMap<String, u32> = after
        .iter()
        .filter(|(name, _)| !is_harness_system(name))
        .map(|(k, v)| (k.clone(), *v))
        .collect();
    let mut cov = SystemCoverage {
        registered: after.keys().cloned().collect(),
        ..Default::default()
    };
    for (name, age) in &after {
        match before.get(name) {
            Some(prev) => {
                if let Some((tick_before, tick_after)) = ticks {
                    // 716.32: sound classification under Bevy's change-
                    // tick CLAMPING (check_change_ticks rebases stale
                    // last_run values, so absolute tick comparisons
                    // lie). Sound invariant: the world tick advanced
                    // by span = tick_after - tick_between between
                    // snapshots. A system whose last run happened
                    // DURING the run has age_after < span (its last
                    // run is more recent than the pre-snapshot). A
                    // system that last ran before the snapshot has
                    // age_after >= span (age grew by at least span).
                    // Never-initialized systems keep the huge clamped
                    // age → not executed. This classifies bursty
                    // systems correctly where raw age comparison
                    // (age >= prev → "did not run") lied.
                    let span = tick_after.wrapping_sub(tick_before);
                    if *age < span {
                        cov.executed.push(name.clone());
                    }
                } else if age < prev {
                    // Legacy fallback (no tick info): shrinking age =
                    // ran more recently.
                    cov.executed.push(name.clone());
                }
            }
            // Before missing → system added mid-run → executed.
            None => cov.executed.push(name.clone()),
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
            warned_liveness_off: false,
            coverage: Coverage::default(),
            frame_timing: FrameTimingStats::default(),
            api_history: HashMap::default(),
            eventually_state: HashMap::default(),
            frozen_frames: 0,
            last_alive_frame: 0,
            last_intent_frame: None,
            stuck_after_intent_quanta: 0,
            inference_last_archetype_len: 0,
            inference_last_entity_len: 0,
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

#[cfg(test)]
mod tests_716_32 {
    use super::*;

    /// 716.32: a system that ran once shortly BEFORE the pre-run
    /// snapshot (prev age 2) and once EARLY in a long scenario
    /// (age_after large) must be classified EXECUTED. Raw age
    /// comparison said "did not run".
    #[test]
    fn early_runner_in_long_scenario_classified_executed() {
        // Ran 2 ticks before the pre-run snapshot, then once EARLY in
        // the run (tick 1_100 of a 1_000→10_000 span): age_after
        // (8_900) >= prev (2) — raw age comparison said "did not run".
        let tick_before = 1000u32;
        let tick_after = 10_000u32;
        let before = HashMap::from([("bursty".to_string(), 2u32)]);
        let after = HashMap::from([("bursty".to_string(), 8_900u32)]);

        let cov = coverage_from(&before, &after, Some((tick_before, tick_after)));
        assert!(
            cov.executed.iter().any(|n| n == "bursty"),
            "early runner must be classified executed, got {:?}",
            cov.executed
        );
    }

    /// A system that did NOT run during the scenario must stay
    /// unexecuted: age grows by at least the span.
    #[test]
    fn non_runner_classified_not_executed() {
        let tick_before = 1000u32;
        let tick_after = 10_000u32;
        let before = HashMap::from([("idle".to_string(), 2u32)]);
        let after = HashMap::from([("idle".to_string(), 9_002u32)]);

        let cov = coverage_from(&before, &after, Some((tick_before, tick_after)));
        assert!(
            !cov.executed.iter().any(|n| n == "idle"),
            "non-runner must not be classified executed"
        );
    }

    /// Never-initialized system (huge clamped age) stays unexecuted.
    #[test]
    fn never_initialized_stays_unexecuted() {
        let tick_before = 1000u32;
        let tick_after = 10_000u32;
        let before = HashMap::from([("never".to_string(), 3_258_167_296u32)]);
        let after = HashMap::from([("never".to_string(), 3_258_166_730u32)]);

        let cov = coverage_from(&before, &after, Some((tick_before, tick_after)));
        assert!(
            !cov.executed.iter().any(|n| n == "never"),
            "never-initialized system must not be classified executed"
        );
    }

    /// Bursty system running mid-run stays executed across spans.
    #[test]
    fn bursty_system_across_spans() {
        for span in [10u32, 100, 1_000, 10_000] {
            let tick_before = 500u32;
            let tick_after = tick_before.wrapping_add(span);
            let before = HashMap::from([("every_n".to_string(), 5u32)]);
            let last_run = tick_before + span / 2;
            let age_after = tick_after.wrapping_sub(last_run);
            let after = HashMap::from([("every_n".to_string(), age_after)]);

            let cov = coverage_from(&before, &after, Some((tick_before, tick_after)));
            assert!(
                cov.executed.iter().any(|n| n == "every_n"),
                "span {span}: bursty system must be classified executed"
            );
        }
    }
}
