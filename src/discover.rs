//! Z4 (swarm-m2h.5): input-surface discovery by differential probing.
//!
//! A Tier-0 game doesn't declare which inputs it reads. Discovery
//! answers it causally: input X matters in state S iff the A4 state
//! digest diverges between "press X at frame F" and "do nothing at
//! frame F", starting from the same deterministic prefix.
//!
//! Pipeline:
//! 0. Static device pruning — inspect game systems' read access
//!    (`Schedule::systems_with_access`), skipping device classes no
//!    game system reads.
//! 1. Baseline: replay bot, no inputs, `StateTrace` with snapshots.
//! 2. Probes: one fresh run per candidate (tap + hold variants),
//!    injected through the replayable raw-input path, run in
//!    parallel via `run_matrix`.
//! 3. Diff: first hash mismatch from frame F, then snapshot diff
//!    (ignoring harness-owned keys) fills `Effect.changed`.
//! 4. Group completion: arrows/WASD/d-pad/face-button groups are
//!    completed so a dead key's siblings still get sampled (the
//!    dead_verb oracle needs them to notice a dead verb).

use crate::branch::InProcess;
use crate::determinism::FrameDigest;
use crate::scenario::{RawAction, ReplayIntent, Scenario};
use std::collections::BTreeMap;

/// Probe tuning. Defaults probe ~1 s of headless play per candidate;
/// discovery for ~50 candidates stays well under 30 s.
#[derive(Clone, Debug)]
pub struct ProbeConfig {
    /// Frame at which the probe input fires. `None` = readiness + 1 s.
    pub probe_frame: Option<u64>,
    /// Frames observed after the probe input fires.
    pub observe_frames: u64,
    /// Ticks-per-second used for the replay scenarios.
    pub tps: u32,
    /// Maximum parallel probe runs.
    pub max_parallel: usize,
}

impl Default for ProbeConfig {
    fn default() -> Self {
        Self {
            probe_frame: None,
            observe_frames: 60,
            tps: 60,
            max_parallel: 8,
        }
    }
}

/// A differential effect: when a probe input diverged from baseline.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Effect {
    /// Frames between the probe input and the first digest
    /// divergence (1 = same frame; 0 = group-completion filler).
    pub first_divergence_frame_delta: u32,
    /// Top changed percept keys from the A4 snapshot diff
    /// (game-owned keys only).
    pub changed: Vec<String>,
}

/// Confidence in the discovered surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SurfaceConfidence {
    /// Deterministic differential probing (exact).
    Exact,
    /// Baseline was nondeterministic; statistical fallback (I5
    /// effect-rate comparison).
    Statistical,
}

/// The result of discovery.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct DiscoveredSurface {
    pub keys: Vec<(String, Effect)>,
    pub mouse_buttons: Vec<(String, Effect)>,
    pub mouse_motion: Option<Effect>,
    pub wheel: Option<Effect>,
    pub gamepad_buttons: Vec<(String, Effect)>,
    pub gamepad_axes: Vec<(String, Effect)>,
    /// Which states were probed (frame).
    pub probed_at: Vec<u64>,
    /// Game systems that read each device class (the input-adapter
    /// discovery artifact).
    pub readers: Vec<(String, Vec<String>)>,
    pub confidence: SurfaceConfidence,
    /// Device classes skipped by static pruning (no game reader).
    pub pruned_devices: Vec<String>,
}

impl Default for DiscoveredSurface {
    fn default() -> Self {
        Self {
            keys: vec![],
            mouse_buttons: vec![],
            mouse_motion: None,
            wheel: None,
            gamepad_buttons: vec![],
            gamepad_axes: vec![],
            probed_at: vec![],
            readers: vec![],
            confidence: SurfaceConfidence::Exact,
            pruned_devices: vec![],
        }
    }
}

