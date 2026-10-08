//! Bots — all emit UserIntent (contract-aware, genre-general).

use bevy::ecs::entity::Entity;
use bevy::ecs::message::MessageWriter;
use bevy::ecs::query::With;
use bevy::ecs::system::{Query, Res, ResMut};
use bevy::picking::pointer::{PointerAction, PointerButton, PointerId, PointerInput};
use bevy::prelude::Camera;
use bevy::prelude::{GlobalTransform, Name};

use crate::contract::{Gameplay, IntentSurface, SurfaceVariant, UserIntent};
use crate::driver::ScenarioResource;
use crate::scenario::*;
use crate::state::*;

// ---------------------------------------------------------------------------
// Bots — all emit UserIntent (contract-aware, genre-general)
// ---------------------------------------------------------------------------

/// chaos: samples a random intent variant FROM THE GAME'S DECLARED
/// `IntentSurface` each fire interval. The harness holds zero genre
/// knowledge — a dialogue game gets random Choices (within its declared
/// range), a strategy game gets random Selects/Chooses, a racing game gets
/// declared axes. An empty/missing surface is a contract violation
/// (loud), not a silent no-op — chaos refuses to issue input it can't
/// know the game consumes. `Select` targets are drawn from live
/// `Gameplay` entities (a placeholder entity would be ignored-or-panic
/// noise, not a test). `SurfaceVariant::Axis(name)` counts as variant
/// `"axis:<name>"` for coverage tracking.
#[allow(clippy::too_many_arguments)]
pub(crate) fn chaos_bot_system(
    mut state: ResMut<PlaytestState>,
    mut choices: ResMut<crate::choice::ChoiceStream>,
    mut intents: MessageWriter<UserIntent>,
    mut violations: ResMut<Violations>,
    surface: Option<Res<IntentSurface>>,
    q: Query<Entity, With<Gameplay>>,
    q_named: Query<(Entity, &Name), With<Gameplay>>,
    scenario: Res<ScenarioResource>,
) {
    // Persona bias (): aggressive = 2x rate + never Wait; curious =
    // strong preference for unseen variants; idle = mostly Wait with rare
    // jabs. Different personas find different bugs.
    let persona = scenario
        .0
        .bot
        .persona
        .unwrap_or(crate::enums::Persona::Curious);
    let rate = scenario.0.bot.input_rate_hz.max(1);
    let effective_rate = if persona == crate::enums::Persona::Aggressive {
        rate.saturating_mul(2)
    } else {
        rate
    };
    let fire_every = (state.tps / effective_rate as u64).max(1);
    if !state.frame.is_multiple_of(fire_every) {
        return;
    }
    // Idle persona: skip 7 of 8 fire slots entirely (occasional jabs).
    if persona == crate::enums::Persona::Idle && !choices.chance(state.frame, 1, 8) {
        return;
    }
    let Some(surface) = surface else {
        violations.report(
            "chaos_bot_config",
            "",
            "game does not implement testable-conventions: IntentSurface resource missing — chaos cannot know which intents the game consumes"
                .to_string(),
            state.frame,
        );
        return;
    };
    if surface.0.is_empty() {
        violations.report(
            "chaos_bot_config",
            "",
            "IntentSurface is empty — the game declared no intent variants. Declare the game's input surface (IntentSurface::new(...))"
                .to_string(),
            state.frame,
        );
        return;
    }
    let roll_uniform = choices.below(state.frame, surface.0.len() as u64) as usize;
    // Aggressive persona: never Wait — draw from the FILTERED candidate
    // set (no reject loops anywhere; a Wait-only surface still yields
    // Wait rather than hanging).
    let roll = if persona == crate::enums::Persona::Aggressive {
        let candidates: Vec<usize> = surface
            .0
            .iter()
            .enumerate()
            .filter(|(_, v)| !matches!(v, SurfaceVariant::Wait))
            .map(|(i, _)| i)
            .collect();
        if candidates.is_empty() {
            roll_uniform
        } else {
            candidates[choices.below(state.frame, candidates.len() as u64) as usize]
        }
    } else {
        roll_uniform
    };
    let roll = roll as u64;
    let variant_name: String;
    let intent = match &surface.0[roll as usize] {
        SurfaceVariant::Move => {
            variant_name = "move".into();
            UserIntent::Move {
                dir: bevy::math::Vec2::new(
                    choices.unit_f32(state.frame, -1.0, 1.0),
                    choices.unit_f32(state.frame, -1.0, 1.0),
                ),
            }
        }
        SurfaceVariant::Choice(max_index) => {
            variant_name = "choice".into();
            UserIntent::Choice {
                index: choices.below(state.frame, *max_index as u64 + 1) as usize,
            }
        }
        SurfaceVariant::Axis(name) => {
            variant_name = "axis".into();
            UserIntent::Axis {
                name: name.clone(),
                value: choices.unit_f32(state.frame, -1.0, 1.0),
            }
        }
        SurfaceVariant::Select => {
            variant_name = "select".into();
            // Draw from live Gameplay entities; zero entities = surface
            // declaration mismatch (declared Select but nothing selectable)
            let all: Vec<Entity> = q.iter().collect();
            if all.is_empty() {
                violations.report(
                    "chaos_bot_config",
                    "",
                    "SurfaceVariant::Select declared but no Gameplay entities exist to select from"
                        .to_string(),
                    state.frame,
                );
                return;
            }
            // Prefer NAMED candidates: Select on an unnamed entity is
            // recorded as entity bits only and is NOT replayable
            // (counted via unreplayable_actions). Warn once per run.
            let named: Vec<Entity> = q_named.iter().map(|(e, _)| e).collect();
            let pool = if named.is_empty() {
                if !state.warned_select_unnamed {
                    state.warned_select_unnamed = true;
                    violations.report(
                        "chaos_select_unnamed",
                        "",
                        "Select targets have no Name — runs are not replayable; add Name to Gameplay entities"
                            .to_string(),
                        state.frame,
                    );
                }
                &all
            } else {
                &named
            };
            let idx = choices.below(state.frame, pool.len() as u64) as usize;
            UserIntent::Select { target: pool[idx] }
        }
        SurfaceVariant::Wait => {
            variant_name = "wait".into();
            UserIntent::Wait
        }
    };
    // Coverage: track which variant was sampled
    state
        .coverage
        .intents_emitted
        .entry(variant_name.clone())
        .and_modify(|c| *c += 1)
        .or_insert(1);
    state.coverage.chaos_surface_indices.insert(roll);
    // Context for violations
    violations.set_context(format!("chaos:{},idx={}", variant_name, roll));
    intents.write(intent);
    state.metrics.input_count += 1;
}

