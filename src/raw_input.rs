//! Raw input actuator for tier-0 games (Z3).
//!
//! Injects keyboard and mouse events directly into the Bevy event system,
//! exercising the full input chain without requiring a UserIntent adapter.
//!
//! Per the Z3 amendments (verified 0.20 input facts): every mouse button,
//! wheel, and cursor event is written in BOTH forms — the standalone message
//! (`MouseButtonInput`, `MouseWheel`) AND the window event
//! (`WindowEvent::MouseButtonInput`, etc.) — because without winit nothing
//! forwards one into the other, picking reads window events, and
//! `ButtonInput` reads the standalone messages. Keyboard: standalone
//! `KeyboardInput` plus `WindowEvent::KeyboardInput`.
//!
//! Gamepad support is behind the `gamepad` crate feature (adds
//! `bevy/gamepad`): a virtual gamepad entity is spawned on first gamepad
//! action and driven with `GamepadConnectionEvent` /
//! `RawGamepadButtonChangedEvent` / `RawGamepadAxisChangedEvent`.

use bevy::ecs::entity::Entity;
use bevy::ecs::message::MessageWriter;
use bevy::ecs::system::ResMut;
#[cfg(feature = "gamepad")]
use bevy::input::gamepad::{
    GamepadConnection, GamepadConnectionEvent, RawGamepadAxisChangedEvent,
    RawGamepadButtonChangedEvent,
};
use bevy::input::keyboard::{KeyCode, KeyboardInput};
use bevy::input::mouse::{
    MouseButton as WindowMouseButton, MouseButtonInput, MouseMotion, MouseWheel,
};
use bevy::input::ButtonState;
use bevy::prelude::{Camera, GlobalTransform, Name};
use bevy::window::{CursorMoved, WindowEvent};
use std::collections::{HashMap, VecDeque};

use crate::scenario::{EntityRef, MouseBtn, RawAction};
use crate::state::{PlaytestState, Violations};

/// Queue of raw actions to inject (populated by chaos bot or replay).
/// Interior mutability (`Mutex`) so sibling systems can enqueue without
/// conflicting borrows; target frame allows scheduling ahead of time.
#[derive(bevy::ecs::prelude::Resource, Default)]
pub struct RawActionQueue(pub std::sync::Mutex<VecDeque<(u64, RawAction)>>);

/// Combined hold-tracking for keys, mouse buttons, and gamepad buttons
/// and axes: (frame when pressed, input) -> release frame.
#[derive(bevy::ecs::prelude::Resource, Default)]
pub struct InputHolds {
    pub keys: HashMap<(u64, KeyCode), u64>,
    pub mouse: HashMap<(u64, WindowMouseButton), u64>,
    #[cfg(feature = "gamepad")]
    pub gamepad_buttons: HashMap<(u64, bevy::input::gamepad::GamepadButton), u64>,
    #[cfg(feature = "gamepad")]
    pub gamepad_axes: HashMap<(u64, bevy::input::gamepad::GamepadAxis), u64>,
}

/// Virtual gamepad entity, spawned lazily on the first gamepad action (Z3).
#[derive(bevy::ecs::prelude::Resource, Default)]
pub struct VirtualGamepad {
    pub pad: Option<bevy::ecs::entity::Entity>,
    pub connected: bool,
}

#[cfg(feature = "gamepad")]
fn ensure_virtual_gamepad(
    virtual_gamepad: &mut VirtualGamepad,
    commands: &mut bevy::ecs::system::Commands,
) -> bevy::ecs::entity::Entity {
    *virtual_gamepad.pad.get_or_insert_with(|| {
        commands
            .spawn(bevy::input::gamepad::Gamepad::default())
            .id()
    })
}

/// Whether the harness owns the (headless) window it spawned. Games
/// that manage their own windows never see this flag — the fallback
/// window only appears when the raw actuator would otherwise target
/// `Entity::PLACEHOLDER`.
#[derive(bevy::ecs::prelude::Resource, Default)]
pub struct HeadlessWindow {
    pub entity: Option<bevy::ecs::entity::Entity>,
}

