//! Contract diagnostics, calibration, and feature detection.

use bevy::app::App;
use bevy::ecs::world::World;
use bevy::prelude::*;

use crate::contract::{IntentSurface, ResetHooks, TestApi, TestApiResolver, HARNESS_TYPE_PATHS};
use crate::driver::ScenarioResource;
use crate::scenario::{Scenario, ScenarioError};
use crate::state::{PlaytestState, Violations};

// ---------------------------------------------------------------------------
// Contract diagnostics — detect contract pieces, warn on gaps
// ---------------------------------------------------------------------------

/// Report of contract-tier detection for a built App. Produced by
/// [`contract_diagnostics`]; consumed by the CLI and human users.
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
    /// Detected gameplay features via field vocabulary.
    pub detected_features: Vec<DetectedFeature>,
}

/// Scan a built App for contract coverage. Pure inspection — no
/// mutation, safe to call on any App.
pub fn contract_diagnostics(world: &World) -> ContractReport {
    let mut recommendations = Vec::new();

    let reset_hooks: Vec<String> = world
        .get_resource::<ResetHooks>()
        .map(|h| h.0.keys().cloned().collect())
        .unwrap_or_default();
    if reset_hooks.is_empty() {
        recommendations.push(
            "no reset hooks — add register ResetHooks explicitly or register ResetHooks".into(),
        );
    }

    let cheat_kinds: Vec<String> = world
        .get_resource::<crate::contract::CheatHooks>()
        .map(|h| h.0.keys().cloned().collect())
        .unwrap_or_default();
    if cheat_kinds.is_empty() {
        recommendations.push(
            "no cheats registered — CheatHooks unlock scenario setup.cheats for state shaping"
                .into(),
        );
    }

    let named_intents: Vec<String> = world
        .get_resource::<crate::contract::NamedIntents>()
        .map(|n| n.0.keys().cloned().collect())
        .unwrap_or_default();
    if named_intents.is_empty() {
        recommendations.push(
            "no named intents — NamedIntents let agents drive gameplay semantically (e.g. end_turn)"
                .into(),
        );
    }

    let intent_surface_size = world
        .get_resource::<IntentSurface>()
        .map(|s| s.0.len())
        .unwrap_or(0);
    if intent_surface_size == 0 {
        recommendations.push(
            "empty IntentSurface — chaos/pursuit bots can't sample meaningful actions".into(),
        );
    }

    let test_api_present = world.get_resource::<TestApi>().is_some();

    // Feature detection: classify gameplay by component/resource field
    // vocabulary (zero game cooperation). Feeds persona selection and
    // invariant suggestions.
    let detected_features = detect_features(world);

    ContractReport {
        reset_hooks,
        cheat_kinds,
        named_intents,
        intent_surface_size,
        test_api_present,
        recommendations,
        detected_features,
    }
}

/// A gameplay feature detected by reflection — zero game cooperation.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub enum DetectedFeature {
    TurnBased,
    Combat,
    Inventory,
    Dialogue,
    RealTime,
}

impl DetectedFeature {
    pub fn label(&self) -> &'static str {
        match self {
            DetectedFeature::TurnBased => "turn_based",
            DetectedFeature::Combat => "combat",
            DetectedFeature::Inventory => "inventory",
            DetectedFeature::Dialogue => "dialogue",
            DetectedFeature::RealTime => "real_time",
        }
    }
}

impl std::fmt::Display for DetectedFeature {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.label())
    }
}

/// Field-vocabulary probes for feature detection: (feature, field names).
const FEATURE_VOCAB: [(&str, &[&str]); 4] = [
    (
        "turn_based",
        &["turn", "turn_number", "current_turn", "faction_turn"],
    ),
    ("combat", &["hp", "health", "damage", "attack", "defense"]),
    (
        "inventory",
        &["item", "capacity", "slot", "inventory", "stack"],
    ),
    ("dialogue", &["dialogue", "speaker", "line", "node_choice"]),
];

/// Walk reflected types, collect all field names present in the app.
fn collect_field_vocab(world: &World) -> std::collections::HashSet<String> {
    use bevy::ecs::reflect::AppTypeRegistry;
    let mut fields = std::collections::HashSet::new();
    let Some(registry) = world.get_resource::<AppTypeRegistry>() else {
        return fields;
    };
    let registry = registry.0.clone();
    let reg = registry.read();
    for ty in reg.iter() {
        let path = ty.type_info().type_path();
        // Skip harness-owned types (they'd pollute detection).
        if HARNESS_TYPE_PATHS.iter().any(|h| path.starts_with(h)) {
            continue;
        }
        if let bevy::reflect::TypeInfo::Struct(s) = ty.type_info() {
            for name in s.field_names() {
                fields.insert(name.to_string());
            }
        }
    }
    fields
}

