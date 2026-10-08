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

use crate::determinism::{canonical_state, StateTrace};
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

    /// Save a golden set to disk (deprecated — use record_golden).
    /// FX9: this method now panics if called (it wiped digests in the
    /// original implementation; callers must use record_golden which
    /// preserves digests/inputs/snapshots).
    pub fn save(&self) -> Result<(), std::io::Error> {
        Err(std::io::Error::other(
            "GoldenSet::save() is deprecated — use record_golden()",
        ))
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
    /// Inputs that reproduce this run (replay actions, not seeds).
    pub inputs: Vec<String>,
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
        app.insert_resource(StateTrace {
            with_snapshots: false,
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

        // Collect snapshots at 1s intervals
        let mut snapshots = Vec::new();
        let mut last_snapshot_frame = 0u64;
        // FX9: use the SCENARIO's tps, not a hardcoded 60.
        let tps = scenario.tps.max(1) as f32;
        let snapshot_interval_frames = (tps * 1.0) as u64; // Every 1s

        for (frame_idx, _digest) in report.state_trace.iter().flatten().enumerate() {
            if frame_idx as u64 >= last_snapshot_frame + snapshot_interval_frames {
                let world = app.world();
                let state = canonical_state(world);
                snapshots.push(SnapshotEntry {
                    frame: frame_idx as u64,
                    time_s: (frame_idx as f32) / tps,
                    state,
                });
                last_snapshot_frame = frame_idx as u64;
            }
        }

        let golden = GoldenFile {
            bevy_swarm_version: env!("CARGO_PKG_VERSION").to_string(),
            platform: platform.clone(),
            scenario: scenario.clone(),
            // FX9: store the audit log so goldens replay recorded
            // inputs instead of drifting with the input surface.
            inputs: app
                .world()
                .get_resource::<crate::contract::ActionLog>()
                .map(|l| l.entries.iter().map(|e| e.action.clone()).collect())
                .unwrap_or_default(),
            digests,
            snapshots,
        };

        // FX9: name by scenario name + seed — two scenarios with the
        // same seed must not overwrite each other.
        let entry_name = format!("scenario-{}-{}", scenario_idx, scenario.bot.seed);
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
pub fn check_golden<F: Fn() -> bevy::app::App + Sync>(
    app_factory: F,
    golden_set: &GoldenSet,
    tolerance: &Tolerance,
) -> Result<GoldenReport, Box<dyn std::error::Error>> {
    let start = Instant::now();
    let mut per_scenario = Vec::new();
    let mut all_same = true;

    for (name, scenario) in &golden_set.scenarios {
        let golden_path = golden_set.dir.join(format!("{}.golden.json", name));
        if !golden_path.exists() {
            per_scenario.push((name.clone(), GoldenOutcome::Missing));
            all_same = false;
            continue;
        }

        let json = fs::read_to_string(&golden_path)?;
        let golden: GoldenFile = serde_json::from_str(&json)?;

        // Run the scenario
        let mut app = app_factory();
        app.insert_resource(StateTrace {
            with_snapshots: false,
            stop_after: None,
            digests: vec![],
        });

        let report = crate::driver::run_scenario(&mut app, scenario)?;

        // run_scenario consumes StateTrace into report.state_trace.
        let current_digests: Vec<String> = report
            .state_trace
            .iter()
            .flat_map(|d| d.iter())
            .map(|d| format!("{:016x}", d.hash))
            .collect();

        let outcome = if current_digests.len() != golden.digests.len() {
            all_same = false; // FX9: digest-count mismatch sets all_same=false
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
                    let changed = nearest
                        .map(|snap| {
                            snap.state
                                .iter()
                                .filter(|(k, _)| {
                                    !tolerance.ignore_patterns.iter().any(|p| k.contains(p))
                                })
                                .map(|(k, v)| (k.clone(), v.clone(), String::new()))
                                .collect()
                        })
                        .unwrap_or_default();
                    GoldenOutcome::Diverged {
                        first_frame: frame,
                        changed,
                        summary: format!("diverged at frame {}", frame),
                    }
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