/// Resolve a primary-window entity, spawning a hidden fallback window
/// when the app has none (headless test apps that forgot a window).
/// The spawned entity gets `PrimaryWindow` so picking/UI focus route
/// to it; `HeadlessWindow` tracks ownership so we never duplicate.
fn ensure_window(
    windows: &mut bevy::ecs::system::Query<
        (bevy::ecs::entity::Entity, &mut bevy::window::Window),
        bevy::ecs::query::With<bevy::window::PrimaryWindow>,
    >,
    commands: &mut bevy::ecs::system::Commands,
    headless: &mut HeadlessWindow,
) -> bevy::ecs::entity::Entity {
    if let Some((e, _)) = windows.iter().next() {
        return e;
    }
    if let Some(e) = headless.entity {
        // Spawned last frame but the query has not observed it yet.
        if windows.get(e).is_ok() {
            return e;
        }
    }
    let e = commands
        .spawn((
            bevy::window::Window {
                // Headless fallback: the entity exists purely as a
                // routing target for input events (never opened — no
                // winit backend runs in test apps).
                visible: false,
                ..Default::default()
            },
            bevy::window::PrimaryWindow,
        ))
        .id();
    headless.entity = Some(e);
    e
}

/// Inject pending raw actions for the current frame.
///
/// Runs in `PreUpdate` after `PlaytestSet::Bots` and before Bevy's
/// `InputSystems`, so `ButtonInput` reflects injected keys same-frame
/// (A2 frame semantics). Handles keyboard, mouse buttons, gamepad, and
/// hold-release scheduling; defers cursor/motion/wheel to the Update
/// pass ([`raw_input_update_system`]).
#[allow(clippy::too_many_arguments)]
pub fn raw_input_preupdate_system(
    queue: ResMut<RawActionQueue>,
    mut holds: ResMut<InputHolds>,
    mut keyboard_events: MessageWriter<KeyboardInput>,
    mut mouse_button_events: MessageWriter<MouseButtonInput>,
    mut window_events: MessageWriter<WindowEvent>,
    mut cursor_events: Option<MessageWriter<CursorMoved>>,
    mut violations: ResMut<Violations>,
    playtest_state: Option<ResMut<PlaytestState>>,
    mut action_effects: ResMut<crate::effects::ActionEffects>,
    mut action_log: Option<ResMut<crate::contract::ActionLog>>,
    mut windows: bevy::ecs::system::Query<
        (bevy::ecs::entity::Entity, &mut bevy::window::Window),
        bevy::ecs::query::With<bevy::window::PrimaryWindow>,
    >,
    mut commands: bevy::ecs::system::Commands,
    mut virtual_gamepad: ResMut<VirtualGamepad>,
    mut headless_window: ResMut<HeadlessWindow>,
    #[cfg(feature = "gamepad")] mut raw_gamepad_connection_events: MessageWriter<
        bevy::input::gamepad::GamepadConnectionEvent,
    >,
    #[cfg(feature = "gamepad")] mut raw_gamepad_events: MessageWriter<
        bevy::input::gamepad::RawGamepadEvent,
    >,
) {
    let frame = playtest_state.as_ref().map(|s| s.frame).unwrap_or(0);
    let window_entity = ensure_window(&mut windows, &mut commands, &mut headless_window);
    #[cfg(not(feature = "gamepad"))]
    {
        // Params only exercised by the gamepad feature.
        let _ = (&mut commands, &mut virtual_gamepad, &queue);
    }

    // Release holds whose release frame has arrived, then drop them.
    let expired_keys: Vec<(u64, KeyCode)> = holds
        .keys
        .iter()
        .filter(|(_, release_frame)| **release_frame <= frame)
        .map(|(k, _)| *k)
        .collect();
    for (_, key_code) in &expired_keys {
        write_keyboard_input(
            &mut keyboard_events,
            &mut window_events,
            key_code,
            false,
            window_entity,
        );
    }
    for k in expired_keys {
        holds.keys.remove(&k);
    }
    let expired_buttons: Vec<(u64, WindowMouseButton)> = holds
        .mouse
        .iter()
        .filter(|(_, release_frame)| **release_frame <= frame)
        .map(|(k, _)| *k)
        .collect();
    for (_, btn) in &expired_buttons {
        write_mouse_button_input(
            &mut mouse_button_events,
            &mut window_events,
            *btn,
            false,
            window_entity,
        );
    }
    for k in expired_buttons {
        holds.mouse.remove(&k);
    }

    // Release expired gamepad button holds.
    #[cfg(feature = "gamepad")]
    {
        let expired_pads: Vec<((u64, bevy::input::gamepad::GamepadButton), u64)> = holds
            .gamepad_buttons
            .iter()
            .filter(|(_, release_frame)| **release_frame <= frame)
            .map(|(k, v)| (*k, *v))
            .collect();
        for ((press_frame, btn), _) in &expired_pads {
            if let Some(pad) = virtual_gamepad.pad {
                raw_gamepad_events.write(bevy::input::gamepad::RawGamepadEvent::Button(
                    RawGamepadButtonChangedEvent {
                        gamepad: pad,
                        button: *btn,
                        value: 0.0,
                    },
                ));
                let _ = press_frame;
            }
        }
        for (k, _) in expired_pads {
            holds.gamepad_buttons.remove(&k);
        }
    }

    // Neutralize expired gamepad axis holds.
    #[cfg(feature = "gamepad")]
    {
        let expired_axes: Vec<((u64, bevy::input::gamepad::GamepadAxis), u64)> = holds
            .gamepad_axes
            .iter()
            .filter(|(_, release_frame)| **release_frame <= frame)
            .map(|(k, v)| (*k, *v))
            .collect();
        for ((_, axis), _) in &expired_axes {
            if let Some(pad) = virtual_gamepad.pad {
                raw_gamepad_events.write(bevy::input::gamepad::RawGamepadEvent::Axis(
                    RawGamepadAxisChangedEvent {
                        gamepad: pad,
                        axis: *axis,
                        value: 0.0,
                    },
                ));
            }
        }
        for (k, _) in expired_axes {
            holds.gamepad_axes.remove(&k);
        }
    }

    // Record every injected raw action to the audit log (replayable via
    // ReplayIntent::Raw; details carry the lossless JSON form).
    let elapsed_ms = playtest_state
        .as_ref()
        .map(|s| s.elapsed_s() as f64 * 1000.0)
        .unwrap_or(0.0);
    let mut audit = |action: &str, raw: &RawAction| {
        if let Some(log) = action_log.as_mut() {
            log.entries.push(crate::contract::ActionEntry {
                frame,
                elapsed_ms,
                source: crate::contract::ActionSource::BotScenario("raw-input".into()),
                action: action.to_string(),
                details: Some(serde_json::to_string(raw).unwrap_or_default()),
            });
        }
    };

    // Process queued actions for this frame.
    let mut still_pending = VecDeque::new();
    let mut deferred = VecDeque::new();
    while let Some((target_frame, action)) = queue
        .0
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .pop_front()
    {
        if target_frame > frame {
            still_pending.push_back((target_frame, action));
            continue;
        }
        match action {
            RawAction::Key {
                ref key,
                hold_frames,
            } => {
                let Some(key_code) = parse_raw_key(key) else {
                    violations.report(
                        crate::rules::Rule::RawKeyInvalid,
                        key,
                        "unknown key name".to_string(),
                        frame,
                    );
                    continue;
                };
                write_keyboard_input(
                    &mut keyboard_events,
                    &mut window_events,
                    &key_code,
                    true,
                    window_entity,
                );
                holds
                    .keys
                    .insert((frame, key_code), frame + hold_frames as u64);
                action_effects.record(&format!("raw:key:{:?}", key_code), frame);
                audit(&format!("raw:key:{:?}", key_code), &action);
            }
            RawAction::MouseButton {
                button,
                hold_frames,
            } => {
                let btn = parse_mouse_btn(button);
                write_mouse_button_input(
                    &mut mouse_button_events,
                    &mut window_events,
                    btn,
                    true,
                    window_entity,
                );
                holds.mouse.insert((frame, btn), frame + hold_frames as u64);
                action_effects.record(&format!("raw:mouse:{:?}", btn), frame);
                audit(&format!("raw:mouse:{:?}", btn), &action);
            }
            RawAction::Click {
                pos,
                button,
                hold_frames,
            } => {
                // Gesture: move cursor FIRST (same PreUpdate, before the
                // press — picking/Interaction readers see the press at the
                // clicked position), then press/hold/release.
                if let Some(ref mut ce) = cursor_events {
                    move_cursor(
                        ce,
                        &mut window_events,
                        &mut windows,
                        bevy::math::Vec2::new(pos.0, pos.1),
                    );
                }
                let btn = parse_mouse_btn(button);
                write_mouse_button_input(
                    &mut mouse_button_events,
                    &mut window_events,
                    btn,
                    true,
                    window_entity,
                );
                holds.mouse.insert((frame, btn), frame + hold_frames as u64);
                action_effects.record(&format!("raw:mouse:{:?}", btn), frame);
                audit(&format!("raw:mouse:{:?}", btn), &action);
            }
            #[cfg_attr(not(feature = "gamepad"), allow(unused_variables))]
            RawAction::GamepadButton {
                ref button,
                hold_frames,
            } => {
                #[cfg(feature = "gamepad")]
                {
                    inject_gamepad_button(
                        button,
                        hold_frames,
                        &mut violations,
                        &mut commands,
                        &mut virtual_gamepad,
                        frame,
                        &mut raw_gamepad_connection_events,
                        &mut raw_gamepad_events,
                        &mut holds.gamepad_buttons,
                    );
                    action_effects.record(&format!("raw:gamepad:{}", button), frame);
                    // RF16: audit so gamepad chaos is replayable and
                    // minimizable.
                    audit(&format!("raw:gamepad:{}", button), &action);
                }
            }
            RawAction::GamepadAxis {
                ref axis,
                value,
                hold_frames,
            } => {
                #[cfg(feature = "gamepad")]
                {
                    inject_gamepad_axis(
                        axis,
                        value,
                        hold_frames,
                        &mut violations,
                        &mut commands,
                        &mut virtual_gamepad,
                        frame,
                        &mut raw_gamepad_connection_events,
                        &mut raw_gamepad_events,
                        &mut holds.gamepad_axes,
                    );
                    audit(&format!("raw:gamepad_axis:{}", axis), &action);
                }
                #[cfg(not(feature = "gamepad"))]
                {
                    inject_gamepad_axis(
                        axis,
                        value,
                        hold_frames,
                        &mut violations,
                        &mut commands,
                        &mut virtual_gamepad,
                        frame,
                    );
                }
            }
            RawAction::MouseMove { .. } | RawAction::Cursor { .. } | RawAction::Wheel { .. } => {
                // Handled in the Update pass.
                deferred.push_back((frame, action));
            }
            RawAction::ClickEntity { .. } => {
                // Resolved in the Update pass where camera and transform
                // queries are available (raw_input_click_entity_system).
                deferred.push_back((frame, action));
            }
            RawAction::Wait { frames: _ } => {}
        }
    }
    queue
        .0
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .extend(still_pending);
    queue
        .0
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .extend(deferred);
}