impl DiscoveredSurface {
    /// Group completion: if any member of a known verb group is
    /// discovered, include the whole group (so a dead key's siblings
    /// stay sampleable for the dead_verb oracle).
    pub fn apply_group_completion(&mut self) {
        const ARROWS: &[&str] = &["ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight"];
        const WASD: &[&str] = &["KeyW", "KeyA", "KeyS", "KeyD"];
        for group in [ARROWS, WASD] {
            if self.keys.iter().any(|(k, _)| group.contains(&k.as_str())) {
                for k in group {
                    if !self.keys.iter().any(|(dk, _)| dk == *k) {
                        self.keys.push((
                            k.to_string(),
                            Effect {
                                first_divergence_frame_delta: 0,
                                changed: vec![
                                    "group-completion (sibling of a live key)".to_string()
                                ],
                            },
                        ));
                    }
                }
            }
        }
        const DPAD: &[&str] = &["DPadUp", "DPadDown", "DPadLeft", "DPadRight"];
        const FACE: &[&str] = &["South", "East", "North", "West"];
        for group in [DPAD, FACE] {
            if self
                .gamepad_buttons
                .iter()
                .any(|(b, _)| group.contains(&b.as_str()))
            {
                for b in group {
                    if !self.gamepad_buttons.iter().any(|(db, _)| db == *b) {
                        self.gamepad_buttons.push((
                            b.to_string(),
                            Effect {
                                first_divergence_frame_delta: 0,
                                changed: vec![
                                    "group-completion (sibling of a live button)".to_string()
                                ],
                            },
                        ));
                    }
                }
            }
        }
    }
}

/// Reusable snapshot diff: keys whose values differ, with both
/// values. Factored out of the determinism checker (Z4 reuses it).
pub fn diff_snapshots(
    a: &BTreeMap<String, String>,
    b: &BTreeMap<String, String>,
) -> Vec<(String, String, String)> {
    let mut out = vec![];
    for (k, va) in a {
        if let Some(vb) = b.get(k) {
            if va != vb {
                out.push((k.clone(), va.clone(), vb.clone()));
            }
        }
    }
    out
}

/// Harness-owned percept keys ignored by the diff: the entity
/// census (the virtual gamepad + headless window shift it) and
/// harness resources.
fn is_harness_owned(key: &str) -> bool {
    key.starts_with("census.")
        || key.starts_with("bevy_swarm::")
        || key.starts_with("Resource:RawAction")
        || key.starts_with("Resource:ActionEffects")
        || key.starts_with("Resource:Violations")
        || key.starts_with("Resource:PlaytestMetrics")
        || key.starts_with("Resource:ChoiceStream")
        || key.starts_with("Resource:StateTrace")
        || key.starts_with("Resource:DigestHooks")
        || key.starts_with("Resource:PlaytestState")
        || key.starts_with("Resource:HeadlessWindow")
}

/// Prior-likelihood key candidates (ordered by likelihood).
const PRIOR_KEYS: &[&str] = &[
    "KeyW",
    "KeyA",
    "KeyS",
    "KeyD",
    "ArrowUp",
    "ArrowDown",
    "ArrowLeft",
    "ArrowRight",
    "Space",
    "Enter",
    "Escape",
    "ShiftLeft",
    "ControlLeft",
    "Tab",
    "KeyE",
    "KeyF",
    "KeyQ",
    "KeyR",
    "Digit1",
    "Digit2",
    "Digit3",
    "Digit4",
    "Digit5",
    "Digit6",
    "Digit7",
    "Digit8",
    "Digit9",
];

const PRIOR_PAD_BUTTONS: &[&str] = &[
    "South",
    "East",
    "West",
    "North",
    "Start",
    "Select",
    "DPadUp",
    "DPadDown",
    "DPadLeft",
    "DPadRight",
];

/// One probe candidate.
#[derive(Clone)]
struct Candidate {
    label: String,
    action: RawAction,
}

