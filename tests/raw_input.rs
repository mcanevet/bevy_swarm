//! Z3: Raw input actuator tests (tier-0 games).

use bevy::prelude::*;
use bevy_swarm::contract::{IntentSurface, SurfaceVariant, TestConventionsPlugin};
use bevy_swarm::driver::PlaytestPlugin;
use bevy_swarm::harness::Gameplay;
use bevy_swarm::raw_input::RawActionQueue;
use bevy_swarm::scenario::{MouseBtn, RawAction};
use bevy_swarm::state::PlaytestState;

fn build_headless_game() -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        TransformPlugin,
        TestConventionsPlugin,
        PlaytestPlugin,
    ));
    // Initialize raw input messages
    app.add_message::<bevy::input::keyboard::KeyboardInput>();
    app.add_message::<bevy::input::mouse::MouseButtonInput>();
    app.add_message::<bevy::input::mouse::MouseMotion>();
    app.add_message::<bevy::input::mouse::MouseWheel>();
    app.add_message::<bevy::window::WindowEvent>();
    #[cfg(feature = "gamepad")]
    {
        app.add_message::<bevy::input::gamepad::GamepadConnectionEvent>();
        app.add_message::<bevy::input::gamepad::RawGamepadButtonChangedEvent>();
        app.add_message::<bevy::input::gamepad::RawGamepadAxisChangedEvent>();
    }
    app.insert_resource(IntentSurface::new(vec![
        SurfaceVariant::Choice(3),
        SurfaceVariant::Wait,
    ]));
    app.world_mut().spawn((
        Name::new("Player"),
        Gameplay,
        Transform::from_xyz(0.0, 0.0, 0.0),
    ));
    // Add a primary window for raw input routing
    app.world_mut()
        .spawn((bevy::window::Window::default(), bevy::window::PrimaryWindow));
    app
}

#[test]
fn raw_key_injection_runs_without_panicking() {
    let mut app = build_headless_game();

    // Queue a key press for frame 0, held for 2 frames
    app.world_mut()
        .resource_mut::<RawActionQueue>()
        .0
        .lock()
        .unwrap()
        .push_back((
            0,
            RawAction::Key {
                key: "ArrowRight".to_string(),
                hold_frames: 2,
            },
        ));

    // Set frame to 0
    let mut state = PlaytestState::new(60, 42);
    state.frame = 0;
    app.world_mut().insert_resource(state);

    // Run PreUpdate (frame 0: press) - should not panic
    app.update();

    // Advance to frame 1 (still holding)
    app.world_mut().resource_mut::<PlaytestState>().frame = 1;
    app.update();

    // Advance to frame 2 (release) - should not panic
    app.world_mut().resource_mut::<PlaytestState>().frame = 2;
    app.update();
}

#[test]
fn raw_mouse_button_injection_runs_without_panicking() {
    let mut app = build_headless_game();

    // Queue a mouse button press for frame 0, held for 1 frame
    app.world_mut()
        .resource_mut::<RawActionQueue>()
        .0
        .lock()
        .unwrap()
        .push_back((
            0,
            RawAction::MouseButton {
                button: MouseBtn::Left,
                hold_frames: 1,
            },
        ));

    let mut state = PlaytestState::new(60, 42);
    state.frame = 0;
    app.world_mut().insert_resource(state);

    // Run PreUpdate (frame 0: press) - should not panic
    app.update();

    // Advance to frame 1 (release) - should not panic
    app.world_mut().resource_mut::<PlaytestState>().frame = 1;
    app.update();
}

#[test]
fn raw_cursor_movement_runs_without_panicking() {
    let mut app = build_headless_game();

    // Queue cursor move for frame 0
    app.world_mut()
        .resource_mut::<RawActionQueue>()
        .0
        .lock()
        .unwrap()
        .push_back((
            0,
            RawAction::Cursor {
                pos: (100.0, 200.0),
            },
        ));

    let mut state = PlaytestState::new(60, 42);
    state.frame = 0;
    app.world_mut().insert_resource(state);

    // Run Update (cursor moved in Update) - should not panic
    app.update();
}

#[test]
fn raw_click_entity_errors_on_missing_resolution() {
    let mut app = build_headless_game();

    // Queue ClickEntity (requires Z5/Z6 resolution)
    app.world_mut()
        .resource_mut::<RawActionQueue>()
        .0
        .lock()
        .unwrap()
        .push_back((
            0,
            RawAction::ClickEntity {
                target: bevy_swarm::scenario::EntityRef::ByName("Missing".to_string()),
            },
        ));

    let mut state = PlaytestState::new(60, 42);
    state.frame = 0;
    app.world_mut().insert_resource(state);

    // Run PreUpdate - should report violation but not panic
    app.update();

    // Check that a violation was reported
    let violations = app.world().resource::<bevy_swarm::state::Violations>();
    assert!(violations
        .entries
        .iter()
        .any(|(_, e)| e.rule == "raw_click_entity_unimplemented"));
}

#[cfg(feature = "gamepad")]
#[test]
fn raw_gamepad_axis_moves_player() {
    let mut app = build_headless_game();

    // Queue gamepad axis action for frame 0
    app.world_mut()
        .resource_mut::<RawActionQueue>()
        .0
        .lock()
        .unwrap()
        .push_back((
            0,
            RawAction::GamepadAxis {
                axis: "left_x".to_string(),
                value: 0.5,
                hold_frames: 0,
            },
        ));

    let mut state = PlaytestState::new(60, 42);
    state.frame = 0;
    app.world_mut().insert_resource(state);

    // Should not panic, and should connect a virtual gamepad
    app.update();

    // Verify a Gamepad entity was spawned
    let mut gamepads = app
        .world_mut()
        .query_filtered::<Entity, With<bevy::input::gamepad::Gamepad>>();
    assert!(
        gamepads.iter(app.world()).count() >= 1,
        "virtual gamepad should be spawned"
    );
}