/// Entity-motion heuristic for real_time detection: if Transform components
/// exist AND entity positions change across two consecutive reads... but we
/// only have one world here. Simpler proxy: presence of a `Time` resource
/// with advancing virtual clock plus moving transforms is undecidable at
/// rest — fall back to Transform+Velocity-style field vocab.
fn detect_features(world: &World) -> Vec<DetectedFeature> {
    let mut found = Vec::new();
    let vocab = collect_field_vocab(world);
    for (feature, names) in FEATURE_VOCAB {
        if names.iter().any(|n| vocab.contains(*n)) {
            match feature {
                "turn_based" => found.push(DetectedFeature::TurnBased),
                "combat" => found.push(DetectedFeature::Combat),
                "inventory" => found.push(DetectedFeature::Inventory),
                "dialogue" => found.push(DetectedFeature::Dialogue),
                _ => unreachable!(),
            }
        }
    }
    found
}

/// Probes the world's reflected resources and auto-publishes observable
/// paths for them. Contract shrinks to "derive Reflect + register_type"
/// (which games do anyway for saves). Games with their own TestApi keep
/// their hand-written resolution, but gain automatic observability of
/// every reflected resource field.
pub fn auto_observed_paths(world: &World) -> Vec<String> {
    use bevy::ecs::reflect::AppTypeRegistry;
    let mut paths = Vec::new();
    let Some(registry) = world.get_resource::<AppTypeRegistry>() else {
        return paths;
    };
    let registry = registry.0.clone();
    let reg = registry.read();
    'outer: for registration in reg.iter() {
        let type_path = registration.type_info().type_path();
        if HARNESS_TYPE_PATHS.iter().any(|h| type_path.starts_with(h)) {
            continue;
        }
        let Some(rc) = registration.data::<bevy::ecs::reflect::ReflectComponent>() else {
            continue;
        };
        let Some(component_id) = world.components().get_valid_id(registration.type_id()) else {
            continue;
        };
        for (cid, entity) in world.resource_entities().iter() {
            if cid != component_id {
                continue;
            }
            let Ok(entity_ref) = world.get_entity(entity) else {
                continue;
            };
            if rc.reflect(entity_ref).is_none() {
                continue;
            }
            // Enumerate struct fields from static TypeInfo — no live
            // value probing needed for path listing.
            if let bevy::reflect::TypeInfo::Struct(s) = registration.type_info() {
                let short_name = short_type_name(type_path);
                for name in s.field_names() {
                    paths.push(format!("{}.{}", short_name, name));
                }
            }
            continue 'outer;
        }
    }
    paths.sort();
    paths.dedup();
    paths
}

fn short_type_name(type_path: &str) -> &str {
    type_path.rsplit("::").next().unwrap_or(type_path)
}

// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Auto-discovery calibration — derive invariants from calibration runs
// ---------------------------------------------------------------------------

/// Calibration snapshot: baseline measurements from a short chaos run.
#[derive(Clone, Debug, Default)]
pub struct CalibrationSnapshot {
    /// Archetype counts: (component_signatures_as_sorted_pipe_delimited, min_count, max_count)
    pub archetype_envelopes: Vec<(String, usize, usize)>,
    /// Resources observed: (short_type_name, last_numeric_value)
    pub resource_baselines: Vec<(String, f64)>,
}

