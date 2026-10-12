//! A4: Determinism self-check oracle (ggrs-SyncTest style).
//!
//! Verifies that a scenario is deterministic (same seed → same trace)
//! rather than assuming it: runs the scenario several times on fresh
//! Apps and compares per-frame canonical state digests, pinpointing
//! the first divergent frame and fields when it isn't.
//!
//! A4 also owns the shared FrameRecorder concept: per-frame digests,
//! optional snapshots, and game-provided `DigestHooks`. Later beads
//! (T3, Z11, T11, I6, S2) consume `StateTrace` instead of building
//! their own recorders.

use std::collections::BTreeMap;

use bevy::app::App;
use bevy::ecs::resource::Resource;
use serde::Serialize;

use crate::driver::run_scenario;
use crate::scenario::{Scenario, ScenarioError};

/// Per-frame canonical state digest recorded during a run.
#[derive(Clone, Debug, Serialize)]
pub struct FrameDigest {
    pub frame: u64,
    pub hash: u64,
    /// Only populated when tracing (`StateTrace::with_snapshots`);
    /// explains WHICH fields diverged.
    pub snapshot: Option<BTreeMap<String, String>>,
}

/// Games add their own digest contributors (e.g. RNG state, AI
/// blackboards).
/// Hook for game-provided digest contributions.
pub type DigestHookFn = dyn Fn(&bevy::ecs::world::World) -> String + Send + Sync;

/// Games add their own digest contributors (e.g. RNG state, AI
/// blackboards).
#[derive(Resource, Default)]
pub struct DigestHooks(pub Vec<(String, Box<DigestHookFn>)>);

/// Records per-frame digests while present in the world; normal runs
/// (no resource) pay nothing.
#[derive(Resource, Default)]
pub struct StateTrace {
    pub with_snapshots: bool,
    /// Stop recording after this frame (bounds memory during
    /// divergence tracing).
    pub stop_after: Option<u64>,
    pub digests: Vec<FrameDigest>,
}

/// A single divergent field at a frame.
#[derive(Clone, Debug, Serialize)]
pub struct Divergence {
    pub frame: u64,
    /// Keys whose values differ (key, run_a, run_b).
    pub differing: Vec<(String, String, String)>,
}

/// Result of a determinism check.
#[derive(Clone, Debug, Serialize)]
pub struct DeterminismReport {
    pub frames_compared: u64,
    pub first_divergence: Option<Divergence>,
}

impl DeterminismReport {
    pub fn is_deterministic(&self) -> bool {
        self.first_divergence.is_none()
    }
}

/// Canonical, order-independent state snapshot: sorted key/value pairs.
/// Includes the TestApi/custom surface, Gameplay transforms (bit-exact
/// via f32::to_bits), entity/archetype census, and DigestHooks.
pub fn canonical_state(world: &bevy::ecs::world::World) -> BTreeMap<String, String> {
    use bevy::prelude::{Name, Transform};

    let mut map = BTreeMap::new();

    // 1. Scenario-referenced observable surface: resolve every
    // TestApi-style path the crate's resolver knows (D1's resolve_path
    // covers custom TestApi and world percepts).
    // TestApi custom entries (crate conventions).
    if let Some(api) = world.get_resource::<crate::contract::TestApi>() {
        for (k, v) in &api.custom_numeric {
            map.insert(format!("TestApi.{}", k), v.to_string());
        }
        for (k, v) in &api.custom_text {
            map.insert(format!("TestApi.{}", k), v.clone());
        }
    }
    // resolve_path-observable numeric paths via reflection on Resources.
    collect_resource_percepts(world, &mut map);

    // 2. Gameplay entities: bit-exact transforms keyed by name (with
    // occurrence index for collisions).
    let mut names: std::collections::HashMap<String, usize> = Default::default();
    #[derive(Clone)]
    struct Row {
        key: String,
        pos: [String; 3],
        rotation: String,
    }
    let mut rows: Vec<Row> = vec![];
    for entity in world.iter_entities() {
        let (Some(name), Some(t)) = (entity.get::<Name>(), entity.get::<Transform>()) else {
            continue;
        };
        if entity.get::<crate::harness::Gameplay>().is_none() {
            continue;
        }
        let occ = names.entry(name.as_str().to_string()).or_insert(0);
        let key = if *occ == 0 {
            name.as_str().to_string()
        } else {
            format!("{}#{}", name.as_str(), occ)
        };
        *occ += 1;
        rows.push(Row {
            key,
            pos: [
                t.translation.x.to_bits().to_string(),
                t.translation.y.to_bits().to_string(),
                t.translation.z.to_bits().to_string(),
            ],
            // 6wt: Quat Debug output is not stable across platforms/
            // rustc versions — encode rotation components as bit
            // patterns (matching translation) so golden files recorded
            // on machine A don't diverge on machine B.
            rotation: format!(
                "({},{},{},{})",
                t.rotation.x.to_bits(),
                t.rotation.y.to_bits(),
                t.rotation.z.to_bits(),
                t.rotation.w.to_bits()
            ),
        });
    }
    for row in &rows {
        map.insert(
            format!("Gameplay[{}].translation.x", row.key),
            row.pos[0].clone(),
        );
        map.insert(
            format!("Gameplay[{}].translation.y", row.key),
            row.pos[1].clone(),
        );
        map.insert(
            format!("Gameplay[{}].translation.z", row.key),
            row.pos[2].clone(),
        );
        map.insert(
            format!("Gameplay[{}].rotation", row.key),
            row.rotation.clone(),
        );
    }

    // 3. Entity/archetype census (catches spawn divergence).
    map.insert("census.entities".into(), world.entities().len().to_string());
    map.insert(
        "census.archetypes".into(),
        world.archetypes().len().to_string(),
    );

    // 4. Game-provided digest parts.
    if let Some(hooks) = world.get_resource::<DigestHooks>() {
        for (key, f) in &hooks.0 {
            map.insert(format!("hook.{}", key), f(world));
        }
    }

    map
}