#[allow(clippy::too_many_arguments)]
/// Handle ClickEntity actions in Update (after cursor moves, before mouse press).
/// RF11: complex entity query supports both world-space entities
/// (GlobalTransform) and UI nodes (ComputedNode + UiGlobalTransform).
#[allow(clippy::type_complexity)]
pub fn raw_input_click_entity_system(
    queue: ResMut<RawActionQueue>,
    mut holds: ResMut<InputHolds>,
    mut mouse_button_events: MessageWriter<MouseButtonInput>,
    mut window_events: MessageWriter<WindowEvent>,
    mut cursor_events: Option<MessageWriter<CursorMoved>>,
    mut violations: ResMut<Violations>,
    playtest_state: Option<ResMut<PlaytestState>>,
    mut action_effects: ResMut<crate::effects::ActionEffects>,
    mut action_log: Option<ResMut<crate::contract::ActionLog>>,
    mut windows: bevy::ecs::system::Query<
        (bevy::ecs::entity::Entity, &mut bevy::window::Window),
        bevy::ecs::query::With<bevy::window::PrimaryWindow>,
    >,
    mut headless_window: ResMut<HeadlessWindow>,
    mut commands: bevy::ecs::system::Commands,
    entities: bevy::ecs::system::Query<(
        Entity,
        Option<&GlobalTransform>,
        Option<&Name>,
        Option<&crate::identity::StableId>,
        Option<&bevy::ui::ComputedNode>,
        Option<&bevy::ui::UiGlobalTransform>,
    )>,
    cameras: bevy::ecs::system::Query<(&Camera, &GlobalTransform)>,
) {
    let frame = playtest_state.as_ref().map(|s| s.frame).unwrap_or(0);
    let window_entity = ensure_window(&mut windows, &mut commands, &mut headless_window);

    let mut still_pending = VecDeque::new();
    while let Some((target_frame, action)) = queue
        .0
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .pop_front()
    {
        if target_frame > frame {
            still_pending.push_back((target_frame, action));
            continue;
        }
        match action {
            RawAction::ClickEntity { ref target } => {
                match resolve_click_position(&entities, &cameras, target) {
                    Ok(pos) => {
                        if let Some(ref mut ce) = cursor_events {
                            move_cursor(ce, &mut window_events, &mut windows, pos);
                        }
                        let btn = bevy::input::mouse::MouseButton::Left;
                        write_mouse_button_input(
                            &mut mouse_button_events,
                            &mut window_events,
                            btn,
                            true,
                            window_entity,
                        );
                        // RF11: schedule the release so a second ClickEntity
                        // registers a fresh just_pressed (and the button is
                        // not stuck held at run end).
                        holds.mouse.insert((frame, btn), frame + 1);
                        action_effects.record(&format!("raw:click_entity:{target:?}"), frame);
                        if let Some(log) = action_log.as_mut() {
                            log.entries.push(crate::contract::ActionEntry {
                                frame,
                                elapsed_ms: playtest_state
                                    .as_ref()
                                    .map(|s| s.elapsed_s() as f64 * 1000.0)
                                    .unwrap_or(0.0),
                                source: crate::contract::ActionSource::BotScenario(
                                    "raw-input".into(),
                                ),
                                action: format!("raw:click_entity:{target:?}"),
                                details: Some(serde_json::to_string(&action).unwrap_or_default()),
                            });
                        }
                    }
                    Err(reason) => {
                        violations.report(
                            crate::rules::Rule::RawClickTargetOffscreen,
                            &format!("{target:?}"),
                            reason,
                            frame,
                        );
                    }
                }
            }
            _ => {
                still_pending.push_back((target_frame, action));
            }
        }
    }
    queue
        .0
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .extend(still_pending);
}

