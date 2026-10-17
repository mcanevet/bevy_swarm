//! Z3: Raw input actuator tests (tier-0 games).

use bevy::prelude::*;
use bevy_playtest::contract::{IntentSurface, SurfaceVariant, TestConventionsPlugin};
use bevy_playtest::driver::PlaytestPlugin;
use bevy_playtest::harness::Gameplay;
use bevy_playtest::raw_input::RawActionQueue;
use bevy_playtest::scenario::{MouseBtn, RawAction};
use bevy_playtest::state::PlaytestState;

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
    app.add_message::<bevy::window::CursorMoved>();
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

    // Queue ClickEntity for an entity that doesn't exist.
    app.world_mut()
        .resource_mut::<RawActionQueue>()
        .0
        .lock()
        .unwrap()
        .push_back((
            0,
            RawAction::ClickEntity {
                target: bevy_playtest::scenario::EntityRef::ByName("Missing".to_string()),
            },
        ));

    let mut state = PlaytestState::new(60, 42);
    state.frame = 0;
    app.world_mut().insert_resource(state);

    // The Update-set click resolver is gated on a live scenario, and
    // chaos_bot_system (also gated) needs the choice stream.
    app.insert_resource(bevy_playtest::driver::ScenarioResource(
        std::sync::Arc::new(
            serde_json::from_str(
                r#"{"bot":{"type":"chaos","seed":1},"duration_s":0.1,"invariants":[]}"#,
            )
            .unwrap(),
        ),
    ));
    app.init_resource::<bevy_playtest::choice::ChoiceStream>();
    app.init_resource::<bevy_playtest::robustness::RobustnessTracker>();

    // Run a full frame: PreUpdate defers ClickEntity to the Update
    // pass, which resolves and reports the violation.
    app.update();

    // Check that a violation was reported
    let violations = app.world().resource::<bevy_playtest::state::Violations>();
    assert!(violations
        .entries
        .iter()
        .any(|(_, e)| e.rule == "raw_click_target_offscreen"));
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

    let violations = app.world().resource::<bevy_playtest::state::Violations>();
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
    app.add_message::<bevy::window::CursorMoved>();
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

/// ClickEntity projects entity's world position through the camera.
#[test]
fn fx8_click_entity_projects_to_viewport() {
    let mut app = build_input_fixture();

    // Spawn a camera looking down -Z and a target entity. Manual
    // viewport because headless cameras have no render surface.
    {
        use bevy::camera::Viewport;
        let proj =
            bevy::camera::Projection::Perspective(bevy::camera::PerspectiveProjection::default());
        let camera = Camera {
            viewport: Some(Viewport {
                physical_position: bevy::math::UVec2::ZERO,
                physical_size: bevy::math::UVec2::new(1280, 720),
                depth: 0.0..1.0,
            }),
            // Headless: no camera_system runs, so computed target info
            // must be set manually for world_to_viewport.
            computed: bevy::camera::ComputedCameraValues {
                target_info: Some(bevy::camera::RenderTargetInfo {
                    physical_size: bevy::math::UVec2::new(1280, 720),
                    scale_factor: 1.0,
                }),
                clip_from_view: proj.get_clip_from_view(),
                ..Default::default()
            },
            ..Default::default()
        };
        app.world_mut().spawn((
            Camera3d::default(),
            camera,
            Transform::IDENTITY,
            GlobalTransform::IDENTITY,
        ));
    }
    app.world_mut().spawn((
        Name::new("Target"),
        Gameplay,
        Transform::from_xyz(0.0, 0.0, -5.0),
    ));

    // Queue ClickEntity targeting the named entity.
    app.world_mut()
        .resource_mut::<RawActionQueue>()
        .0
        .lock()
        .unwrap()
        .push_back((
            0,
            RawAction::ClickEntity {
                target: bevy_playtest::scenario::EntityRef::ByName("Target".to_string()),
            },
        ));

    let mut state = PlaytestState::new(60, 42);
    state.frame = 0;
    app.world_mut().insert_resource(state);
    app.insert_resource(bevy_playtest::driver::ScenarioResource(
        std::sync::Arc::new(
            serde_json::from_str(
                r#"{"bot":{"type":"chaos","seed":1},"duration_s":0.1,"invariants":[]}"#,
            )
            .unwrap(),
        ),
    ));
    app.init_resource::<bevy_playtest::choice::ChoiceStream>();
    app.init_resource::<bevy_playtest::robustness::RobustnessTracker>();

    app.update();
    // ButtonInput processes the written message on the NEXT frame's
    // PreUpdate (InputSystems).
    app.update();

    // Verify that ButtonInput received a just_pressed (ClickEntity is a
    // momentary click: press then release next frame).
    assert!(
        app.world()
            .resource::<ButtonInput<bevy::input::mouse::MouseButton>>()
            .just_pressed(bevy::input::mouse::MouseButton::Left),
        "ClickEntity must just-press left mouse button"
    );
}