/// replay: frame-indexed typed intents — genre-free. Tracks coverage
/// of each intent type replayed.
pub(crate) fn replay_bot_system(
    mut state: ResMut<PlaytestState>,
    mut intents: MessageWriter<UserIntent>,
    mut violations: ResMut<Violations>,
    q_named: Query<(bevy::ecs::entity::Entity, &Name), With<Gameplay>>,
    scenario: Res<ScenarioResource>,
    idx: Res<crate::identity::IdentityIndex>,
) {
    for entry in &scenario.0.bot.inputs {
        if entry.frame == state.frame {
            let variant_name: String;
            let intent = match &entry.intent {
                ReplayIntent::Move { dir } => {
                    variant_name = "move".into();
                    UserIntent::Move {
                        dir: bevy::math::Vec2::new(dir.0, dir.1),
                    }
                }
                ReplayIntent::Choice { index } => {
                    variant_name = "choice".into();
                    UserIntent::Choice { index: *index }
                }
                ReplayIntent::Axis { name, value } => {
                    variant_name = "axis".into();
                    UserIntent::Axis {
                        name: name.clone(),
                        value: *value,
                    }
                }
                ReplayIntent::Wait => {
                    variant_name = "wait".into();
                    UserIntent::Wait
                }
                ReplayIntent::Select { target } => {
                    variant_name = "select".into();
                    // Resolve against the identity index (I1): StableId
                    // first (unnamed entities), then Name. Stable across
                    // resets, unlike raw entity ids.
                    let resolved: Option<bevy::ecs::entity::Entity> = match &target {
                        crate::scenario::SelectTarget::Stable { stable_id } => idx
                            .by_stable(crate::identity::StableId(*stable_id))
                            .filter(|e| q_named.get(*e).is_ok()),
                        crate::scenario::SelectTarget::Name(name) => {
                            idx.by_name(name).filter(|e| q_named.get(*e).is_ok())
                        }
                    };
                    match resolved {
                        Some(entity) => UserIntent::Select { target: entity },
                        None => {
                            // Loud miss: a replay referencing a missing
                            // target is a contract break, not a skip.
                            let display = match &target {
                                crate::scenario::SelectTarget::Name(n) => n.clone(),
                                crate::scenario::SelectTarget::Stable { stable_id } => {
                                    format!("#{}", stable_id)
                                }
                            };
                            violations.report(
                                "replay_target_missing",
                                &display,
                                format!(
                                    "frame {}: no Gameplay entity for target '{}'",
                                    entry.frame, display
                                ),
                                state.frame,
                            );
                            continue;
                        }
                    }
                }
            };
            state
                .coverage
                .intents_emitted
                .entry(variant_name)
                .and_modify(|c| *c += 1)
                .or_insert(1);
            violations.set_context(format!(
                "replay:frame={},intent={:?}",
                entry.frame, entry.intent
            ));
            intents.write(intent);
        }
    }
}