/// Inject mouse motion / cursor / wheel messages in `Update`
/// (alongside the other raw-input bots).
#[allow(clippy::too_many_arguments)]
pub fn raw_input_update_system(
    queue: ResMut<RawActionQueue>,
    mut mouse_motion_events: MessageWriter<MouseMotion>,
    mut mouse_wheel_events: MessageWriter<MouseWheel>,
    mut window_events: MessageWriter<WindowEvent>,
    mut cursor_events: Option<MessageWriter<CursorMoved>>,
    playtest_state: Option<ResMut<PlaytestState>>,
    mut action_log: Option<ResMut<crate::contract::ActionLog>>,
    mut windows: bevy::ecs::system::Query<
        (bevy::ecs::entity::Entity, &mut bevy::window::Window),
        bevy::ecs::query::With<bevy::window::PrimaryWindow>,
    >,
    mut headless_window: ResMut<HeadlessWindow>,
    mut commands: bevy::ecs::system::Commands,
) {
    let frame = playtest_state.as_ref().map(|s| s.frame).unwrap_or(0);
    let window_entity = ensure_window(&mut windows, &mut commands, &mut headless_window);
    let elapsed_ms = playtest_state
        .as_ref()
        .map(|s| s.elapsed_s() as f64 * 1000.0)
        .unwrap_or(0.0);
    let mut audit = |action: &str, raw: &RawAction| {
        if let Some(log) = action_log.as_mut() {
            log.entries.push(crate::contract::ActionEntry {
                frame,
                elapsed_ms,
                source: crate::contract::ActionSource::BotScenario("raw-input".into()),
                action: action.to_string(),
                details: Some(serde_json::to_string(raw).unwrap_or_default()),
            });
        }
    };
    let mut still_pending = VecDeque::new();
    while let Some((target_frame, action)) = queue
        .0
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .pop_front()
    {
        if target_frame > frame {
            still_pending.push_back((target_frame, action));
            continue;
        }
        match action {
            RawAction::MouseMove { delta } => {
                mouse_motion_events.write(MouseMotion {
                    delta: bevy::math::Vec2::new(delta.0, delta.1),
                });
                window_events.write(WindowEvent::MouseMotion(MouseMotion {
                    delta: bevy::math::Vec2::new(delta.0, delta.1),
                }));
                audit("raw:mouse_motion", &action);
            }
            RawAction::Cursor { pos } => {
                if let Some(ref mut ce) = cursor_events {
                    move_cursor(
                        ce,
                        &mut window_events,
                        &mut windows,
                        bevy::math::Vec2::new(pos.0, pos.1),
                    );
                }
                audit("raw:cursor", &action);
            }
            RawAction::Wheel { dy } => {
                mouse_wheel_events.write(MouseWheel {
                    unit: bevy::input::mouse::MouseScrollUnit::Line,
                    x: 0.0,
                    y: dy,
                    phase: bevy::input::touch::TouchPhase::Started,
                    window: window_entity,
                });
                window_events.write(WindowEvent::MouseWheel(MouseWheel {
                    unit: bevy::input::mouse::MouseScrollUnit::Line,
                    x: 0.0,
                    y: dy,
                    phase: bevy::input::touch::TouchPhase::Started,
                    window: window_entity,
                }));
                audit("raw:wheel", &action);
            }
            _ => {
                // Not handled here; keep for the PreUpdate pass.
                still_pending.push_back((target_frame, action));
            }
        }
    }
    queue
        .0
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .extend(still_pending);
}