/// Discover the input surface of a game via differential probing.
///
/// `factory` must build a fresh, equivalent App per call (the same
/// contract as `InProcess::run`). Probes run in parallel via
/// `run_matrix`; each is a full scenario run (prefix + one input).
pub fn discover_surface<F>(factory: F, ctx: &ProbeConfig) -> DiscoveredSurface
where
    F: Fn() -> bevy::prelude::App + Send + Sync + 'static,
{
    // Every probe run needs StateTrace with snapshots: wrap the
    // factory so every App it produces records digests.
    let factory = {
        let factory = factory;
        move || {
            let mut app = factory();
            app.insert_resource(crate::determinism::StateTrace {
                with_snapshots: true,
                stop_after: None,
                digests: vec![],
            });
            app
        }
    };

    // ---- Step 0: static device pruning -----------------------------
    // systems_with_access requires an initialized schedule — run one
    // update first (the probe app is then discarded).
    let mut probe_app = factory();
    // Z18: a game that panics at startup (panic_on_load) must not
    // crash the harness; catch the panic and treat as "no surface".
    let probe_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        probe_app.update();
    }));
    if probe_result.is_err() {
        // Startup panic: return an empty surface (autotest will still
        // run seeds and report the crash via the panic oracle).
        return DiscoveredSurface {
            keys: vec![],
            mouse_buttons: vec![],
            mouse_motion: None,
            wheel: None,
            gamepad_buttons: vec![],
            gamepad_axes: vec![],
            probed_at: vec![],
            readers: vec![],
            confidence: SurfaceConfidence::Statistical,
            pruned_devices: vec![],
        };
    }
    let (readers, pruned) = static_device_readers(&probe_app);

    let probe_keys = !pruned.contains(&"keyboard".to_string());
    let probe_mouse_buttons = !pruned.contains(&"mouse_buttons".to_string());
    let probe_motion = !pruned.contains(&"mouse_motion".to_string());
    let probe_wheel = !pruned.contains(&"wheel".to_string());
    let probe_gamepad = !pruned.contains(&"gamepad".to_string());

    // ---- Step 1: baseline ------------------------------------------
    let (baseline_digests, probe_frame) = run_baseline(&factory, ctx);

    // ---- Step 2: probe runs ----------------------------------------
    let mut candidates: Vec<Candidate> = vec![];
    if probe_keys {
        for k in PRIOR_KEYS {
            for hold in [1u32, 10] {
                candidates.push(Candidate {
                    label: format!("raw:key:{k}"),
                    action: RawAction::Key {
                        key: k.to_string(),
                        hold_frames: hold,
                    },
                });
            }
        }
    }
    if probe_mouse_buttons {
        for b in [
            crate::scenario::MouseBtn::Left,
            crate::scenario::MouseBtn::Right,
        ] {
            candidates.push(Candidate {
                label: format!("raw:mouse:{b:?}"),
                action: RawAction::MouseButton {
                    button: b,
                    hold_frames: 1,
                },
            });
        }
    }
    if probe_motion {
        candidates.push(Candidate {
            label: "raw:mouse_motion".into(),
            action: RawAction::MouseMove { delta: (10.0, 5.0) },
        });
    }
    if probe_wheel {
        candidates.push(Candidate {
            label: "raw:wheel".into(),
            action: RawAction::Wheel { dy: 1.0 },
        });
    }
    if probe_gamepad {
        for b in PRIOR_PAD_BUTTONS {
            candidates.push(Candidate {
                label: format!("raw:gamepad:{b}"),
                action: RawAction::GamepadButton {
                    button: b.to_string(),
                    hold_frames: 1,
                },
            });
        }
        for axis in ["LeftStickX", "LeftStickY"] {
            candidates.push(Candidate {
                label: format!("raw:gamepad_axis:{axis}"),
                action: RawAction::GamepadAxis {
                    axis: axis.to_string(),
                    value: 0.7,
                    hold_frames: 10,
                },
            });
        }
    }

    let variants: Vec<(String, Scenario)> = candidates
        .iter()
        .map(|c| (c.label.clone(), probe_scenario(&c.action, probe_frame, ctx)))
        .collect();

    let runner = InProcess { factory };
    // run_matrix swallows scenario errors as Crash outcomes; empty
    // state_trace is treated as no effect below.
    let matrix = crate::branch::run_matrix(&runner, variants, ctx.max_parallel)
        .map(|m| m.outcomes)
        .unwrap_or_default();

    // ---- Step 3: diff ----------------------------------------------
    let mut surface = DiscoveredSurface {
        probed_at: vec![probe_frame],
        readers,
        pruned_devices: pruned,
        confidence: SurfaceConfidence::Exact,
        ..Default::default()
    };

    // Best (earliest divergence, most changed keys) effect per label.
    let mut best: BTreeMap<String, Effect> = BTreeMap::new();
    for (i, outcome) in matrix.iter().enumerate() {
        let Some(trace) = outcome.report.state_trace.as_ref() else {
            continue;
        };
        if let Some(effect) = diff_trace(trace, &baseline_digests, probe_frame) {
            let label = candidates[i].label.clone();
            let better = match best.get(&label) {
                Some(prev) => {
                    effect.changed.len() > prev.changed.len()
                        || (effect.changed.len() == prev.changed.len()
                            && effect.first_divergence_frame_delta
                                < prev.first_divergence_frame_delta)
                }
                None => true,
            };
            if better {
                best.insert(label, effect);
            }
        }
    }

    for (label, effect) in best {
        let (family, name) = label.rsplit_once(':').unwrap_or((label.as_str(), ""));
        match family {
            "raw:key" => surface.keys.push((name.to_string(), effect)),
            "raw:mouse" => surface.mouse_buttons.push((name.to_string(), effect)),
            "raw:gamepad" => surface.gamepad_buttons.push((name.to_string(), effect)),
            "raw:gamepad_axis" => surface.gamepad_axes.push((name.to_string(), effect)),
            "raw:mouse_motion" => surface.mouse_motion = Some(effect),
            "raw:wheel" => surface.wheel = Some(effect),
            _ => {}
        }
    }

    // ---- Group completion (dead_verb needs siblings) ----------------
    surface.apply_group_completion();
    surface
}

