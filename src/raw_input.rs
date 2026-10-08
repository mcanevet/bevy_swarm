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
use bevy::window::{CursorMoved, WindowEvent};
use std::collections::{HashMap, VecDeque};

use crate::scenario::{MouseBtn, RawAction};
use crate::state::{PlaytestState, Violations};

/// Queue of raw actions to inject (populated by chaos bot or replay).
/// Interior mutability (`Mutex`) so sibling systems can enqueue without
/// conflicting borrows; target frame allows scheduling ahead of time.
#[derive(bevy::ecs::prelude::Resource, Default)]
pub struct RawActionQueue(pub std::sync::Mutex<VecDeque<(u64, RawAction)>>);

/// Active key holds: (frame when pressed, KeyCode) -> release frame.
#[derive(bevy::ecs::prelude::Resource, Default)]
pub struct ActiveKeyHolds(pub HashMap<(u64, KeyCode), u64>);

/// Active mouse button holds: (frame when pressed, button) -> release frame.
#[derive(bevy::ecs::prelude::Resource, Default)]
pub struct ActiveMouseHolds(pub HashMap<(u64, WindowMouseButton), u64>);

/// Virtual gamepad entity, spawned lazily on the first gamepad action (Z3).
#[derive(bevy::ecs::prelude::Resource, Default)]
pub struct VirtualGamepad(pub Option<bevy::ecs::entity::Entity>);

/// Active gamepad button holds: (frame when pressed, button) -> release frame.
#[cfg(feature = "gamepad")]
#[derive(bevy::ecs::prelude::Resource, Default)]
pub struct ActiveGamepadHolds(pub HashMap<(u64, bevy::input::gamepad::GamepadButton), u64>);