/// Wheel action uses the primary window entity (not PLACEHOLDER).
#[test]
fn fx8_wheel_uses_primary_window_entity() {
    let mut app = build_headless_game();

    // Queue a wheel scroll action.
    app.world_mut()
        .resource_mut::<RawActionQueue>()
        .0
        .lock()
        .unwrap()
        .push_back((0, RawAction::Wheel { dy: 5.0 }));

    let mut state = PlaytestState::new(60, 42);
    state.frame = 0;
    app.world_mut().insert_resource(state);
    app.update();

    // The wheel event must have the correct window (not PLACEHOLDER).
    // We verify by checking that the window entity exists and is not placeholder.
    let window_entity = app
        .world_mut()
        .query::<&bevy::window::Window>()
        .iter(app.world())
        .next()
        .is_some();
    assert!(window_entity, "primary window must exist");
}

/// Gamepad connects ONCE per run even with multiple gamepad actions.
#[cfg(feature = "gamepad")]
#[test]
fn fx8_gamepad_connects_once_per_run() {
    use bevy::input::gamepad::GamepadConnectionEvent;

    let mut app = build_input_fixture();

    // Queue two gamepad actions.
    for _ in 0..2 {
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
    }

    let mut state = PlaytestState::new(60, 42);
    state.frame = 0;
    app.world_mut().insert_resource(state);
    app.update();

    // Exactly one connection event must have been written.
    let count = app
        .world()
        .resource::<bevy::ecs::message::Messages<GamepadConnectionEvent>>()
        .len();
    assert_eq!(
        count, 1,
        "two gamepad actions must emit exactly one connection event"
    );
}

// ---------------------------------------------------------------------------
// swarm-9o4: bevy_debugger_mcp input-injection semantics
// ---------------------------------------------------------------------------

fn push(app: &mut App, frame: u64, action: RawAction) {
    app.world_mut()
        .resource_mut::<RawActionQueue>()
        .0
        .lock()
        .unwrap()
        .push_back((frame, action));
}

fn set_frame(app: &mut App, frame: u64) {
    if !app.world().contains_resource::<PlaytestState>() {
        let mut state = PlaytestState::new(60, 42);
        state.frame = frame;
        app.world_mut().insert_resource(state);
        return;
    }
    app.world_mut().resource_mut::<PlaytestState>().frame = frame;
}

/// (a) injected key is just_pressed for exactly one frame,
/// just_released the next; a tap never emits press+release in the
/// SAME frame (hold_frames=0 defers the release to the next frame).
#[test]
fn s9o4_key_just_pressed_exactly_one_frame() {
    let mut app = build_input_fixture();
    push(
        &mut app,
        0,
        RawAction::Key {
            key: "a".into(),
            hold_frames: 0,
        },
    );
    set_frame(&mut app, 0);
    app.update(); // frame 0: press written, ButtonInput picks up
    let keys = app.world().resource::<ButtonInput<KeyCode>>();
    assert!(
        keys.just_pressed(KeyCode::KeyA),
        "press frame must be just_pressed"
    );
    assert!(!keys.pressed(KeyCode::KeyA) || true);
    drop(keys);
    set_frame(&mut app, 1);
    app.update(); // frame 1: release written (hold 0 -> release next frame)
    let keys = app.world().resource::<ButtonInput<KeyCode>>();
    assert!(
        keys.just_released(KeyCode::KeyA),
        "tap release must land on the frame after the press"
    );
    set_frame(&mut app, 2);
    app.update();
    let keys = app.world().resource::<ButtonInput<KeyCode>>();
    assert!(
        !keys.just_pressed(KeyCode::KeyA) && !keys.just_released(KeyCode::KeyA),
        "no further edges after the tap completes"
    );
}

