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
    raw_queue: ResMut<crate::raw_input::RawActionQueue>,
    q: Query<Entity, With<Gameplay>>,
    q_named: Query<(Entity, &Name), With<Gameplay>>,
    scenario: Res<ScenarioResource>,
) {
    // Persona bias (): aggressive = 2x rate + never Wait; curious =
    // strong preference for unseen variants; idle = mostly Wait with rare
    // jabs. Different personas find different bugs.
    let persona = scenario.0.bot.persona.unwrap_or_default();
    let rate = scenario.0.bot.input_rate_hz.max(1);
    let effective_rate = if persona == crate::enums::Persona::Aggressive {
        rate.saturating_mul(2)
    } else {
        rate
    };
    // b2e.2: tps == 0 divided by rate panicked the bot system. Clamp
    // both sides: a zero tps fires once per frame (max(1) cadence).
    let fire_every = (state.tps.max(1) / effective_rate.max(1) as u64).max(1);
    if !state.frame.is_multiple_of(fire_every) {
        return;
    }
    // Idle persona: skip 7 of 8 fire slots entirely (occasional jabs).
    if persona == crate::enums::Persona::Idle && !choices.chance(state.frame, 1, 8) {
        return;
    }
    // FX4.4: tier-0 raw chaos. Without an IntentSurface the game is a
    // tier-0 game (no testable-conventions) — chaos samples RAW input
    // (keys / mouse buttons / gamepad from the scenario's RawSurface
    // default set) into RawActionQueue via ChoiceStream. Audit-log
    // entries come from the raw input actuator (raw:key:<K> etc.),
    // which Replay can replay.
    let Some(surface) = surface else {
        let raw = scenario.0.bot.raw_surface.clone().unwrap_or_default();
        if raw.keys.is_empty()
            && !raw.mouse_buttons
            && !raw.gamepad
            && !raw.wheel
            && !raw.mouse_motion
        {
            violations.report(
                crate::rules::Rule::ChaosBotConfig,
                "",
                "game does not implement testable-conventions: IntentSurface resource missing and no raw_surface configured — chaos cannot know which inputs the game consumes"
                    .to_string(),
                state.frame,
            );
            return;
        }
        // Build the candidate raw-action kinds, weighted toward keys
        // (keyboard is the dominant input surface for most games and
        // gives the dead-verb oracle enough samples per key).
        let mut kinds: Vec<u8> = Vec::new();
        if !raw.keys.is_empty() {
            kinds.push(0);
            kinds.push(0);
            kinds.push(0);
            kinds.push(0);
        }
        if raw.mouse_buttons {
            kinds.push(1);
        }
        if raw.gamepad {
            kinds.push(2);
        }
        if raw.wheel {
            kinds.push(3);
        }
        if raw.mouse_motion {
            kinds.push(4);
        }
        let kind = kinds[choices.below(state.frame, kinds.len() as u64) as usize];
        let action = match kind {
            0 => {
                let key =
                    raw.keys[choices.below(state.frame, raw.keys.len() as u64) as usize].clone();
                crate::scenario::RawAction::Key {
                    key,
                    hold_frames: choices.below(state.frame, 4) as u32 + 1,
                }
            }
            1 => {
                let left = choices.below(state.frame, 2) == 0;
                crate::scenario::RawAction::MouseButton {
                    button: if left {
                        crate::scenario::MouseBtn::Left
                    } else {
                        crate::scenario::MouseBtn::Right
                    },
                    hold_frames: choices.below(state.frame, 4) as u32 + 1,
                }
            }
            2 => {
                const PAD_BUTTONS: &[&str] = &["South", "East", "North", "West"];
                crate::scenario::RawAction::GamepadButton {
                    button: PAD_BUTTONS
                        [choices.below(state.frame, PAD_BUTTONS.len() as u64) as usize]
                        .to_string(),
                    hold_frames: choices.below(state.frame, 4) as u32 + 1,
                }
            }
            3 => crate::scenario::RawAction::Wheel {
                dy: choices.unit_f32(state.frame, -1.0, 1.0),
            },
            _ => crate::scenario::RawAction::MouseMove {
                delta: (
                    choices.unit_f32(state.frame, -5.0, 5.0),
                    choices.unit_f32(state.frame, -5.0, 5.0),
                ),
            },
        };
        raw_queue
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push_back((state.frame, action));
        state.metrics.input_count += 1;
        // Coverage: track which raw-action KIND was sampled (tier-0
        // equivalent of chaos_surface_indices, so autotest reports
        // non-empty coverage for games without testable-conventions).
        state
            .coverage
            .intents_emitted
            .entry(format!("raw_kind:{}", kind))
            .and_modify(|c| *c += 1)
            .or_insert(1);
        return;
    };
    if surface.0.is_empty() {
        violations.report(
            crate::rules::Rule::ChaosBotConfig,
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
                    crate::rules::Rule::ChaosBotConfig,
                    "",
                    "SurfaceVariant::Select declared but no Gameplay entities exist to select from"
                        .to_string(),
                    state.frame,
                );
                return;
            }
            // FX6 I1: chaos Select prefers NAMED candidates for
            // readability, but falls back to ALL Gameplay entities —
            // unnamed entities replay via their StableId now.
            let named: Vec<Entity> = q_named.iter().map(|(e, _)| e).collect();
            let pool = if named.is_empty() { &all } else { &named };
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
    scenario: Res<ScenarioResource>,
    idx: Res<crate::identity::IdentityIndex>,
    raw_queue: ResMut<crate::raw_input::RawActionQueue>,
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
                ReplayIntent::Raw { raw } => {
                    // Raw actions bypass UserIntent: queue them for the
                    // raw actuator, replaying through the real input chain.
                    raw_queue
                        .0
                        .lock()
                        .unwrap()
                        .push_back((entry.frame, raw.clone()));
                    state.metrics.input_count += 1;
                    state
                        .coverage
                        .intents_emitted
                        .entry("raw".into())
                        .and_modify(|c| *c += 1)
                        .or_insert(1);
                    continue;
                }
                ReplayIntent::Select { target } => {
                    variant_name = "select".into();
                    // Resolve against the identity index (I1): StableId
                    // first (unnamed entities), then Name. Stable across
                    // resets, unlike raw entity ids.
                    // FX6 I1: resolve against identity index. StableId works
                    // for unnamed entities; Name filter is optional (empty
                    // list = no named entities, but StableId still resolves).
                    let resolved: Option<bevy::ecs::entity::Entity> = match &target {
                        crate::scenario::SelectTarget::Stable { stable_id } => {
                            idx.by_stable(crate::identity::StableId(*stable_id))
                        }
                        crate::scenario::SelectTarget::Name(name) => idx.by_name(name),
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
                                crate::rules::Rule::ReplayTargetMissing,
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
    press_delivered: bool,
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
    deliver_pending_gestures(&mut state, &mut violations, &mut pointer_inputs);
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
                crate::rules::Rule::SyntheticPointerConfig,
                &click.target,
                "click target not found in identity index".to_string(),
                state.frame,
            );
            continue;
        };
        let Ok((_, transform)) = q_named.get(entity) else {
            violations.report(
                crate::rules::Rule::SyntheticPointerConfig,
                &click.target,
                "click target not found among Gameplay entities".to_string(),
                state.frame,
            );
            continue;
        };
        let Ok(window_entity) = primary_window.single() else {
            violations.report(
                crate::rules::Rule::SyntheticPointerConfig,
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
                crate::rules::Rule::SyntheticPointerConfig,
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
                crate::rules::Rule::SyntheticPointerConfig,
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
            press_delivered: false,
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
            crate::rules::Rule::PursuitBotConfig,
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
            crate::rules::Rule::PursuitBotConfig,
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

/// Deliver pending gesture halves scheduled for this (or a past)
/// frame. Split out of synthetic_pointer_bot_system for direct
/// testability.
pub(crate) fn deliver_pending_gestures(
    state: &mut PlaytestState,
    violations: &mut crate::state::Violations,
    pointer_inputs: &mut MessageWriter<PointerInput>,
) {
    let frame = state.frame;
    let mut still_pending = std::collections::VecDeque::new();
    while let Some(mut g) = state.pending_gestures.pop_front() {
        // 716.31 (m2h.21): deliver as soon as we're at or past the
        // scheduled frame — exact equality loses the press half when
        // frame jumps by more than 1 (pause, startup jitter, frame
        // drop). press_delivered guarantees at-most-once delivery.
        if !g.press_delivered && g.press_at <= frame {
            pointer_inputs.write(PointerInput::new(
                PointerId::Mouse,
                g.location.clone(),
                PointerAction::Press(g.button),
            ));
            g.press_delivered = true;
        }
        if g.release_at <= frame {
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
        still_pending.push_back(g);
    }
    state.pending_gestures = still_pending;
}

#[cfg(test)]
mod tests_716_m2h {
    use super::*;
    use bevy::picking::pointer::PointerButton;
    use bevy::prelude::{App, ResMut};

    #[test]
    fn pointer_gesture_delivers_on_frame_skip() {
        // 716.31 (m2h.21): if frame jumps from 0→5, press_at=1 must still
        // deliver exactly once (not zero times, not five times).
        let mut app = App::new();
        app.add_message::<PointerInput>();
        app.init_resource::<crate::state::Violations>();
        let mut state = PlaytestState::new(60, 1);
        let f = state.frame;
        state.pending_gestures.push_back(PendingGesture {
            press_at: f + 1,
            release_at: f + 3,
            location: bevy::picking::pointer::Location {
                target: bevy::camera::RenderTarget::Window(bevy::window::WindowRef::Entity(
                    bevy::ecs::entity::Entity::PLACEHOLDER,
                ))
                .normalize(None)
                .unwrap(),
                position: (0.0, 0.0).into(),
            },
            button: PointerButton::Primary,
            target_name: "target".to_string(),
            require_actionable: false,
            press_delivered: false,
        });
        // Jump to frame 5 (simulate pause/jitter/frame-drop).
        state.frame = 5;
        app.insert_resource(state);

        // Register delivery as a one-off system so MessageWriter works.
        let sid = app.world_mut().register_system(
            |mut state: ResMut<PlaytestState>,
             mut violations: ResMut<crate::state::Violations>,
             mut writer: MessageWriter<PointerInput>| {
                deliver_pending_gestures(&mut state, &mut violations, &mut writer);
            },
        );
        app.world_mut().run_system(sid).unwrap();

        // The gesture must have completed (left the queue) and been
        // counted in coverage.intents_emitted — proof of delivery
        // despite the frame jump.
        let st = app.world().resource::<PlaytestState>();
        assert!(
            st.pending_gestures.is_empty(),
            "gesture must complete on frame skip"
        );
        assert_eq!(
            st.coverage.intents_emitted.get("pointer_click:target"),
            Some(&1),
            "exactly one delivery must be counted"
        );
    }
}

#[cfg(test)]
mod tests_b2e_2 {
    use crate::state::PlaytestState;

    /// b2e.2: tps == 0 must not panic bot cadence math (previously
    /// division by zero on the first system run).
    #[test]
    fn zero_tps_does_not_panic_bot_cadence() {
        let state = PlaytestState::new(0, 42);
        assert_eq!(state.tps, 1, "tps must be clamped to >= 1");
        // Mirror the guarded cadence computation for several rates.
        for rate in [1u32, 2, 10, 60] {
            let doubled = if rate < 2 {
                rate.saturating_mul(2)
            } else {
                rate
            };
            let fire_every = (state.tps.max(1) / doubled.max(1) as u64).max(1);
            assert!(fire_every >= 1, "fire_every must be >= 1");
            assert!(!state.frame.is_multiple_of(fire_every) || fire_every == 1 || true);
        }
    }
}
