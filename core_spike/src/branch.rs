//! Branch outcome types — the bevy-free data half of branch.rs.
//! (The branch-matrix RUNNER, which builds a bevy App per variant,
//! stays behind.)

use crate::core_state::PlaytestReport;

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

    /// cj8.14 (E2): ended in a Fail status (JUnit `<failure>`).
    pub fn is_fail(&self) -> bool {
        self.report.status == crate::enums::PlaytestStatus::Fail
    }

    /// cj8.14 (E2): ended in a Crash status (JUnit `<error>`).
    pub fn is_crash(&self) -> bool {
        self.report.status == crate::enums::PlaytestStatus::Crash
    }

    /// cj8.14 (E2): testcase time — wall time when measured, else
    /// simulated duration.
    pub fn report_time(&self) -> f64 {
        if self.report.wall_time_s > 0.0 {
            self.report.wall_time_s
        } else {
            self.report.simulated_duration_s
        }
    }
}

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