/// Compare a probe trace against the baseline from `probe_frame`
/// onward; on first hash mismatch, diff snapshots at that frame
/// (ignoring harness-owned keys).
fn diff_trace(probe: &[FrameDigest], baseline: &[FrameDigest], probe_frame: u64) -> Option<Effect> {
    let base_from: Vec<&FrameDigest> = baseline.iter().filter(|d| d.frame >= probe_frame).collect();
    let probe_from: Vec<&FrameDigest> = probe.iter().filter(|d| d.frame >= probe_frame).collect();
    for (p, b) in probe_from.iter().zip(base_from.iter()) {
        if p.hash != b.hash {
            let changed = match (&p.snapshot, &b.snapshot) {
                (Some(pa), Some(pb)) => diff_snapshots(pa, pb)
                    .into_iter()
                    .filter(|(k, _, _)| !is_harness_owned(k))
                    .map(|(k, _, _)| k)
                    .collect(),
                _ => vec![],
            };
            return Some(Effect {
                first_divergence_frame_delta: (p.frame - probe_frame + 1).max(1) as u32,
                changed,
            });
        }
    }
    None
}

/// Baseline run: replay bot with no inputs, StateTrace on.
/// Returns (digests, probe_frame).
fn run_baseline<F>(factory: &F, ctx: &ProbeConfig) -> (Vec<FrameDigest>, u64)
where
    F: Fn() -> bevy::prelude::App + Send + Sync + 'static,
{
    let duration_s = (ctx.observe_frames.max(1) as f32) / (ctx.tps.max(1) as f32) + 2.0;
    let scenario = empty_replay_scenario(duration_s, ctx.tps);
    let mut app = factory();
    app.insert_resource(crate::determinism::StateTrace {
        with_snapshots: true,
        stop_after: None,
        digests: vec![],
    });
    let report = crate::driver::run_scenario(&mut app, &scenario)
        .unwrap_or_else(|_| crate::driver::PlaytestReport::crashed_empty());
    let digests = report.state_trace.unwrap_or_default();
    let probe_frame = ctx.probe_frame.unwrap_or_else(|| {
        // readiness + 1 s.
        report.pre_ready_frames + ctx.tps as u64
    });
    (digests, probe_frame)
}

/// Build the probe scenario for a candidate: replay bot, one raw
/// input at `probe_frame`.
fn probe_scenario(action: &RawAction, probe_frame: u64, ctx: &ProbeConfig) -> Scenario {
    let prefix_s = (probe_frame as f32) / (ctx.tps.max(1) as f32);
    let observe_s = (ctx.observe_frames.max(1) as f32) / (ctx.tps.max(1) as f32);
    let mut scen = empty_replay_scenario(prefix_s + observe_s + 2.0, ctx.tps);
    scen.bot.inputs = vec![crate::scenario::ReplayInput {
        frame: probe_frame,
        intent: ReplayIntent::Raw {
            raw: action.clone(),
        },
    }];
    scen
}