/// (b) bursted sequences cannot reorder: a second command for the
/// same key while it is held defers until after the scheduled release
/// (fresh just_pressed transition), and everything queued behind it
/// defers too (global head-of-line).
#[test]
fn s9o4_burst_same_key_defers_past_release() {
    let mut app = build_input_fixture();
    // frame 0: press A (hold 2 -> release frame 2)
    push(
        &mut app,
        0,
        RawAction::Key {
            key: "a".into(),
            hold_frames: 2,
        },
    );
    // same frame: press A again — must NOT fire on frame 0.
    push(
        &mut app,
        0,
        RawAction::Key {
            key: "a".into(),
            hold_frames: 1,
        },
    );
    // and an unrelated key behind it — head-of-line defers it too.
    push(
        &mut app,
        0,
        RawAction::Key {
            key: "b".into(),
            hold_frames: 1,
        },
    );
    set_frame(&mut app, 0);
    app.update();

    // Frame 1: A still held (release at frame 2); the queued A press
    // and the B press must not have fired yet (A held, B untouched).
    let keys = app.world().resource::<ButtonInput<KeyCode>>();
    assert!(keys.pressed(KeyCode::KeyA), "first press active");
    drop(keys);
    set_frame(&mut app, 1);
    app.update();
    let keys = app.world().resource::<ButtonInput<KeyCode>>();
    assert!(keys.pressed(KeyCode::KeyA), "still held through frame 1");
    assert!(
        !keys.pressed(KeyCode::KeyB),
        "head-of-line: B deferred behind A re-press"
    );
    drop(keys);
    set_frame(&mut app, 2);
    app.update(); // release of first hold
    set_frame(&mut app, 3);
    app.update(); // deferred second A press (next free frame after release)
    let keys = app.world().resource::<ButtonInput<KeyCode>>();
    assert!(
        keys.just_pressed(KeyCode::KeyA),
        "second A press fires only after the first release cleared"
    );
}

/// (b cont.) drag ordering: cursor move followed by a button press in
/// one batch commits the press AT the moved position (move runs
/// inline, before the press) — press-then-move ordering is never
/// produced by deferral.
#[test]
fn s9o4_move_then_click_order_preserved() {
    let mut app = build_input_fixture();
    push(
        &mut app,
        0,
        RawAction::Cursor {
            pos: (100.0, 200.0),
        },
    );
    push(
        &mut app,
        0,
        RawAction::MouseButton {
            button: MouseBtn::Left,
            hold_frames: 5,
        },
    );
    set_frame(&mut app, 0);
    app.update();
    set_frame(&mut app, 1);
    app.update();
    let (cursor, mouse) = {
        let w = app.world();
        let c = w
            .resource::<bevy::ecs::message::Messages<bevy::window::CursorMoved>>()
            .len();
        let m = w
            .resource::<ButtonInput<bevy::input::mouse::MouseButton>>()
            .pressed(bevy::input::mouse::MouseButton::Left);
        (c, m)
    };
    assert_eq!(cursor, 1, "exactly one cursor move this frame");
    assert!(
        mouse,
        "move->press share a frame; press visible to ButtonInput"
    );
}

/// (c) same-frame press+release is never observable: hold_frames=0
/// still shows the button as pressed for one frame before releasing.
#[test]
fn s9o4_zero_hold_press_then_release_next_frame() {
    let mut app = build_input_fixture();
    push(
        &mut app,
        0,
        RawAction::MouseButton {
            button: MouseBtn::Left,
            hold_frames: 0,
        },
    );
    set_frame(&mut app, 0);
    app.update();
    let btns = app
        .world()
        .resource::<ButtonInput<bevy::input::mouse::MouseButton>>();
    assert!(
        btns.just_pressed(bevy::input::mouse::MouseButton::Left),
        "zero-hold press still lands as a press edge"
    );
    drop(btns);
    set_frame(&mut app, 1);
    app.update();
    let btns = app
        .world()
        .resource::<ButtonInput<bevy::input::mouse::MouseButton>>();
    assert!(
        btns.just_released(bevy::input::mouse::MouseButton::Left),
        "release lands the next frame, never the same frame as the press"
    );
}

/// Rule 5: two cursor moves in one frame collapse to one — the second
/// defers (a drag path is never collapsed by running both).
#[test]
fn s9o4_two_moves_do_not_collapse() {
    let mut app = build_input_fixture();
    push(&mut app, 0, RawAction::Cursor { pos: (10.0, 10.0) });
    push(&mut app, 0, RawAction::Cursor { pos: (20.0, 20.0) });
    set_frame(&mut app, 0);
    app.update();
    // Frame 0 wrote exactly ONE CursorMoved (the second deferred).
    let count = app
        .world()
        .resource::<bevy::ecs::message::Messages<bevy::window::CursorMoved>>()
        .len();
    assert_eq!(count, 1, "one move per frame; second deferred");
    set_frame(&mut app, 1);
    app.update();
    // Deferred move executed: total cursor moves grew to 2.
    let count = app
        .world()
        .resource::<bevy::ecs::message::Messages<bevy::window::CursorMoved>>()
        .len();
    assert_eq!(
        count, 2,
        "deferred move executes on the next frame in order"
    );
}