/// Run a calibration phase: short chaos-driven run that probes the world
/// for measurable patterns. Returns a snapshot suitable for generating
/// candidate invariants.
///
/// USAGE: Call this once per game binary, save the snapshot, then
/// programmatically generate invariants (e.g., "archetype X never drops
/// below N"). Zero game-code changes required.
/// Options for calibration runs.
#[derive(Debug, Clone)]
pub struct CalibrationOptions {
    pub duration_s: f32,
    pub tps: u32,
    pub seed: u64,
    /// Apply A1 deterministic simulated time (recommended: makes the
    /// envelope independent of host speed).
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

pub fn calibrate_world(
    app: &mut App,
    duration_s: f32,
) -> Result<CalibrationSnapshot, ScenarioError> {
    calibrate_world_opts(
        app,
        &CalibrationOptions {
            duration_s,
            ..Default::default()
        },
    )
}

pub fn calibrate_world_opts(
    app: &mut App,
    opts: &CalibrationOptions,
) -> Result<CalibrationSnapshot, ScenarioError> {
    crate::driver::finish_plugins(app);

    // Contract check: PlaytestPlugin must be added (its systems make
    // ScenarioResource observable by bots). A missing TestApi is NOT a
    // rejection — numeric percepts fall back to archetype envelopes only.
    if app
        .world()
        .get_resource::<crate::state::PlaytestState>()
        .is_none()
        && !app.is_plugin_added::<crate::harness::PlaytestPlugin>()
    {
        return Err(ScenarioError::ContractMissing(
            "PlaytestPlugin not added — calibrate_world needs the harness plugin".into(),
        ));
    }

    // Inject a minimal chaos scenario so harness systems can run.
    let scenario = serde_json::from_str::<Scenario>(&format!(
        r#"{{"bot":{{"type":"chaos","seed":{}}},"tps":{},"simulated_time":{}}}"#,
        opts.seed, opts.tps, opts.simulated_time
    ))
    .map_err(|e| ScenarioError::Rejected(format!("internal: bad calibration scenario: {e}")))?;
    app.insert_resource(ScenarioResource(std::sync::Arc::new(scenario)));
    app.insert_resource(Violations::default());

    // Robustness: insert PlaytestState if missing (the App may have been
    // constructed bare for calibration only).
    if app.world().get_resource::<PlaytestState>().is_none() {
        app.insert_resource(PlaytestState::new(opts.tps as u64, opts.seed));
    }
    if opts.simulated_time {
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_secs_f64(1.0 / opts.tps as f64),
        ));
    }

    let tps = app.world().resource::<PlaytestState>().tps;
    let total_ticks = (opts.duration_s * tps as f32) as u64;

    // Track per-signature count extremes across MANY samples — a
    // two-point (start/end) estimate misses spawn/despawn transients
    // (e.g. Startup-spawned Tiles sampled as 0 before tick 1).
    let mut archetype_min: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    let mut archetype_max: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    let mut resource_values = std::collections::HashMap::<String, Vec<f64>>::new();

    let sample_every = (tps / 10).max(1); // ~10 samples/sec

    // Run calibration, sampling archetype counts periodically. Panics
    // in game systems are captured: the snapshot gathered so far is
    // still returned (a crash mid-calibration usually still tells us
    // the archetype envelope).
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        for tick in 0..total_ticks {
            app.update();

            if tick % sample_every == 0 || tick == total_ticks - 1 {
                for archetype in app.world().archetypes().iter() {
                    let mut sig_parts: Vec<String> = archetype
                        .components()
                        .iter()
                        .filter_map(|id| {
                            app.world()
                                .components()
                                .get_info(*id)
                                .map(|info| info.name().to_string())
                        })
                        .collect();
                    sig_parts.sort();
                    let sig = sig_parts.join("|");
                    let len = archetype.entities().len();
                    if len == 0 {
                        continue;
                    }
                    let cur_min = archetype_min.entry(sig.clone()).or_insert(len);
                    if len < *cur_min {
                        *cur_min = len;
                    }
                    let cur_max = archetype_max.entry(sig).or_insert(len);
                    if len > *cur_max {
                        *cur_max = len;
                    }
                }
            }

            // FX12 (D1): sample numeric percepts through the
            // type-erased resolver (resolve_path) instead of reading
            // the concrete TestApi directly — games registering a
            // custom TestApi via register_test_api are now sampled too.
            {
                let world = app.world();
                if world.get_resource::<TestApiResolver>().is_some()
                    || world.get_resource::<TestApi>().is_some()
                {
                    for probe in ["TestApi.score", "TestApi.active_players"] {
                        if let Some(crate::contract::TestFieldValue::Numeric(v)) =
                            crate::contract::resolve_path(world, probe)
                        {
                            resource_values
                                .entry(probe.to_string())
                                .or_default()
                                .push(v);
                        }
                    }
                }
            }
        }
    })); // end catch_unwind
    let _ = result;

    // Compute envelopes
    let mut archetype_envelopes = Vec::new();
    let mut all_sigs: Vec<String> = archetype_min.keys().cloned().collect();
    all_sigs.sort();
    for sig in all_sigs {
        let min = archetype_min[&sig];
        let max = archetype_max[&sig];
        if max > 0 {
            archetype_envelopes.push((sig, min, max));
        }
    }

    let mut resource_baselines = Vec::new();
    for (name, values) in resource_values {
        let avg = values.iter().sum::<f64>() / values.len() as f64;
        resource_baselines.push((name, avg));
    }
    resource_baselines.sort_by(|a, b| a.0.cmp(&b.0));

    Ok(CalibrationSnapshot {
        archetype_envelopes,
        resource_baselines,
    })
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
            // Query with GAME-OWNED components only — engine internals
            // (Transform, Observer, ...) are not the game author's
            // contract and make brittle invariants.
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
                "check": "above",
                "value": -0.5,
                "eventually_s": null
            }));
        }
    }

    serde_json::to_string_pretty(&invariants).unwrap()
}

fn sanitize_name(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect()
}

// ---------------------------------------------------------------------------
// Frozen-world stuck oracle (aplib-inspired liveness check)
// ---------------------------------------------------------------------------
