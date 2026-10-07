//! Branch testing: run variant scenarios, compare reports.

use crate::driver::{run_scenario, PlaytestReport};
use crate::scenario::{Scenario, ScenarioError};
use bevy::app::App;

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
/// Trait for running a scenario against a game — plug in different
/// execution strategies (in-process today; Z8's subprocess later).
pub trait ScenarioRunner: Sync {
    /// Run a single scenario, returning a full report or a structured error.
    fn run(&self, scenario: &Scenario) -> Result<PlaytestReport, ScenarioError>;
}

/// In-process runner: builds a FRESH App per run via the factory
/// closure, then applies the standard harness pipeline (A1/A3
/// settings). Provides the hook points Z14 needs (enter_run BEFORE the
/// factory call, exit_run after — both on the runner, not the thread).
pub struct InProcess<F> {
    pub factory: F,
}

impl<F: Fn() -> App + Sync> ScenarioRunner for InProcess<F> {
    fn run(&self, scenario: &Scenario) -> Result<PlaytestReport, ScenarioError> {
        let mut app = (self.factory)();
        run_scenario(&mut app, scenario)
    }
}

/// Run a matrix of scenarios in PARALLEL — one freshly-spawned scoped
/// thread per scenario (rand's thread RNG is per-thread, and Z14
/// requires a fresh thread per scenario for determinism), capped by
/// `max_parallel` (semaphore-like: join the oldest handle at the cap).
/// Panic isolation relies on run_scenario's catch_unwind (covers the
/// readiness gate after C2). Output order matches input order.
pub fn run_matrix(
    runner: &dyn ScenarioRunner,
    variants: Vec<(String, Scenario)>,
    max_parallel: usize,
) -> Result<BranchMatrixReport, ScenarioError> {
    use std::sync::{Arc, Mutex};

    let results = Arc::new(Mutex::new(Vec::with_capacity(variants.len())));
    let runner = Arc::new(runner);
    let max_parallel = max_parallel.max(1);

    std::thread::scope(|s| {
        let mut handles = Vec::new();
        for (idx, (name, scenario)) in variants.into_iter().enumerate() {
            let runner = Arc::clone(&runner);
            let results = Arc::clone(&results);
            // Fresh thread per scenario (never a pooled worker).
            let handle = s.spawn(move || {
                let res = runner.run(&scenario);
                let outcome = match res {
                    Ok(report) => BranchOutcome {
                        variant_name: name,
                        ticks_run: report.frame_count,
                        report,
                    },
                    // A structured scenario error (rejection etc.) is
                    // reported as a Crash-status outcome, not a torn-down
                    // whole matrix.
                    Err(_) => BranchOutcome {
                        variant_name: name,
                        ticks_run: 0,
                        report: PlaytestReport::crashed_empty(),
                    },
                };
                results.lock().unwrap().push((idx, outcome));
            });
            handles.push(handle);
            if handles.len() >= max_parallel {
                handles.remove(0).join().unwrap();
            }
        }
        for h in handles {
            h.join().unwrap();
        }
    });

    let mut ordered: Vec<(usize, BranchOutcome)> = results.lock().unwrap().drain(..).collect();
    ordered.sort_by_key(|(idx, _)| *idx);
    Ok(BranchMatrixReport {
        outcomes: ordered.into_iter().map(|(_, o)| o).collect(),
    })
}

/// Run a scenario matrix SEQUENTIALLY (v0.2 behavior) — equivalent to
/// `run_matrix(&InProcess { factory: app_builder }, variants, 1)`.
pub fn run_branch_matrix(
    app_builder: impl Fn() -> App + Sync,
    variants: Vec<(String, Scenario)>,
) -> Result<BranchMatrixReport, ScenarioError> {
    run_matrix(
        &InProcess {
            factory: app_builder,
        },
        variants,
        1,
    )
}

// ---------------------------------------------------------------------------