#[cfg(feature = "gamepad")]
#[allow(clippy::too_many_arguments)]
fn inject_gamepad_button(
    button: &str,
    hold_frames: u32,
    violations: &mut Violations,
    commands: &mut bevy::ecs::system::Commands,
    virtual_gamepad: &mut VirtualGamepad,
    frame: u64,
    raw_gamepad_connection_events: &mut MessageWriter<GamepadConnectionEvent>,
    raw_gamepad_events: &mut MessageWriter<bevy::input::gamepad::RawGamepadEvent>,
    gamepad_holds: &mut HashMap<(u64, bevy::input::gamepad::GamepadButton), u64>,
) {
    let Some(btn) = parse_gamepad_button(button) else {
        violations.report(
            crate::rules::Rule::RawGamepadButtonInvalid,
            button,
            "unknown button name".to_string(),
            frame,
        );
        return;
    };
    let pad = ensure_virtual_gamepad(virtual_gamepad, commands);
    connect_virtual_gamepad(
        pad,
        &mut virtual_gamepad.connected,
        raw_gamepad_connection_events,
    );
    raw_gamepad_events.write(bevy::input::gamepad::RawGamepadEvent::Button(
        RawGamepadButtonChangedEvent {
            gamepad: pad,
            button: btn,
            value: 1.0,
        },
    ));
    // Track for release after N frames.
    gamepad_holds.insert((frame, btn), frame + hold_frames as u64);
}

