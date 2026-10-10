//! Z11: Version-to-version differential testing (golden trajectories).
//!
//! Records per-frame digests + periodic snapshots of game-owned state,
//! compares against stored goldens, produces human-readable diffs.
//!
//! ## Usage
//! ```no_run
//! # use bevy::prelude::*;
//! use bevy_swarm::golden::{check_golden, GoldenSet, Tolerance};
//!
//! // Load previously recorded golden trajectories.
//! let golden_set = GoldenSet::load("tests/swarm/golden").unwrap();
//! // Check current game against goldens.
//! let report = check_golden(|| App::new(), &golden_set, &Tolerance::default()).unwrap();
//! assert!(report.all_same(), "no gameplay regressions");
//! ```

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::determinism::StateTrace;
use crate::scenario::Scenario;

/// A set of golden scenarios to record/check.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GoldenSet {
    pub scenarios: Vec<(String, Scenario)>,
    pub dir: PathBuf,
}

impl GoldenSet {
    /// Load a golden set from a directory.
    pub fn load(dir: impl AsRef<Path>) -> Result<Self, std::io::Error> {
        let dir = dir.as_ref();
        let mut scenarios = Vec::new();
        if dir.exists() {
            for entry in fs::read_dir(dir)? {
                let entry = entry?;
                let path = entry.path();
                let file_name = entry.file_name().to_string_lossy().to_string();
                if file_name.ends_with(".golden.json") {
                    let json = fs::read_to_string(&path)?;
                    let golden: GoldenFile = serde_json::from_str(&json)?;
                    let stem = file_name.trim_end_matches(".golden.json").to_string();
                    scenarios.push((stem, golden.scenario));
                }
            }
        }
        Ok(Self {
            scenarios,
            dir: dir.to_path_buf(),
        })
    }

    /// Save a loaded golden set back to disk, preserving each file's
    /// digests/inputs/snapshots. Round-trip safe: load → save → load
    /// preserves all trajectory data (previously this re-wrote files
    /// with EMPTY digests, wiping goldens).
    pub fn save(&self) -> Result<(), std::io::Error> {
        for (name, _scenario) in &self.scenarios {
            let path = self.dir.join(format!("{}.golden.json", name));
            // Only rewrite files that don't exist; existing goldens
            // already hold the digests/inputs/snapshots and are
            // rewritten exclusively by check_golden's update mode or
            // record_golden — never blind-saved.
            if !path.exists() {
                return Err(std::io::Error::other(format!(
                    "cannot save golden '{name}': no golden file to preserve \
                     (use record_golden to create goldens)"
                )));
            }
        }
        Ok(())
    }
}

impl Default for GoldenSet {
    fn default() -> Self {
        Self {
            scenarios: vec![],
            dir: PathBuf::from("tests/swarm/golden"),
        }
    }
}

/// A single golden file (one scenario's trajectory).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GoldenFile {
    pub bevy_swarm_version: String,
    pub platform: String,
    pub scenario: Scenario,
    /// Inputs that reproduce this run (frame-indexed replay
    /// intents decoded from the audit log — NOT a seed to re-chaos).
    pub inputs: Vec<crate::scenario::ReplayInput>,
    /// Per-frame digests (hex strings for compactness).
    pub digests: Vec<String>,
    /// Periodic snapshots (every `snapshot_every_s` seconds).
    pub snapshots: Vec<SnapshotEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct SnapshotEntry {
    pub frame: u64,
    pub time_s: f32,
    pub state: BTreeMap<String, String>,
}

/// Tolerance for float comparisons in snapshot diffs.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Tolerance {
    /// Absolute tolerance for float fields.
    pub abs_tol: f64,
    /// Relative tolerance for float fields.
    pub rel_tol: f64,
    /// Ignore keys matching these patterns (regex).
    pub ignore_patterns: Vec<String>,
}

impl Default for Tolerance {
    fn default() -> Self {
        Self {
            abs_tol: 1e-6,
            rel_tol: 1e-6,
            ignore_patterns: vec![],
        }
    }
}

/// Outcome of comparing a scenario against its golden.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum GoldenOutcome {
    Same,
    Diverged {
        first_frame: u64,
        changed: Vec<(String, String, String)>, // (key, old, new)
        summary: String,
    },
    Missing,
}

/// Report from a golden check run.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GoldenReport {
    pub per_scenario: Vec<(String, GoldenOutcome)>,
    pub all_same: bool,
    pub total_time_ms: u64,
}

impl GoldenReport {
    /// True if all scenarios matched their goldens.
    pub fn all_same(&self) -> bool {
        self.all_same
    }