/// Collect resource percepts: every REGISTERED resource whose type has
/// `ReflectComponent` data gets one `Resource:<ShortTypePath>` entry
/// (Debug-formatted whole-resource string). Resources are singletons
/// so the count is bounded. Harness-internal resources are excluded —
/// hashing harness bookkeeping (TestApi, StateTrace...) would flag
/// harness behavior as divergence.
fn collect_resource_percepts(world: &bevy::ecs::world::World, map: &mut BTreeMap<String, String>) {
    let Some(registry_arc) = world.get_resource::<bevy::ecs::reflect::AppTypeRegistry>() else {
        return;
    };
    let registry = registry_arc.0.read();

    let internal: &[&str] = &["TestApi", "StateTrace", "DigestHooks", "PlaytestState"];

    for type_reg in registry.iter() {
        let short = type_reg.type_info().type_path_table().short_path();
        if internal.contains(&short) {
            continue;
        }
        let Some(reflect_component) = type_reg.data::<bevy::ecs::reflect::ReflectComponent>()
        else {
            continue;
        };
        // Engine resources (bevy_time::Time etc.) carry wall-clock
        // Instants and task-pool state — inherently non-deterministic
        // and not GAME state. Only game-crate resources are digested.
        let full_path = type_reg.type_info().type_path_table().path();
        if full_path.starts_with("bevy_") || full_path.starts_with("bevy::") {
            continue;
        }
        let Some(type_id) = registry.get_with_short_type_path(short) else {
            continue;
        };
        let Some(comp_id) = world.components().get_valid_id(type_id.type_id()) else {
            continue;
        };
        // Resources live on singleton entities addressed via
        // resource_entities.
        let Some(entity) = world
            .resource_entities()
            .iter()
            .find(|(id, _)| *id == comp_id)
            .map(|(_, e)| e)
        else {
            continue;
        };
        let Ok(entity_ref) = world.get_entity(entity) else {
            continue;
        };
        let Some(reflect) = reflect_component.reflect(entity_ref) else {
            continue;
        };
        map.insert(format!("Resource:{}", short), format!("{:?}", reflect));
    }
}

/// Exclusive oracle: record the canonical state digest for this frame
/// when a StateTrace resource exists. Zero cost otherwise.
pub(crate) fn record_state_digest_system(world: &mut bevy::ecs::world::World) {
    if !world.contains_resource::<StateTrace>() {
        return;
    }
    let stop = world.resource::<StateTrace>().stop_after;
    let frame = world.resource::<crate::state::PlaytestState>().frame;
    if let Some(stop_at) = stop {
        if frame > stop_at {
            return;
        }
    }
    let map = canonical_state(world);
    let mut h = std::hash::DefaultHasher::new();
    use std::hash::{Hash, Hasher};
    for (k, v) in &map {
        k.hash(&mut h);
        v.hash(&mut h);
    }
    let with_snapshots = world.resource::<StateTrace>().with_snapshots;
    let snapshot = with_snapshots.then_some(map);
    let Some(mut t) = world.get_resource_mut::<StateTrace>() else {
        return;
    };
    t.digests.push(FrameDigest {
        frame,
        hash: h.finish(),
        snapshot,
    });
}

