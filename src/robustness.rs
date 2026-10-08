//! J1: Quantitative robustness semantics — STL-style signed margins.
//!
//! An invariant's robustness ρ is the signed distance to violation:
//! positive = satisfied by that margin, negative = violated. Run-level
//! robustness = min over frames; overall = min over invariants of the
//! NORMALIZED min (divided by a per-invariant scale, default
//! max(|threshold|, 1.0), optional "scale" field), so large-unit
//! invariants don't dominate.
//!
//! The boolean verdict must be exactly ρ < 0 (le/ge) or ρ ≤ 0 (strict
//! ops) — enforced by debug_assert! in the oracle so semantics can't
//! drift.

use bevy::prelude::*;
use std::collections::BTreeMap;

/// Per-invariant running robustness: (min ρ over run, frame of min,
/// normalized min). Serialized into PlaytestReport.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize)]
pub struct RobustnessSummary {
    /// invariant name -> (min robustness over run, frame where min
    /// occurred, normalized min).
    pub per_invariant: BTreeMap<String, (f64, u64, f64)>,
    /// min over invariants of normalized min: < 0 iff the run failed
    /// some invariant. None when no invariants tracked.
    pub overall: Option<f64>,
}

/// Resource accumulating robustness per frame per invariant.
#[derive(Resource, Clone, Debug, Default)]
pub struct RobustnessTracker {
    /// name -> (min ρ, frame of min, scale)
    mins: BTreeMap<String, (f64, u64, f64)>,
}

impl RobustnessTracker {
    /// Record this frame's robustness ρ for an invariant.
    /// `scale` defaults to max(|threshold|, 1.0) — callers pass the
    /// invariant-specific normalization.
    pub fn observe(&mut self, name: &str, frame: u64, rho: f64, scale: f64) {
        let scale = if scale.is_finite() && scale > 0.0 {
            scale
        } else {
            1.0
        };
        let e = self
            .mins
            .entry(name.to_string())
            .or_insert((rho, frame, scale));
        if rho < e.0 {
            e.0 = rho;
            e.1 = frame;
        }
        e.2 = scale;
    }

    /// Freeze the summary (call at report time).
    pub fn summary(&self) -> RobustnessSummary {
        let per_invariant = self
            .mins
            .iter()
            .map(|(k, (rho, frame, scale))| (k.clone(), (*rho, *frame, rho / *scale)))
            .collect();
        let overall =
            self.mins
                .values()
                .map(|(rho, _, scale)| rho / *scale)
                .fold(None::<f64>, |acc, v| {
                    Some(match acc {
                        Some(a) if a < v => a,
                        _ => v,
                    })
                });
        RobustnessSummary {
            per_invariant,
            overall,
        }
    }
}

/// Signed margin for a threshold check. le/lt: thr - v; ge/gt: v - thr;
/// equals: -(distance) (0 when equal); ne: +distance.
pub fn threshold_margin(check: crate::enums::CheckOp, value: f64, thr: f64) -> f64 {
    use crate::enums::CheckOp::*;
    match check {
        Le | Lt => thr - value,
        Ge | Gt => value - thr,
        Equals => {
            let d = (value - thr).abs();
            if d == 0.0 {
                0.0
            } else {
                -d
            }
        }
        Ne => {
            let d = (value - thr).abs();
            if d == 0.0 {
                -1.0
            } else {
                d
            }
        }
    }
}

/// Default normalization scale for a threshold: max(|thr|, 1.0).
pub fn default_scale(thr: f64) -> f64 {
    thr.abs().max(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::enums::CheckOp::*;

    #[test]
    fn boolean_verdict_matches_sign() {
        // Table test: verdict ⇔ sign of ρ, across seeded random values.
        let mut rng = 0x1234_5678_9abc_def0u64;
        let mut next = || {
            rng ^= rng >> 12;
            rng ^= rng << 25;
            rng ^= rng >> 27;
            rng.wrapping_mul(0x2545F4914F6CDD1D)
        };
        for _ in 0..500 {
            let value = (next() % 2001) as f64 / 100.0 - 10.0; // [-10, 10]
            let thr = (next() % 2001) as f64 / 100.0 - 10.0;
            for check in [Le, Lt, Ge, Gt, Equals, Ne] {
                let rho = threshold_margin(check, value, thr);
                let holds = check.holds(value, thr);
                let strict = matches!(check, Lt | Gt | Ne);
                // ρ < 0 → violated; ρ == 0 → strict ops violated.
                let rho_violated = rho < 0.0 || (strict && rho == 0.0);
                assert_eq!(
                    holds, !rho_violated,
                    "{check:?} v={value} thr={thr} rho={rho}: boolean {holds} vs sign {}",
                    !rho_violated
                );
            }
        }
    }

    #[test]
    fn eventually_robustness_is_best_so_far() {
        // Max over t of inner ρ — the tracker keeps min over run for
        // the REPORT; eventually-mode callers track best-so-far by
        // negating (they feed -best). Here: verify the min-tracking
        // primitive that backs it.
        let mut tr = RobustnessTracker::default();
        tr.observe("e", 1, 5.0, 1.0);
        tr.observe("e", 2, 2.0, 1.0);
        tr.observe("e", 3, 3.0, 1.0); // recovering — min stays 2.0
        let s = tr.summary();
        assert_eq!(s.per_invariant["e"], (2.0, 2, 2.0));
    }

    #[test]
    fn normalized_min_not_dominated_by_units() {
        // inv A: ρ=-900 scale 1000 → -0.9; inv B: ρ=-0.05 scale 1 → -0.05.
        // Overall min is -0.9 (A dominates in raw units too, but B
        // would win with raw min). With normalization both contribute
        // comparably.
        let mut tr = RobustnessTracker::default();
        tr.observe("px_bound", 1, -900.0, 1000.0);
        tr.observe("hp_bound", 1, -0.05, 1.0);
        let s = tr.summary();
        assert!((s.overall.unwrap() - (-0.9)).abs() < 1e-12);
        assert_eq!(s.per_invariant["px_bound"].2, -0.9);
        assert_eq!(s.per_invariant["hp_bound"].2, -0.05);
    }

    #[test]
    fn overall_negative_iff_fail() {
        // Trivially: overall < 0 ⟺ some normalized min < 0 ⟺ some ρ < 0.
        let mut tr = RobustnessTracker::default();
        tr.observe("a", 1, 0.5, 1.0);
        tr.observe("b", 1, 0.2, 1.0);
        assert!(tr.summary().overall.unwrap() > 0.0);
        tr.observe("c", 5, -0.1, 1.0);
        assert!(tr.summary().overall.unwrap() < 0.0);
    }
}