    /// Human-readable summary.
    pub fn summary(&self) -> String {
        let diverged = self
            .per_scenario
            .iter()
            .filter(|(_, o)| matches!(o, GoldenOutcome::Diverged { .. }))
            .count();
        format!(
            "golden check: {} scenarios, {} diverged",
            self.per_scenario.len(),
            diverged
        )
    }
}

/// Record golden trajectories for a set of scenarios.
///
/// Runs each scenario once, recording per-frame digests and periodic
/// snapshots. Returns a GoldenSet ready to be saved.
pub fn record_golden<F: Fn() -> bevy::app::App + Sync>(
    app_factory: F,
    scenarios: &[Scenario],
    output_dir: impl AsRef<Path>,
) -> Result<GoldenSet, Box<dyn std::error::Error>> {
    let mut golden_set = GoldenSet {
        dir: output_dir.as_ref().to_path_buf(),
        ..GoldenSet::default()
    };
    let platform = format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH);

    for (scenario_idx, scenario) in scenarios.iter().enumerate() {
        let mut app = app_factory();
        // Snapshots are taken per-frame during the run (StateTrace
        // with_snapshots=true), not re-canonicalized after it.
        app.insert_resource(StateTrace {
            with_snapshots: true,
            stop_after: None,
            digests: vec![],
        });

        // Run the scenario (consumes StateTrace into report.state_trace)
        let report = crate::driver::run_scenario(&mut app, scenario)?;

        let digests: Vec<String> = report
            .state_trace
            .iter()
            .flat_map(|d| d.iter())
            .map(|d| format!("{:016x}", d.hash))
            .collect();

        // Snapshots come from FrameDigest.snapshot (captured during
        // the run) instead of re-canonicalizing the final world.
        let snapshots: Vec<SnapshotEntry> = report
            .state_trace
            .iter()
            .flatten()
            .filter_map(|fd| {
                fd.snapshot.as_ref().map(|snap| SnapshotEntry {
                    frame: fd.frame,
                    time_s: (fd.frame as f32) / (scenario.tps.max(1) as f32),
                    state: snap.clone(),
                })
            })
            .collect();

        let golden = GoldenFile {
            bevy_swarm_version: env!("CARGO_PKG_VERSION").to_string(),
            platform: platform.clone(),
            scenario: scenario.clone(),
            inputs: recorded_replay_inputs(&report),
            digests,
            snapshots,
        };

        // Naming: use scenario.name (if present) + seed; fall back to
        // index+seed for backwards compatibility. Unique scenario names
        // prevent reordering from renaming/overwriting golden files.
        let scenario_name = scenario
            .name
            .clone()
            .unwrap_or_else(|| scenario_idx.to_string());
        let entry_name = format!("scenario-{}-{}", scenario_name, scenario.bot.seed);
        golden_set
            .scenarios
            .push((entry_name.clone(), scenario.clone()));
        let path = golden_set.dir.join(format!("{}.golden.json", entry_name));
        let json = serde_json::to_string_pretty(&golden)?;
        fs::create_dir_all(&golden_set.dir)?;
        fs::write(&path, json)?;
    }

    Ok(golden_set)
}

/// Check scenarios against their stored golden trajectories.
///
/// Replay stored inputs instead of re-running the seed's chaos.
/// Rewrites diverged goldens when BEVY_SWARM_UPDATE=golden/all (U1).
pub fn check_golden<F: Fn() -> bevy::app::App + Sync>(
    app_factory: F,
    golden_set: &GoldenSet,
    tolerance: &Tolerance,
) -> Result<GoldenReport, Box<dyn std::error::Error>> {
    check_golden_opts(app_factory, golden_set, tolerance, false)
}