#[cfg(feature = "gamepad")]
#[allow(clippy::too_many_arguments)]
fn inject_gamepad_axis(
    axis: &str,
    value: f32,
    hold_frames: u32,
    violations: &mut Violations,
    commands: &mut bevy::ecs::system::Commands,
    virtual_gamepad: &mut VirtualGamepad,
    frame: u64,
    raw_gamepad_connection_events: &mut MessageWriter<GamepadConnectionEvent>,
    raw_gamepad_events: &mut MessageWriter<bevy::input::gamepad::RawGamepadEvent>,
    gamepad_axis_holds: &mut HashMap<(u64, bevy::input::gamepad::GamepadAxis), u64>,
) {
    let Some(ax) = parse_gamepad_axis(axis) else {
        violations.report(
            crate::rules::Rule::RawGamepadAxisInvalid,
            axis,
            "unknown axis name".to_string(),
            frame,
        );
        return;
    };
    let pad = ensure_virtual_gamepad(virtual_gamepad, commands);
    connect_virtual_gamepad(
        pad,
        &mut virtual_gamepad.connected,
        raw_gamepad_connection_events,
    );
    raw_gamepad_events.write(bevy::input::gamepad::RawGamepadEvent::Axis(
        RawGamepadAxisChangedEvent {
            gamepad: pad,
            axis: ax,
            value,
        },
    ));
    if hold_frames > 0 {
        gamepad_axis_holds.insert((frame, ax), frame + hold_frames as u64);
    }
}

#[cfg(not(feature = "gamepad"))]
#[allow(clippy::too_many_arguments)]
fn inject_gamepad_axis(
    axis: &str,
    _value: f32,
    _hold_frames: u32,
    violations: &mut Violations,
    _commands: &mut bevy::ecs::system::Commands,
    _virtual_gamepad: &mut VirtualGamepad,
    frame: u64,
) {
    violations.report(
        crate::rules::Rule::RawGamepadDisabled,
        axis,
        "gamepad feature not enabled".to_string(),
        frame,
    );
}