#[cfg(feature = "gamepad")]
fn ensure_virtual_gamepad(
    virtual_gamepad: &mut VirtualGamepad,
    commands: &mut bevy::ecs::system::Commands,
) -> bevy::ecs::entity::Entity {
    *virtual_gamepad.0.get_or_insert_with(|| {
        commands
            .spawn(bevy::input::gamepad::Gamepad::default())
            .id()
    })
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
    mut key_holds: ResMut<ActiveKeyHolds>,
    mut mouse_holds: ResMut<ActiveMouseHolds>,
    #[cfg(feature = "gamepad")] mut gamepad_holds: ResMut<ActiveGamepadHolds>,
    mut keyboard_events: MessageWriter<KeyboardInput>,
    mut mouse_button_events: MessageWriter<MouseButtonInput>,
    mut window_events: MessageWriter<WindowEvent>,
    mut violations: ResMut<Violations>,
    playtest_state: Option<ResMut<PlaytestState>>,
    mut action_effects: ResMut<crate::effects::ActionEffects>,
    primary_window: bevy::ecs::system::Query<
        bevy::ecs::entity::Entity,
        bevy::ecs::query::With<bevy::window::PrimaryWindow>,
    >,
    mut commands: bevy::ecs::system::Commands,
    mut virtual_gamepad: ResMut<VirtualGamepad>,
    #[cfg(feature = "gamepad")] mut raw_gamepad_connection_events: MessageWriter<
        bevy::input::gamepad::GamepadConnectionEvent,
    >,
    #[cfg(feature = "gamepad")] mut raw_gamepad_events: MessageWriter<
        bevy::input::gamepad::RawGamepadEvent,
    >,
) {
    let frame = playtest_state.as_ref().map(|s| s.frame).unwrap_or(0);
    let window_entity = primary_window
        .single()
        .ok()
        .unwrap_or(bevy::ecs::entity::Entity::PLACEHOLDER);
    #[cfg(not(feature = "gamepad"))]
    {
        // Params only exercised by the gamepad feature.
        let _ = (&mut commands, &mut virtual_gamepad, &queue);
    }

    // Release holds whose release frame has arrived, then drop them.
    let expired_keys: Vec<(u64, KeyCode)> = key_holds
        .0
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
        key_holds.0.remove(&k);
    }
    let expired_buttons: Vec<(u64, WindowMouseButton)> = mouse_holds
        .0
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
        mouse_holds.0.remove(&k);
    }

    // Release expired gamepad button holds.
    #[cfg(feature = "gamepad")]
    {
        let expired_pads: Vec<((u64, bevy::input::gamepad::GamepadButton), u64)> = gamepad_holds
            .0
            .iter()
            .filter(|(_, release_frame)| **release_frame <= frame)
            .map(|(k, v)| (*k, *v))
            .collect();
        for ((press_frame, btn), _) in &expired_pads {
            if let Some(pad) = virtual_gamepad.0 {
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
            gamepad_holds.0.remove(&k);
        }
    }

    // Process queued actions for this frame.
    let mut still_pending = VecDeque::new();
    let mut deferred = VecDeque::new();
    while let Some((target_frame, action)) = queue.0.lock().unwrap().pop_front() {
        if target_frame > frame {
            still_pending.push_back((target_frame, action));
            continue;
        }
        match action {
            RawAction::Key { key, hold_frames } => {
                let Some(key_code) = parse_raw_key(&key) else {
                    violations.report(
                        "raw_key_invalid",
                        &key,
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
                key_holds
                    .0
                    .insert((frame, key_code), frame + hold_frames as u64);
                action_effects.record(&format!("raw:key:{:?}", key_code), frame);
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
                mouse_holds
                    .0
                    .insert((frame, btn), frame + hold_frames as u64);
                action_effects.record(&format!("raw:mouse:{:?}", btn), frame);
            }
            RawAction::Click {
                pos,
                button,
                hold_frames,
            } => {
                // Gesture: move cursor FIRST (same PreUpdate, before the
                // press — picking/Interaction readers see the press at the
                // clicked position), then press/hold/release.
                window_events.write(WindowEvent::CursorMoved(CursorMoved {
                    window: window_entity,
                    position: bevy::math::Vec2::new(pos.0, pos.1),
                    delta: None,
                }));
                let btn = parse_mouse_btn(button);
                write_mouse_button_input(
                    &mut mouse_button_events,
                    &mut window_events,
                    btn,
                    true,
                    window_entity,
                );
                mouse_holds
                    .0
                    .insert((frame, btn), frame + hold_frames as u64);
                action_effects.record(&format!("raw:mouse:{:?}", btn), frame);
            }
            RawAction::GamepadButton {
                button,
                hold_frames,
            } => {
                #[cfg(feature = "gamepad")]
                {
                    inject_gamepad_button(
                        &button,
                        hold_frames,
                        &mut violations,
                        &mut commands,
                        &mut virtual_gamepad,
                        frame,
                        &mut raw_gamepad_connection_events,
                        &mut raw_gamepad_events,
                        &mut gamepad_holds,
                    );
                    action_effects.record(&format!("raw:gamepad:{}", button), frame);
                }
            }
            RawAction::GamepadAxis {
                axis,
                value,
                hold_frames: _,
            } => {
                inject_gamepad_axis(
                    &axis,
                    value,
                    &mut violations,
                    &mut commands,
                    &mut virtual_gamepad,
                    frame,
                    #[cfg(feature = "gamepad")]
                    &mut raw_gamepad_connection_events,
                    #[cfg(feature = "gamepad")]
                    &mut raw_gamepad_events,
                );
            }
            RawAction::MouseMove { .. } | RawAction::Cursor { .. } | RawAction::Wheel { .. } => {
                // Handled in the Update pass.
                deferred.push_back((frame, action));
            }
            RawAction::ClickEntity { target } => {
                violations.report(
                    "raw_click_entity_unimplemented",
                    &format!("{target:?}"),
                    "ClickEntity resolution requires Z5/Z6 (UI monkey / entity inference)"
                        .to_string(),
                    frame,
                );
            }
            RawAction::Wait { frames: _ } => {}
        }
    }
    queue.0.lock().unwrap().extend(still_pending);
    queue.0.lock().unwrap().extend(deferred);
}

/// Inject mouse motion / cursor / wheel messages in `Update`
/// (alongside the other raw-input bots).
pub fn raw_input_update_system(
    queue: ResMut<RawActionQueue>,
    mut mouse_motion_events: MessageWriter<MouseMotion>,
    mut mouse_wheel_events: MessageWriter<MouseWheel>,
    mut window_events: MessageWriter<WindowEvent>,
    playtest_state: Option<ResMut<PlaytestState>>,
    primary_window: bevy::ecs::system::Query<
        bevy::ecs::entity::Entity,
        bevy::ecs::query::With<bevy::window::PrimaryWindow>,
    >,
) {
    let frame = playtest_state.as_ref().map(|s| s.frame).unwrap_or(0);
    let window_entity = primary_window
        .single()
        .unwrap_or(bevy::ecs::entity::Entity::PLACEHOLDER);
    let mut still_pending = VecDeque::new();
    while let Some((target_frame, action)) = queue.0.lock().unwrap().pop_front() {
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
            }
            RawAction::Cursor { pos } => {
                window_events.write(WindowEvent::CursorMoved(CursorMoved {
                    window: window_entity,
                    position: bevy::math::Vec2::new(pos.0, pos.1),
                    delta: None,
                }));
            }
            RawAction::Wheel { dy } => {
                mouse_wheel_events.write(MouseWheel {
                    unit: bevy::input::mouse::MouseScrollUnit::Line,
                    x: 0.0,
                    y: dy,
                    phase: bevy::input::touch::TouchPhase::Started,
                    window: bevy::ecs::entity::Entity::PLACEHOLDER,
                });
                window_events.write(WindowEvent::MouseWheel(MouseWheel {
                    unit: bevy::input::mouse::MouseScrollUnit::Line,
                    x: 0.0,
                    y: dy,
                    phase: bevy::input::touch::TouchPhase::Started,
                    window: window_entity,
                }));
            }
            _ => {
                // Not handled here; keep for the PreUpdate pass.
                still_pending.push_back((target_frame, action));
            }
        }
    }
    queue.0.lock().unwrap().extend(still_pending);
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
    gamepad_holds: &mut ActiveGamepadHolds,
) {
    let Some(btn) = parse_gamepad_button(button) else {
        violations.report(
            "raw_gamepad_button_invalid",
            button,
            "unknown button name".to_string(),
            frame,
        );
        return;
    };
    let pad = ensure_virtual_gamepad(virtual_gamepad, commands);
    connect_virtual_gamepad(pad, raw_gamepad_connection_events);
    raw_gamepad_events.write(bevy::input::gamepad::RawGamepadEvent::Button(
        RawGamepadButtonChangedEvent {
            gamepad: pad,
            button: btn,
            value: 1.0,
        },
    ));
    // Track for release after N frames.
    gamepad_holds
        .0
        .insert((frame, btn), frame + hold_frames as u64);
}