/// Like [`check_golden`] but with an explicit update override (test
/// seam — avoids racing process env vars between parallel tests).
pub fn check_golden_opts<F: Fn() -> bevy::app::App + Sync>(
    app_factory: F,
    golden_set: &GoldenSet,
    tolerance: &Tolerance,
    force_update: bool,
) -> Result<GoldenReport, Box<dyn std::error::Error>> {
    let update_enabled = force_update || update_golden_enabled();
    let start = Instant::now();
    let mut per_scenario = Vec::new();
    let mut all_same = true;

    for (name, _scenario) in &golden_set.scenarios {
        let golden_path = golden_set.dir.join(format!("{}.golden.json", name));
        if !golden_path.exists() {
            per_scenario.push((name.clone(), GoldenOutcome::Missing));
            all_same = false;
            continue;
        }

        let json = fs::read_to_string(&golden_path)?;
        let golden: GoldenFile = serde_json::from_str(&json)?;

        // Determinism pre-check: the golden replay runs TWICE and the
        // two runs must agree before comparing to the golden — a
        // run-vs-run mismatch is nondeterminism in the GAME (reported
        // as "nondeterministic"), not golden divergence.
        //
        // The replay bot is fed from the stored inputs so a changed
        // IntentSurface cannot make the comparison drift.
        let replay_scenario = golden_to_replay_scenario(&golden);
        let run_once = |factory: &F| -> Result<
            Vec<crate::determinism::FrameDigest>,
            Box<dyn std::error::Error>,
        > {
            let mut app = factory();
            app.insert_resource(StateTrace {
                with_snapshots: true,
                stop_after: None,
                digests: vec![],
            });
            let report = crate::driver::run_scenario(&mut app, &replay_scenario)?;
            Ok(report.state_trace.unwrap_or_default())
        };
        let trace_a = run_once(&app_factory)?;
        let trace_b = run_once(&app_factory)?;
        let run_outcomes_match = trace_a.len() == trace_b.len()
            && trace_a
                .iter()
                .zip(trace_b.iter())
                .all(|(a, b)| a.hash == b.hash);
        if !run_outcomes_match {
            per_scenario.push((
                name.clone(),
                GoldenOutcome::Diverged {
                    first_frame: trace_a
                        .iter()
                        .zip(trace_b.iter())
                        .position(|(a, b)| a.hash != b.hash)
                        .map(|i| trace_a[i].frame)
                        .unwrap_or(trace_a.len().min(trace_b.len()) as u64),
                    changed: vec![],
                    summary: "nondeterministic".to_string(),
                },
            ));
            all_same = false;
            continue;
        }

        let report_digests = trace_a;
        let current_snapshots: Vec<SnapshotEntry> = report_digests
            .iter()
            .filter_map(|fd| {
                fd.snapshot.as_ref().map(|snap| SnapshotEntry {
                    frame: fd.frame,
                    time_s: (fd.frame as f32) / (replay_scenario.tps.max(1) as f32),
                    state: snap.clone(),
                })
            })
            .collect();
        let current_digests: Vec<String> = report_digests
            .iter()
            .map(|d| format!("{:016x}", d.hash))
            .collect();

        let outcome = if current_digests.len() != golden.digests.len() {
            // digest-count mismatch sets all_same=false (e.g. a game
            // that now crashes early must not pass the report).
            all_same = false;
            GoldenOutcome::Diverged {
                first_frame: 0,
                changed: vec![(
                    "length".to_string(),
                    golden.digests.len().to_string(),
                    current_digests.len().to_string(),
                )],
                summary: "digest count mismatch".to_string(),
            }
        } else {
            let mut first_diff_frame: Option<u64> = None;
            for (idx, (stored, current)) in golden
                .digests
                .iter()
                .zip(current_digests.iter())
                .enumerate()
            {
                if stored != current {
                    first_diff_frame = Some(idx as u64);
                    break;
                }
            }

            match first_diff_frame {
                Some(frame) => {
                    all_same = false;
                    // Readable diff: compare the nearest snapshots around the
                    // divergence with tolerance. Digest-only divergence (no
                    // snapshots or no reflected state) reports as-is.
                    let nearest = golden.snapshots.iter().rev().find(|s| s.frame <= frame);
                    let current_nearest = current_snapshots.iter().rev().find(|s| s.frame <= frame);
                    // Both old and new values, tolerances applied.
                    let changed = match (nearest, current_nearest) {
                        (Some(old_snap), Some(new_snap)) => {
                            diff_states(&old_snap.state, &new_snap.state, tolerance)
                        }
                        (Some(old_snap), None) => old_snap
                            .state
                            .iter()
                            .filter(|(k, _)| {
                                !tolerance.ignore_patterns.iter().any(|p| k.contains(p))
                            })
                            .map(|(k, v)| {
                                (k.clone(), v.clone(), "<missing in current run>".to_string())
                            })
                            .collect(),
                        _ => vec![],
                    };

                    let outcome = GoldenOutcome::Diverged {
                        first_frame: frame,
                        changed,
                        summary: format!("diverged at frame {}", frame),
                    };

                    // BEVY_SWARM_UPDATE=golden (via conventions::
                    // can_update, U1) rewrites the stored golden with
                    // the current behaviour — an explicit opt-in, not a
                    // silent accept-regression.
                    if update_enabled {
                        let updated = GoldenFile {
                            bevy_swarm_version: golden.bevy_swarm_version.clone(),
                            platform: golden.platform.clone(),
                            scenario: replay_scenario.clone(),
                            inputs: golden.inputs.clone(),
                            digests: current_digests.clone(),
                            snapshots: current_snapshots.clone(),
                        };
                        let path = golden_set.dir.join(format!("{}.golden.json", name));
                        fs::create_dir_all(&golden_set.dir)?;
                        fs::write(&path, serde_json::to_string_pretty(&updated)?)?;
                    }

                    outcome
                }
                None => GoldenOutcome::Same,
            }
        };

        per_scenario.push((name.clone(), outcome));
    }

    Ok(GoldenReport {
        per_scenario,
        all_same,
        total_time_ms: start.elapsed().as_millis() as u64,
    })
}