#[cfg(feature = "gamepad")]
fn connect_virtual_gamepad(
    pad: bevy::ecs::entity::Entity,
    already_connected: &mut bool,
    events: &mut MessageWriter<GamepadConnectionEvent>,
) {
    if *already_connected {
        return;
    }
    *already_connected = true;
    events.write(GamepadConnectionEvent {
        gamepad: pad,
        connection: GamepadConnection::Connected {
            name: "bevy_swarm_virtual".to_string(),
            vendor_id: None,
            product_id: None,
        },
    });
}

/// Write a key press/release in both forms: standalone `KeyboardInput`
/// and `WindowEvent::KeyboardInput` (see module docs).
/// Resolve a ClickEntity target to a viewport (logical) position.
/// Returns the screen position if the entity is visible through an
/// active camera; otherwise an error describing why not.
/// RF11: UI nodes are supported via ComputedNode + UiGlobalTransform
/// (they have no GlobalTransform, so the world-space projection path
/// can't find them); their center in logical pixels is used directly.
#[allow(clippy::type_complexity)]
fn resolve_click_position(
    entities: &bevy::ecs::system::Query<(
        Entity,
        Option<&GlobalTransform>,
        Option<&Name>,
        Option<&crate::identity::StableId>,
        Option<&bevy::ui::ComputedNode>,
        Option<&bevy::ui::UiGlobalTransform>,
    )>,
    cameras: &bevy::ecs::system::Query<(&Camera, &GlobalTransform)>,
    target: &EntityRef,
) -> Result<bevy::math::Vec2, String> {
    let (_, world_transform, _, _, computed_node, ui_transform) = match target {
        EntityRef::ByName(name) => entities
            .iter()
            .find(|(_, _, n, _, _, _)| n.is_some_and(|n| n.as_ref() == name))
            .ok_or_else(|| format!("no entity with Name {name:?}"))?,
        EntityRef::ByStableId(sid) => entities
            .iter()
            .find(|(_, _, _, s, _, _)| s.is_some_and(|s| s.0 == *sid))
            .ok_or_else(|| format!("no entity with StableId {sid:?}"))?,
    };
    // UI branch: node center in logical pixels (physical size scaled by
    // the inverse scale factor, offset by the UiGlobalTransform
    // translation which is in logical pixels).
    if let (Some(node), Some(ui_t)) = (computed_node, ui_transform) {
        let size_logical = node.size() * node.inverse_scale_factor;
        let center = ui_t.affine().translation + size_logical / 2.0;
        return Ok(center);
    }
    // World-space branch: project through the first active camera.
    let target_transform = world_transform
        .ok_or_else(|| "target has neither GlobalTransform nor UI components".to_string())?;
    let world_pos = target_transform.translation();
    let (camera, cam_transform) = cameras
        .iter()
        .next()
        .ok_or_else(|| "no active camera found".to_string())?;
    camera
        .world_to_viewport(cam_transform, world_pos)
        .map_err(|e| format!("projection failed: {e:?}"))
}

/// Move the cursor to a logical position (viewport coordinates).
/// Writes standalone CursorMoved, WindowEvent::CursorMoved, and sets
/// the window's physical_cursor_position (scaled by scale_factor).
fn move_cursor(
    cursor_events: &mut MessageWriter<CursorMoved>,
    window_events: &mut MessageWriter<WindowEvent>,
    windows: &mut bevy::ecs::system::Query<
        (bevy::ecs::entity::Entity, &mut bevy::window::Window),
        bevy::ecs::query::With<bevy::window::PrimaryWindow>,
    >,
    pos: bevy::math::Vec2,
) {
    let window_entity = windows
        .iter()
        .next()
        .map(|(e, _)| e)
        .unwrap_or(bevy::ecs::entity::Entity::PLACEHOLDER);
    cursor_events.write(CursorMoved {
        window: window_entity,
        position: pos,
        delta: None,
    });
    window_events.write(WindowEvent::CursorMoved(CursorMoved {
        window: window_entity,
        position: pos,
        delta: None,
    }));
    if let Some((_e, mut win)) = windows.iter_mut().next() {
        let scale = win.scale_factor();
        win.set_physical_cursor_position(Some(bevy::math::DVec2::new(
            (pos.x * scale) as f64,
            (pos.y * scale) as f64,
        )));
    }
}

