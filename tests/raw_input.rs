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