/// Run `scenario` twice on fresh Apps and compare per-frame digests.
/// Returns `(matches, first_divergence_frame, first_run_digests)` —
/// the frame of the earliest hash mismatch (or the common-prefix
/// length when only the trace lengths differ), plus the first run's
/// digests (with snapshots) so callers can reuse the run instead of
/// paying a third one.
pub(crate) fn compare_two_runs_digests(
    app_builder: impl Fn() -> App,
    scenario: &Scenario,
) -> Result<(bool, Option<u64>, Vec<FrameDigest>), ScenarioError> {
    let run_once = || -> Result<Vec<FrameDigest>, ScenarioError> {
        let mut app = app_builder();
        app.insert_resource(StateTrace {
            with_snapshots: true,
            stop_after: None,
            digests: vec![],
        });
        let report = run_scenario(&mut app, scenario)?;
        Ok(report.state_trace.unwrap_or_default())
    };
    let trace_a = run_once()?;
    let trace_b = run_once()?;
    let matches = trace_a.len() == trace_b.len()
        && trace_a
            .iter()
            .zip(trace_b.iter())
            .all(|(a, b)| a.hash == b.hash);
    let first_divergence = if matches {
        None
    } else {
        Some(
            trace_a
                .iter()
                .zip(trace_b.iter())
                .position(|(a, b)| a.hash != b.hash)
                .map(|i| trace_a[i].frame)
                .unwrap_or(trace_a.len().min(trace_b.len()) as u64),
        )
    };
    Ok((matches, first_divergence, trace_a))
}

/// Run `scenario` `runs` times (>= 2) on fresh Apps and compare
/// per-frame digests. On mismatch, traces twice more with snapshots to
/// explain WHICH fields diverged (reporting the earliest divergence
/// found in the traced pair, falling back to the hash-mismatch frame).
pub fn check_determinism(
    app_builder: impl Fn() -> App,
    scenario: &Scenario,
    runs: usize,
) -> Result<DeterminismReport, ScenarioError> {
    let runs = runs.max(2);
    let mut first_run: Option<Vec<FrameDigest>> = None;
    let mut mismatch_frame: Option<u64> = None;

    for _ in 0..runs {
        let mut app = app_builder();
        app.insert_resource(StateTrace {
            with_snapshots: false,
            stop_after: None,
            digests: vec![],
        });
        let report = run_scenario(&mut app, scenario)?;
        let digests = report.state_trace.unwrap_or_default();
        match &first_run {
            None => first_run = Some(digests),
            Some(baseline) => {
                // FX6 A4: hash scan FIRST (zip covers the common prefix —
                // the earliest divergence frame), length mismatch is only
                // the fallback when all common frames agree (a crashed
                // run previously reported len, hiding the earlier hash
                // mismatch).
                for (a, b) in digests.iter().zip(baseline.iter()) {
                    if a.hash != b.hash && mismatch_frame.is_none() {
                        mismatch_frame = Some(a.frame);
                    }
                }
                if digests.len() != baseline.len() && mismatch_frame.is_none() {
                    mismatch_frame = Some(digests.len().min(baseline.len()) as u64);
                }
            }
        }
    }

    let Some(frame) = mismatch_frame else {
        return Ok(DeterminismReport {
            frames_compared: first_run.map_or(0, |d| d.len() as u64),
            first_divergence: None,
        });
    };

    // Traced pair to explain the divergence.
    let mut snapshots: Vec<BTreeMap<String, String>> = vec![];
    for _ in 0..2 {
        let mut app = app_builder();
        app.insert_resource(StateTrace {
            with_snapshots: true,
            stop_after: Some(frame),
            digests: vec![],
        });
        let report = run_scenario(&mut app, scenario)?;
        let digests = report.state_trace.unwrap_or_default();
        // The snapshot at the STOP frame (the divergence frame), not
        // frame 0 — comparing frame-0 states yields no differences.
        if let Some(fd) = digests.iter().rev().find(|d| d.snapshot.is_some()) {
            if let Some(s) = &fd.snapshot {
                snapshots.push(s.clone());
            }
        }
    }

    let mut differing = vec![];
    if snapshots.len() == 2 {
        let (a, b) = (&snapshots[0], &snapshots[1]);
        for key in a
            .keys()
            .chain(b.keys())
            .collect::<std::collections::BTreeSet<_>>()
        {
            let va = a.get(key).cloned().unwrap_or_default();
            let vb = b.get(key).cloned().unwrap_or_default();
            if va != vb {
                differing.push((key.clone(), va, vb));
            }
        }
    }
    let reported_frame = differing
        .first()
        .map(|_| frame)
        .or(Some(frame))
        .unwrap_or(frame);

    Ok(DeterminismReport {
        frames_compared: first_run.map_or(0, |d| d.len() as u64),
        first_divergence: Some(Divergence {
            frame: reported_frame,
            differing,
        }),
    })
}