fn write_keyboard_input(
    keyboard_events: &mut MessageWriter<KeyboardInput>,
    window_events: &mut MessageWriter<WindowEvent>,
    key_code: &KeyCode,
    pressed: bool,
    window: bevy::ecs::entity::Entity,
) {
    let state = if pressed {
        ButtonState::Pressed
    } else {
        ButtonState::Released
    };
    let input = KeyboardInput {
        key_code: *key_code,
        logical_key: bevy::input::keyboard::Key::Unidentified(
            bevy::input::keyboard::NativeKey::Unidentified,
        ),
        state,
        text: None,
        repeat: false,
        window,
    };
    keyboard_events.write(input.clone());
    window_events.write(WindowEvent::KeyboardInput(input));
}

/// Write a mouse button press/release in both forms.
fn write_mouse_button_input(
    mouse_button_events: &mut MessageWriter<MouseButtonInput>,
    window_events: &mut MessageWriter<WindowEvent>,
    btn: WindowMouseButton,
    pressed: bool,
    window: bevy::ecs::entity::Entity,
) {
    let state = if pressed {
        ButtonState::Pressed
    } else {
        ButtonState::Released
    };
    // Write BOTH the standalone message (consumed by InputPlugin) AND
    // the WindowEvent form (for games that only listen to WindowEvents).
    mouse_button_events.write(MouseButtonInput {
        button: btn,
        state,
        window,
    });
    window_events.write(WindowEvent::MouseButtonInput(MouseButtonInput {
        button: btn,
        state,
        window,
    }));
}

/// Convert a wire key name to `KeyCode` (shared with scenario validation).
fn parse_raw_key(name: &str) -> Option<KeyCode> {
    crate::harness::parse_key_code(name)
}

/// Convert the wire `MouseBtn` to `bevy::input::mouse::MouseButton`.
fn parse_mouse_btn(button: MouseBtn) -> WindowMouseButton {
    match button {
        MouseBtn::Left => WindowMouseButton::Left,
        MouseBtn::Right => WindowMouseButton::Right,
        MouseBtn::Middle => WindowMouseButton::Middle,
    }
}

/// Convert a wire gamepad button name to `GamepadButton` (0.20 plain enum).
#[cfg(feature = "gamepad")]
fn parse_gamepad_button(name: &str) -> Option<bevy::input::gamepad::GamepadButton> {
    use bevy::input::gamepad::GamepadButton as B;
    Some(match name.to_lowercase().as_str() {
        "south" | "a" | "cross" => B::South,
        "east" | "b" | "circle" => B::East,
        "north" | "y" | "triangle" => B::North,
        "west" | "x" | "square" => B::West,
        "c" => B::C,
        "z" => B::Z,
        "left_trigger" | "lb" => B::LeftTrigger,
        "left_trigger_2" | "lt" => B::LeftTrigger2,
        "right_trigger" | "rb" => B::RightTrigger,
        "right_trigger_2" | "rt" => B::RightTrigger2,
        "select" => B::Select,
        "start" => B::Start,
        "mode" => B::Mode,
        "left_thumb" | "left_stick" => B::LeftThumb,
        "right_thumb" | "right_stick" => B::RightThumb,
        "dpad_up" => B::DPadUp,
        "dpad_down" => B::DPadDown,
        "dpad_left" => B::DPadLeft,
        "dpad_right" => B::DPadRight,
        _ => return None,
    })
}

/// Convert a wire gamepad axis name to `GamepadAxis` (0.20 plain enum).
#[cfg(feature = "gamepad")]
fn parse_gamepad_axis(name: &str) -> Option<bevy::input::gamepad::GamepadAxis> {
    use bevy::input::gamepad::GamepadAxis as A;
    Some(match name.to_lowercase().as_str() {
        "left_x" | "leftstickx" => A::LeftStickX,
        "left_y" | "leftsticky" => A::LeftStickY,
        "left_z" | "leftz" => A::LeftZ,
        "right_x" | "rightstickx" => A::RightStickX,
        "right_y" | "rightsticky" => A::RightStickY,
        "right_z" | "rightz" => A::RightZ,
        _ => return None,
    })
}