#[cfg(not(feature = "gamepad"))]
#[allow(clippy::too_many_arguments)]
fn inject_gamepad_button(
    button: &str,
    violations: &mut Violations,
    _commands: &mut bevy::ecs::system::Commands,
    _virtual_gamepad: &mut VirtualGamepad,
    frame: u64,
) {
    violations.report(
        "raw_gamepad_disabled",
        button,
        "gamepad feature not enabled".to_string(),
        frame,
    );
}

#[cfg(feature = "gamepad")]
#[allow(clippy::too_many_arguments)]
fn inject_gamepad_axis(
    axis: &str,
    value: f32,
    violations: &mut Violations,
    commands: &mut bevy::ecs::system::Commands,
    virtual_gamepad: &mut VirtualGamepad,
    frame: u64,
    raw_gamepad_connection_events: &mut MessageWriter<GamepadConnectionEvent>,
    raw_gamepad_events: &mut MessageWriter<bevy::input::gamepad::RawGamepadEvent>,
) {
    let Some(ax) = parse_gamepad_axis(axis) else {
        violations.report(
            "raw_gamepad_axis_invalid",
            axis,
            "unknown axis name".to_string(),
            frame,
        );
        return;
    };
    let pad = ensure_virtual_gamepad(virtual_gamepad, commands);
    connect_virtual_gamepad(pad, raw_gamepad_connection_events);
    raw_gamepad_events.write(bevy::input::gamepad::RawGamepadEvent::Axis(
        RawGamepadAxisChangedEvent {
            gamepad: pad,
            axis: ax,
            value,
        },
    ));
}

#[cfg(not(feature = "gamepad"))]
#[allow(clippy::too_many_arguments)]
fn inject_gamepad_axis(
    axis: &str,
    _value: f32,
    violations: &mut Violations,
    _commands: &mut bevy::ecs::system::Commands,
    _virtual_gamepad: &mut VirtualGamepad,
    frame: u64,
) {
    violations.report(
        "raw_gamepad_disabled",
        axis,
        "gamepad feature not enabled".to_string(),
        frame,
    );
}

#[cfg(feature = "gamepad")]
fn connect_virtual_gamepad(
    pad: bevy::ecs::entity::Entity,
    events: &mut MessageWriter<GamepadConnectionEvent>,
) {
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