/// One in-flight click gesture for the synthetic_pointer bot: press
/// and release frames scheduled relative to the initiating move.
#[derive(Clone)]
pub(crate) struct PendingGesture {
    press_at: u64,
    pub(crate) release_at: u64,
    location: bevy::picking::pointer::Location,
    button: PointerButton,
    pub(crate) target_name: String,
    require_actionable: bool,
}

/// synthetic_pointer: emits RAW `PointerInput` events (Move → Press →
/// Release) at a named entity's projected viewport position. Unlike
/// intent-injection bots (chaos/replay), this drives the game through
/// its real input chain: pointer input → picking backend raycast →
/// `PointerClick` message → the game's own input adapter → UserIntent.
/// A game whose picking wiring is broken (e.g. missing
/// MeshPickingPlugin) produces clicks that hit nothing — caught by the
/// scenario's invariants, not silently skipped.
#[allow(clippy::too_many_arguments)]
pub(crate) fn synthetic_pointer_bot_system(
    mut state: ResMut<PlaytestState>,
    mut violations: ResMut<Violations>,
    mut pointer_inputs: MessageWriter<PointerInput>,
    q_named: Query<(&Name, &GlobalTransform), With<Gameplay>>,
    q_camera: Query<(&Camera, &GlobalTransform)>,
    primary_window: Query<Entity, With<bevy::window::PrimaryWindow>>,
    scenario: Res<ScenarioResource>,
    idx: Res<crate::identity::IdentityIndex>,
) {
    // Deliver pending gesture halves scheduled for this frame.
    let frame = state.frame;
    let mut still_pending = std::collections::VecDeque::new();
    while let Some(mut g) = state.pending_gestures.pop_front() {
        if g.press_at == frame {
            pointer_inputs.write(PointerInput::new(
                PointerId::Mouse,
                g.location.clone(),
                PointerAction::Press(g.button),
            ));
        }
        if g.release_at == frame {
            pointer_inputs.write(PointerInput::new(
                PointerId::Mouse,
                g.location.clone(),
                PointerAction::Release(g.button),
            ));
            // ACTIONABILITY GATE (opt-in): after the gesture the
            // picking backend must show the pointer over SOME entity —
            // proof the click went through live picking wiring, not
            // into the void. Checked one frame AFTER release (hover
            // maps lag input by a frame).
            if g.require_actionable {
                state.pending_actionability_checks.push_back(g.clone());
            }
            state
                .coverage
                .intents_emitted
                .entry(format!("pointer_click:{}", g.target_name))
                .and_modify(|c| *c += 1)
                .or_insert(1);
            violations.set_context(format!(
                "synthetic_pointer:frame={},target={}",
                frame, g.target_name
            ));
            continue; // gesture complete
        }
        // Not yet fully delivered — keep waiting. A gesture that has
        // already pressed but not released stays queued.
        if g.press_at > frame {
            g.press_at = frame; // avoid double-press on future loops
        }
        still_pending.push_back(g);
    }
    state.pending_gestures = still_pending;
    for click in &scenario.0.bot.pointer_clicks {
        if click.frame != state.frame {
            continue;
        }
        let button = match click.button {
            crate::enums::PointerButton::Primary => PointerButton::Primary,
            crate::enums::PointerButton::Secondary => PointerButton::Secondary,
            crate::enums::PointerButton::Middle => PointerButton::Middle,
        };
        let Some(entity) = idx.by_name(&click.target) else {
            violations.report(
                "synthetic_pointer_config",
                &click.target,
                "click target not found in identity index".to_string(),
                state.frame,
            );
            continue;
        };
        let Ok((_, transform)) = q_named.get(entity) else {
            violations.report(
                "synthetic_pointer_config",
                &click.target,
                "click target not found among Gameplay entities".to_string(),
                state.frame,
            );
            continue;
        };
        let Ok(window_entity) = primary_window.single() else {
            violations.report(
                "synthetic_pointer_config",
                &click.target,
                "no primary window entity — cannot synthesize pointer events".to_string(),
                state.frame,
            );
            continue;
        };
        // Project the entity's world position to viewport coords using
        // the first camera that can (2D boards typically have exactly one).
        let mut viewport_pos = None;
        for (camera, cam_transform) in q_camera.iter() {
            if let Ok(p) = camera.world_to_viewport(cam_transform, transform.translation()) {
                viewport_pos = Some(p);
                break;
            }
        }
        let Some(position) = viewport_pos else {
            violations.report(
                "synthetic_pointer_config",
                &click.target,
                "no camera could project the target to viewport coordinates".to_string(),
                state.frame,
            );
            continue;
        };
        let Some(target) =
            bevy::camera::RenderTarget::Window(bevy::window::WindowRef::Entity(window_entity))
                .normalize(Some(window_entity))
        else {
            violations.report(
                "synthetic_pointer_config",
                &click.target,
                "primary window render-target normalization failed".to_string(),
                state.frame,
            );
            continue;
        };
        let location = bevy::picking::pointer::Location { target, position };
        // A click is a THREE-FRAME gesture: Move, then Press, then Release.
        // Rationale: bevy_picking's release/click dispatch reads the
        // PREVIOUS frame's hover map (previous_hover_map), so a press
        // and release in the same frame as the move finds no prior
        // hover and silently drops the click. Spreading the gesture
        // across frames mirrors real mouse timing.
        pointer_inputs.write(PointerInput::new(
            PointerId::Mouse,
            location.clone(),
            PointerAction::Move {
                delta: bevy::math::Vec2::ZERO,
            },
        ));
        // Pending press/release delivered on subsequent frames via
        // SPBGesture state.
        let f = state.frame;
        state.pending_gestures.push_back(PendingGesture {
            press_at: f + 1,
            release_at: f + 1 + click.hold_frames.max(1),
            location,
            button,
            target_name: click.target.clone(),
            require_actionable: click.require_actionable,
        });
    }
}