fn empty_replay_scenario(duration_s: f32, tps: u32) -> Scenario {
    let mut scen: Scenario = serde_json::from_str(&format!(
        r#"{{"bot":{{"type":"replay","inputs":[]}},"duration_s":{duration_s},"tps":{tps}}}"#
    ))
    .expect("valid scenario JSON");
    scen.duration_s = duration_s;
    scen.tps = tps;
    scen
}

/// Static device pruning (step 0): which device classes do GAME
/// systems actually read? Uses per-system access sets (public in
/// bevy_ecs 0.20). Returns (readers per class, pruned classes).
fn static_device_readers(app: &bevy::prelude::App) -> (Vec<(String, Vec<String>)>, Vec<String>) {
    use std::any::TypeId;
    let world = app.world();

    #[derive(Default)]
    struct Acc {
        keyboard: Vec<String>,
        mouse_buttons: Vec<String>,
        mouse_motion: Vec<String>,
        wheel: Vec<String>,
        gamepad: Vec<String>,
        has_read_all: bool,
    }

    let mut acc = Acc::default();
    let Some(schedules) = world.get_resource::<bevy::ecs::schedule::Schedules>() else {
        return (vec![], vec![]);
    };
    for (_label, schedule) in schedules.iter() {
        let Ok(systems) = schedule.systems_with_access() else {
            continue;
        };
        for (_key, swa) in systems {
            use bevy::ecs::system::System;
            let name = swa.name().to_string();
            // Only GAME systems: skip bevy_swarm (harness) and
            // bevy_* (engine).
            if name.starts_with("bevy_swarm::") || name.starts_with("bevy_") {
                continue;
            }
            let combined = match swa.access() {
                bevy::ecs::system::SystemAccess::None => continue,
                bevy::ecs::system::SystemAccess::Shared(set) => set.combined_access(),
                // Exclusive systems may read anything.
                bevy::ecs::system::SystemAccess::Exclusive => {
                    acc.has_read_all = true;
                    continue;
                }
            };
            let reads = |tid: TypeId| {
                world
                    .components()
                    .get_valid_id(tid)
                    .is_some_and(|cid| combined.has_read(cid))
            };
            if reads(TypeId::of::<
                bevy::input::ButtonInput<bevy::input::keyboard::KeyCode>,
            >()) {
                acc.keyboard.push(name.clone());
            }
            if reads(TypeId::of::<
                bevy::input::ButtonInput<bevy::input::mouse::MouseButton>,
            >()) {
                acc.mouse_buttons.push(name.clone());
            }
            if reads(TypeId::of::<bevy::input::mouse::AccumulatedMouseMotion>()) {
                acc.mouse_motion.push(name.clone());
            }
            if reads(TypeId::of::<bevy::input::mouse::AccumulatedMouseScroll>()) {
                acc.wheel.push(name.clone());
            }
            if reads(TypeId::of::<bevy::input::gamepad::Gamepad>()) {
                acc.gamepad.push(name.clone());
            }
            if combined.has_read_all() {
                acc.has_read_all = true;
            }
        }
    }

    // Never prune anything if a game system is Exclusive or
    // has_read_all.
    if acc.has_read_all {
        return (vec![], vec![]);
    }

    let mut readers = vec![];
    let mut pruned = vec![];
    let mut classify = |class: &str, list: Vec<String>| {
        if list.is_empty() {
            pruned.push(class.to_string());
        } else {
            readers.push((class.to_string(), list));
        }
    };
    classify("keyboard", std::mem::take(&mut acc.keyboard));
    classify("mouse_buttons", std::mem::take(&mut acc.mouse_buttons));
    classify("mouse_motion", std::mem::take(&mut acc.mouse_motion));
    classify("wheel", std::mem::take(&mut acc.wheel));
    classify("gamepad", std::mem::take(&mut acc.gamepad));
    (readers, pruned)
}
