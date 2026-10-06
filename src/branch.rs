//! Branch testing: run variant scenarios, compare reports.

use bevy::app::App;
use crate::driver::{run_scenario, PlaytestReport};
use crate::scenario::{Scenario, ScenarioError};

// ---------------------------------------------------------------------------
// Branch testing — fork the World, run variant scenarios, compare reports.
// Bevy-exclusive: cloning a World is cheap. Enables non-destructive
// state-space exploration.
// ---------------------------------------------------------------------------

/// Outcome of a single variant in a branch matrix: how many ticks ran
/// before the status was decided, and the report (if the run reached
/// completion or crashed; early-stopped branches carry a partial report).
#[derive(Debug, Clone)]
pub struct BranchOutcome {
    /// Human-readable variant name (from the scenario's bot config or caller)
    pub variant_name: String,
    /// Tick count at termination
    pub ticks_run: u64,
    /// Report from this branch
    pub report: PlaytestReport,
}

impl BranchOutcome {
    /// Did this branch complete without violations or crashes?
    pub fn passed(&self) -> bool {
        self.report.status == crate::enums::PlaytestStatus::Pass
    }
}

/// Result of a branch matrix run.
#[derive(Debug, Clone)]
pub struct BranchMatrixReport {
    /// Ordered outcomes — same order as input scenarios.
    pub outcomes: Vec<BranchOutcome>,
}

impl BranchMatrixReport {
    /// Did ALL variants pass?
    pub fn all_passed(&self) -> bool {
        self.outcomes.iter().all(|o| o.passed())
    }

    /// Names of variants that did NOT pass.
    pub fn failed_variants(&self) -> Vec<&str> {
        self.outcomes
            .iter()
            .filter(|o| !o.passed())
            .map(|o| o.variant_name.as_str())
            .collect()
    }
}

/// Run a scenario matrix — each variant builds a fresh App via
/// `app_builder` (which registers the game's plugins) and runs its
/// scenario. Determinism comes from per-variant seed + resets, not
/// World snapshots.
///
/// DESIGN NOTE on World cloning: a deep `World` fork (true snapshot/
/// branch) requires every component/resource to be `Reflect`-registered
/// plus a snapshot crate (bevy_save-style). We deliberately do NOT
/// require that — each branch replays setup instead, which is
/// deterministic given identical resets + seed. This covers the common
/// matrix case with zero reflect burden on the game.
///
/// Example:
///
/// ```ignore
/// let mut variants = Vec::new();
/// for seed in [1u64, 2, 3, 4, 5] {
///     let mut scenario = base_scenario.clone();
///     scenario.bot.seed = seed;
///     variants.push((format!("chaos-seed-{seed}"), scenario));
/// }
/// let matrix = run_branch_matrix(build_headless_app, variants)?;
/// assert!(matrix.all_passed(), "failed: {:?}", matrix.failed_variants());
/// ```
pub fn run_branch_matrix(
    app_builder: impl Fn() -> App,
    variants: Vec<(String, Scenario)>,
) -> Result<BranchMatrixReport, ScenarioError> {
    let mut outcomes = Vec::with_capacity(variants.len());
    for (name, scenario) in variants {
        let mut app = app_builder();
        let report = run_scenario(&mut app, &scenario)?;
        outcomes.push(BranchOutcome {
            variant_name: name,
            ticks_run: report.frame_count,
            report,
        });
    }
    Ok(BranchMatrixReport { outcomes })
}

// ---------------------------------------------------------------------------
