//! Diagnostics data half — report-formatting types from
//! diagnostics.rs. The World-probing / App-driving half
//! (contract_diagnostics, calibrate_world, detect_*) stays behind.

/// Report of contract-tier detection for a built App. Produced by
/// `contract_diagnostics`; consumed by the CLI and human users.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ContractReport {
    /// Reset hooks registered (empty => scenarios can't reset).
    pub reset_hooks: Vec<String>,
    /// Cheat kinds registered.
    pub cheat_kinds: Vec<String>,
    /// Named intents registered.
    pub named_intents: Vec<String>,
    /// Intent surface size (0 => chaos bot samples nothing).
    pub intent_surface_size: usize,
    /// Whether a TestApi-like resolve succeeds at all.
    pub test_api_present: bool,
    /// Recommendations based on gaps.
    pub recommendations: Vec<String>,
}

/// Calibration snapshot: baseline measurements from a short chaos run.
#[derive(Clone, Debug, Default)]
pub struct CalibrationSnapshot {
    /// Archetype counts: (component_signatures_as_sorted_pipe_delimited, min_count, max_count)
    pub archetype_envelopes: Vec<(String, usize, usize)>,
    /// Resources observed: (short_type_name, last_numeric_value)
    pub resource_baselines: Vec<(String, f64)>,
}

/// Options for calibration runs.
#[derive(Debug, Clone)]
pub struct CalibrationOptions {
    pub duration_s: f32,
    pub tps: u32,
    pub seed: u64,
    pub simulated_time: bool,
}

impl Default for CalibrationOptions {
    fn default() -> Self {
        Self {
            duration_s: 2.0,
            tps: 60,
            seed: 42,
            simulated_time: true,
        }
    }
}

/// Generate candidate invariants from a calibration snapshot.
/// Returns JSON-formatted invariants ready to drop into a scenario.
pub fn generate_invariants_from_calibration(snapshot: &CalibrationSnapshot) -> String {
    use serde_json::json;

    let mut invariants = Vec::new();

    // Archetype envelope invariants. Skip resource archetypes
    // (IsResource markers — covered by resource baselines instead) and
    // use SHORT component names to match query-resolution semantics.
    for (sig, min, _max) in &snapshot.archetype_envelopes {
        if sig.contains("IsResource") {
            continue;
        }
        if *min > 0 {
            // Query with GAME-OWNED components only.
            let comps: Vec<String> = sig
                .split('|')
                .filter(|c| !c.starts_with("bevy_"))
                .map(|s| short_type_name(s).to_string())
                .collect();
            if comps.is_empty() {
                continue;
            }
            invariants.push(json!({
                "name": format!("archetype_min_{}", sanitize_name(sig)),
                "rule": "custom",
                "query": {
                    "with": comps,
                    "without": []
                },
                "check": "ge",
                "value": *min as f64,
                "eventually_s": null
            }));
        }
    }

    // Resource baseline hints
    for (name, val) in &snapshot.resource_baselines {
        if *val >= 0.0 {
            invariants.push(json!({
                "name": format!("{}_non_negative", sanitize_name(name)),
                "rule": "custom",
                "path": format!("{}.score", name),
                "check": "ge",
                "value": -0.5,
                "eventually_s": null
            }));
        }
    }

    serde_json::to_string_pretty(&invariants).unwrap()
}

fn short_type_name(type_path: &str) -> &str {
    type_path.rsplit("::").next().unwrap_or(type_path)
}

fn sanitize_name(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect()
}