/// pursuit: resolves agent/target by Name (Gameplay + Transform required),
/// emits Move intents toward the target with a deadzone. Fails LOUDLY —
/// a bot issuing NO input is itself a violation, not a silent fallback.
/// Spatial games only; that's inherent to pursuing.
pub(crate) fn pursuit_bot_system(
    state: ResMut<PlaytestState>,
    mut violations: ResMut<Violations>,
    mut intents: MessageWriter<UserIntent>,
    q: Query<(&Name, &GlobalTransform), With<Gameplay>>,
    scenario: Res<ScenarioResource>,
    idx: Res<crate::identity::IdentityIndex>,
) {
    let bot = &scenario.0.bot;
    let (Some(agent_name), Some(target_name)) = (&bot.agent_target, &bot.target) else {
        violations.report(
            "pursuit_bot_config",
            "",
            "pursuit bot requires agent_target and target".to_string(),
            state.frame,
        );
        return;
    };
    let agent = idx.by_name(agent_name);
    let target = idx.by_name(target_name);
    let agent = agent.and_then(|e| q.get(e).ok());
    let target = target.and_then(|e| q.get(e).ok());
    let (Some((_, agent_t)), Some((_, target_t))) = (agent, target) else {
        violations.report(
            "pursuit_bot_config",
            agent_name,
            format!(
                "agent '{}' or target '{}' not found — bot issued NO input. Fix agent_target/target; do not treat this run as a valid pursuit exercise.",
                agent_name, target_name
            ),
            state.frame,
        );
        return;
    };

    let dx = target_t.translation().x - agent_t.translation().x;
    let dy = target_t.translation().y - agent_t.translation().y;
    if dx.abs() > bot.deadzone || dy.abs() > bot.deadzone {
        violations.set_context(format!(
            "pursuit:agent={} target={} dist=({:.1},{:.1})",
            agent_name, target_name, dx, dy
        ));
        intents.write(UserIntent::Move {
            dir: bevy::math::Vec2::new(dx, dy).normalize_or_zero(),
        });
    }
}

pub(crate) fn tick_counter_system(world: &mut bevy::prelude::World) {
    let tick = world.read_change_tick().get();
    let mut state = world.resource_mut::<PlaytestState>();
    state.frame += 1;
    state.metrics.frame_count += 1;
    // FX1: snapshot the change tick BEFORE any schedule runs this frame.
    // The liveness oracle compares against the tick from N FRAMES ago,
    // not N ticks ago (the tick counter advances per system run).
    state.frame_start_ticks.push_back(tick);
    if state.frame_start_ticks.len() > 600 {
        state.frame_start_ticks.pop_front();
    }
    // FX1: also snapshot for effect windows (I5).
    let frame_now = state.frame;
    state.effect_window_snapshots.push((frame_now, tick));
    if state.effect_window_snapshots.len() > 600 {
        state.effect_window_snapshots.remove(0);
    }
}

// ---------------------------------------------------------------------------