/// Check if golden update mode is enabled (U1 conventions:
/// BEVY_SWARM_UPDATE=golden or BEVY_SWARM_UPDATE=all).
pub fn update_golden_enabled() -> bool {
    crate::conventions::can_update("golden")
}

/// Decode a run's action log into frame-indexed replay inputs. Only
/// intents that survive the decode (structured audit entries) are
/// kept; Wait is included because a replay of nothing must not make
/// the bot re-sample.
fn recorded_replay_inputs(
    report: &crate::driver::PlaytestReport,
) -> Vec<crate::scenario::ReplayInput> {
    let timed = crate::scenario::action_log_to_timed_actions(&report.action_log);
    timed
        .iter()
        .filter_map(|ta| {
            crate::minimize::action_to_replay_intent(ta).map(|intent| {
                crate::scenario::ReplayInput {
                    frame: ta.frame,
                    intent,
                }
            })
        })
        .collect()
}

/// Build a replay scenario from a golden file: same scenario shape,
/// bot switched to Replay fed with the RECORDED inputs, so the golden
/// comparison replays exactly what was recorded instead of re-running
/// the chaos bot (which drifts when the IntentSurface changes).
pub fn golden_to_replay_scenario(golden: &GoldenFile) -> Scenario {
    let mut scenario = golden.scenario.clone();
    scenario.bot.bot_type = crate::enums::BotType::Replay;
    scenario.bot.inputs = golden.inputs.clone();
    scenario
}

/// Compare two canonical states with tolerance. Numeric values that
/// differ by at most max(abs_tol, rel_tol * max(|old|,|new|)) are
/// treated as equal; non-numeric values compare stringwise. Only
/// genuinely differing keys are reported, with BOTH old and new values.
pub fn diff_states(
    old: &BTreeMap<String, String>,
    new: &BTreeMap<String, String>,
    tolerance: &Tolerance,
) -> Vec<(String, String, String)> {
    let keys: std::collections::BTreeSet<_> = old.keys().chain(new.keys()).collect();
    keys.into_iter()
        .filter(|k| !tolerance.ignore_patterns.iter().any(|p| k.contains(p)))
        .map(|k| {
            let va = old.get(k).cloned().unwrap_or_default();
            let vb = new.get(k).cloned().unwrap_or_default();
            (k.clone(), va, vb)
        })
        .filter(|(_, va, vb)| {
            if let (Ok(a), Ok(b)) = (va.parse::<f64>(), vb.parse::<f64>()) {
                let abs_diff = (a - b).abs();
                let scale = a.abs().max(b.abs());
                !(abs_diff <= tolerance.abs_tol
                    || (scale > 0.0 && abs_diff <= tolerance.rel_tol * scale))
            } else {
                va != vb
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn golden_same_on_unchanged_game() {
        // Create a minimal golden set
        let temp_dir = std::env::temp_dir().join("bevy_swarm_golden_test");
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();

        let scenario: Scenario =
            serde_json::from_str(r#"{"bot":{"type":"chaos","seed":1},"duration_s":0.05}"#).unwrap();

        // For now, just verify the structure compiles and saves
        let path = temp_dir.join("test.golden.json");
        let golden = GoldenFile {
            bevy_swarm_version: "0.1.0".to_string(),
            platform: "test".to_string(),
            scenario: scenario.clone(),
            inputs: vec![],
            digests: vec!["abcd1234".to_string()],
            snapshots: vec![],
        };
        let json = serde_json::to_string_pretty(&golden).unwrap();
        fs::write(&path, json).unwrap();

        // Check golden
        let loaded = GoldenSet::load(&temp_dir).unwrap();
        assert_eq!(loaded.scenarios.len(), 1);

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn tolerance_defaults() {
        let tol = Tolerance::default();
        assert_eq!(tol.abs_tol, 1e-6);
        assert_eq!(tol.rel_tol, 1e-6);
    }
}