#[cfg(not(feature = "gamepad"))]
#[test]
fn raw_gamepad_reports_disabled_violation() {
    let mut app = build_headless_game();

    app.world_mut()
        .resource_mut::<RawActionQueue>()
        .0
        .lock()
        .unwrap()
        .push_back((
            0,
            RawAction::GamepadAxis {
                axis: "left_x".to_string(),
                value: 0.5,
                hold_frames: 0,
            },
        ));

    let mut state = PlaytestState::new(60, 42);
    state.frame = 0;
    app.world_mut().insert_resource(state);

    app.update();

    let violations = app.world().resource::<bevy_swarm::state::Violations>();
    assert!(violations
        .entries
        .iter()
        .any(|(_, e)| e.rule == "raw_gamepad_disabled"));
}

// ---------------------------------------------------------------------------
// FX8: state-effect tests (RED on main — raw input wrote events Bevy's
// input systems never consumed, so ButtonInput/Gamepad never changed).
// ---------------------------------------------------------------------------

fn build_input_fixture() -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        TransformPlugin,
        bevy::input::InputPlugin,
        TestConventionsPlugin,
        PlaytestPlugin,
    ));
    app.add_message::<bevy::window::WindowEvent>();
    #[cfg(feature = "gamepad")]
    {
        app.add_message::<bevy::input::gamepad::GamepadConnectionEvent>();
        app.add_message::<bevy::input::gamepad::RawGamepadButtonChangedEvent>();
        app.add_message::<bevy::input::gamepad::RawGamepadAxisChangedEvent>();
    }
    app.insert_resource(IntentSurface::new(vec![
        SurfaceVariant::Choice(3),
        SurfaceVariant::Wait,
    ]));
    app.world_mut().spawn((
        Name::new("Player"),
        Gameplay,
        Transform::from_xyz(0.0, 0.0, 0.0),
    ));
    app.world_mut()
        .spawn((bevy::window::Window::default(), bevy::window::PrimaryWindow));
    app
}

/// Key press must reach ButtonInput<KeyCode> SAME-FRAME.
#[test]
fn fx8_key_press_reaches_button_input() {
    let mut app = build_input_fixture();
    app.world_mut()
        .resource_mut::<RawActionQueue>()
        .0
        .lock()
        .unwrap()
        .push_back((
            0,
            RawAction::Key {
                key: "ArrowRight".to_string(),
                hold_frames: 1,
            },
        ));
    let mut state = PlaytestState::new(60, 42);
    state.frame = 0;
    app.world_mut().insert_resource(state);
    app.update();
    let keys = app.world().resource::<ButtonInput<KeyCode>>();
    assert!(
        keys.pressed(KeyCode::ArrowRight),
        "key press must reach ButtonInput<KeyCode> the same frame"
    );
}

/// Mouse press must reach ButtonInput<MouseButton> (not just a WindowEvent).
#[test]
fn fx8_mouse_press_reaches_button_input() {
    let mut app = build_input_fixture();
    app.world_mut()
        .resource_mut::<RawActionQueue>()
        .0
        .lock()
        .unwrap()
        .push_back((
            0,
            RawAction::MouseButton {
                button: MouseBtn::Left,
                hold_frames: 1,
            },
        ));
    let mut state = PlaytestState::new(60, 42);
    state.frame = 0;
    app.world_mut().insert_resource(state);
    app.update();
    let buttons = app.world().resource::<ButtonInput<MouseButton>>();
    assert!(
        buttons.pressed(MouseButton::Left),
        "mouse press must reach ButtonInput<MouseButton>"
    );
}

/// Gamepad button must reach Gamepad::pressed (via RawGamepadEvent::Button).
#[cfg(feature = "gamepad")]
#[test]
fn fx8_gamepad_button_reaches_gamepad_state() {
    use bevy::input::gamepad::Gamepad;

    let mut app = build_input_fixture();
    app.world_mut()
        .resource_mut::<RawActionQueue>()
        .0
        .lock()
        .unwrap()
        .push_back((
            0,
            RawAction::GamepadButton {
                button: "South".to_string(),
                hold_frames: 1,
            },
        ));
    let mut state = PlaytestState::new(60, 42);
    state.frame = 0;
    app.world_mut().insert_resource(state);
    app.update();

    for pad in app.world_mut().query::<&Gamepad>().iter(app.world()) {
        assert!(
            pad.pressed(bevy::input::gamepad::GamepadButton::South),
            "gamepad button press must reach Gamepad component state"
        );
    }
}

/// hold_frames: button released after N frames.
#[test]
fn fx8_hold_frames_releases() {
    let mut app = build_input_fixture();
    app.world_mut()
        .resource_mut::<RawActionQueue>()
        .0
        .lock()
        .unwrap()
        .push_back((
            0,
            RawAction::Key {
                key: "KeyA".to_string(),
                hold_frames: 1,
            },
        ));
    let mut state = PlaytestState::new(60, 42);
    state.frame = 0;
    app.world_mut().insert_resource(state);
    app.update();
    assert!(
        app.world()
            .resource::<ButtonInput<KeyCode>>()
            .pressed(KeyCode::KeyA),
        "held on frame 0"
    );

    // Frame 1: hold expired — must be released.
    app.world_mut().resource_mut::<PlaytestState>().frame = 1;
    app.update();
    assert!(
        !app.world()
            .resource::<ButtonInput<KeyCode>>()
            .pressed(KeyCode::KeyA),
        "hold_frames=1 must release after 1 frame"
    );
}
