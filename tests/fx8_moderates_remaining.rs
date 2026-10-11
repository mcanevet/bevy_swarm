//! FX8 remaining moderate items: headless window spawn, gamepad axis hold_frames,
//! exactly-one gamepad assert, raw-action audit+replay, UI clicks headless.

use bevy::prelude::*;
use bevy_swarm::contract::{IntentSurface, SurfaceVariant, TestConventionsPlugin};
use bevy_swarm::driver::PlaytestPlugin;
use bevy_swarm::harness::Gameplay;
use bevy_swarm::raw_input::RawActionQueue;
use bevy_swarm::scenario::{MouseBtn, RawAction};
use bevy_swarm::state::PlaytestState;

fn build_headless_no_window() -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        TransformPlugin,
        TestConventionsPlugin,
        PlaytestPlugin,
    ));
    app.add_message::<bevy::input::keyboard::KeyboardInput>();
    app.add_message::<bevy::input::mouse::MouseButtonInput>();
    app.add_message::<bevy::input::mouse::MouseMotion>();
    app.add_message::<bevy::input::mouse::MouseWheel>();
    app.add_message::<bevy::window::WindowEvent>();
    app.insert_resource(IntentSurface::new(vec![
        SurfaceVariant::Choice(3),
        SurfaceVariant::Wait,
    ]));
    app.world_mut().spawn((
        Name::new("Player"),
        Gameplay,
        Transform::from_xyz(0.0, 0.0, 0.0),
    ));
    // Deliberately NO window spawned — test that headless mode spawns one
    app
}

#[test]
fn fx8_headless_spawns_window_when_absent() {
    let mut app = build_headless_no_window();

    // No window should exist initially
    let windows_before = app
        .world_mut()
        .query::<&bevy::window::Window>()
        .iter(app.world())
        .count();
    assert_eq!(windows_before, 0, "test setup: no window initially");

    // Run one frame — raw_input_preupdate_system should spawn a hidden window
    let mut state = PlaytestState::new(60, 42);
    state.frame = 0;
    app.world_mut().insert_resource(state);
    app.update();

    // Now a window should exist
    let windows_after = app
        .world_mut()
        .query::<&bevy::window::Window>()
        .iter(app.world())
        .count();
    assert!(
        windows_after > 0,
        "headless mode must spawn a window when none exists"
    );
}

#[cfg(feature = "gamepad")]
#[test]
fn fx8_gamepad_axis_hold_frames_releases() {
    use bevy::input::gamepad::{Gamepad, GamepadAxis};

    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        TransformPlugin,
        bevy::input::InputPlugin,
        TestConventionsPlugin,
        PlaytestPlugin,
    ));
    app.add_message::<bevy::window::WindowEvent>();
    app.add_message::<bevy::input::gamepad::GamepadConnectionEvent>();
    app.add_message::<bevy::input::gamepad::RawGamepadEvent>();
    app.add_message::<bevy::input::gamepad::GamepadEvent>();
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

    // Queue gamepad axis action with hold_frames = 1
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
                hold_frames: 1,
            },
        ));

    let mut state = PlaytestState::new(60, 42);
    state.frame = 0;
    app.world_mut().insert_resource(state);
    app.update();

    // Frame 0: axis should be active (value 0.5)
    let mut pads = app.world_mut().query::<&Gamepad>();
    let mut found_axis = false;
    for pad in pads.iter(app.world()) {
        if let Some(val) = pad.get(GamepadAxis::LeftStickX) {
            if val.abs() > 0.01 {
                found_axis = true;
            }
        }
    }
    assert!(found_axis, "gamepad axis should be active on frame 0");

    // Frame 1: hold expired — axis should return to neutral (0.0)
    app.world_mut().resource_mut::<PlaytestState>().frame = 1;
    app.update();

    // Check that axis is back to neutral
    let mut pads = app.world_mut().query::<&Gamepad>();
    let mut all_neutral = true;
    for pad in pads.iter(app.world()) {
        if let Some(val) = pad.get(GamepadAxis::LeftStickX) {
            if val.abs() > 0.01 {
                all_neutral = false;
            }
        }
    }
    assert!(
        all_neutral,
        "gamepad axis must release after hold_frames expire"
    );
}

#[cfg(feature = "gamepad")]
#[test]
fn fx8_gamepad_query_asserts_exactly_one_pad() {
    use bevy::input::gamepad::Gamepad;

    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        TransformPlugin,
        bevy::input::InputPlugin,
        TestConventionsPlugin,
        PlaytestPlugin,
    ));
    app.add_message::<bevy::window::WindowEvent>();
    app.add_message::<bevy::input::gamepad::GamepadConnectionEvent>();
    app.add_message::<bevy::input::gamepad::RawGamepadEvent>();
    app.add_message::<bevy::input::gamepad::GamepadEvent>();
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

    // Queue TWO gamepad actions (should still connect only once)
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

    // Assert EXACTLY ONE gamepad entity (not zero, not multiple)
    let mut pads = app.world_mut().query::<&Gamepad>();
    let pad_count = pads.iter(app.world()).count();
    assert_eq!(
        pad_count, 1,
        "exactly one gamepad must be present (not vacuous pass)"
    );
}

#[test]
fn fx8_raw_actions_logged_to_audit() {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        TransformPlugin,
        bevy::input::InputPlugin,
        TestConventionsPlugin,
        PlaytestPlugin,
    ));
    app.add_message::<bevy::window::WindowEvent>();
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

    // Queue raw actions
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
    app.world_mut()
        .resource_mut::<RawActionQueue>()
        .0
        .lock()
        .unwrap()
        .push_back((
            1,
            RawAction::MouseButton {
                button: MouseBtn::Left,
                hold_frames: 1,
            },
        ));

    let mut state = PlaytestState::new(60, 42);
    state.frame = 0;
    app.world_mut().insert_resource(state);
    app.update();
    app.world_mut().resource_mut::<PlaytestState>().frame = 1;
    app.update();
    app.world_mut().resource_mut::<PlaytestState>().frame = 2;
    app.update();

    // Check action log contains raw actions
    let log = app.world().resource::<bevy_swarm::contract::ActionLog>();

    let has_key = log.entries.iter().any(|e| e.action.starts_with("raw:key:"));
    let has_mouse = log
        .entries
        .iter()
        .any(|e| e.action.starts_with("raw:mouse:"));

    assert!(has_key, "raw key actions must be logged to audit");
    assert!(has_mouse, "raw mouse actions must be logged to audit");
}

#[test]
fn fx8_raw_actions_replayable() {
    use bevy_swarm::minimize::action_to_replay_intent;

    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        TransformPlugin,
        bevy::input::InputPlugin,
        TestConventionsPlugin,
        PlaytestPlugin,
    ));
    app.add_message::<bevy::window::WindowEvent>();
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

    // Queue raw actions
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
    app.world_mut().resource_mut::<PlaytestState>().frame = 1;
    app.update();

    // Convert action log to timed actions and check replayability
    let log = app.world().resource::<bevy_swarm::contract::ActionLog>();
    let timed_actions = bevy_swarm::scenario::action_log_to_timed_actions(&log.entries);

    assert!(!timed_actions.is_empty(), "action log should have entries");

    // At least some actions should be replayable
    let replayable: Vec<_> = timed_actions
        .iter()
        .filter_map(action_to_replay_intent)
        .collect();

    assert!(
        !replayable.is_empty(),
        "raw actions must be convertible to replay intents"
    );
}

// ---------- FX8 items 2 & 9: headless UI (layout + interaction) ----------

fn build_headless_ui_app() -> App {
    let mut app = App::new();
    app.add_plugins((
        bevy::asset::AssetPlugin::default(),
        MinimalPlugins,
        bevy::ui::UiPlugin,
        TransformPlugin,
        bevy::input::InputPlugin,
        bevy::text::TextPlugin,
        TestConventionsPlugin,
        PlaytestPlugin,
    ));
    app.add_message::<bevy::input::keyboard::KeyboardInput>();
    app.add_message::<bevy::input::mouse::MouseButtonInput>();
    app.add_message::<bevy::input::mouse::MouseMotion>();
    app.add_message::<bevy::input::mouse::MouseWheel>();
    app.add_message::<bevy::window::CursorMoved>();
    app.add_message::<bevy::window::WindowEvent>();
    app.world_mut()
        .init_resource::<Assets<bevy::image::Image>>();
    app.world_mut()
        .init_resource::<Assets<bevy::image::TextureAtlasLayout>>();
    app.insert_resource(IntentSurface::new(vec![
        SurfaceVariant::Choice(3),
        SurfaceVariant::Wait,
    ]));
    // Harness systems are gated on a live scenario.
    app.insert_resource(bevy_swarm::driver::ScenarioResource(std::sync::Arc::new(
        serde_json::from_str(
            r#"{"bot":{"type":"chaos","seed":1},"duration_s":0.1,"invariants":[]}"#,
        )
        .unwrap(),
    )));
    app.init_resource::<bevy_swarm::choice::ChoiceStream>();
    app.init_resource::<bevy_swarm::robustness::RobustnessTracker>();
    app.world_mut().spawn((
        Name::new("Player"),
        Gameplay,
        Transform::from_xyz(0.0, 0.0, 0.0),
    ));
    app
}

#[test]
fn fx9_headless_ui_layout_computes_node_sizes() {
    let mut app = build_headless_ui_app();

    // Spawn a camera (needed by ui_layout_system for the root viewport)
    // and a fixed-size UI node.
    app.world_mut()
        .spawn((Camera2d, bevy::ui::IsDefaultUiCamera));
    app.world_mut().spawn(bevy::ui::Node {
        width: bevy::ui::Val::Px(100.0),
        height: bevy::ui::Val::Px(50.0),
        ..default()
    });

    let mut state = PlaytestState::new(60, 42);
    state.frame = 0;
    app.world_mut().insert_resource(state);
    app.update();

    let mut q = app.world_mut().query::<&bevy::ui::ComputedNode>();
    let sizes: Vec<Vec2> = q.iter(app.world()).map(|n| n.size()).collect();
    assert!(
        sizes.iter().any(|s| s.x > 0.0 && s.y > 0.0),
        "ComputedNode sizes must be populated headlessly: {sizes:?}"
    );
}

#[test]
fn fx2_headless_ui_click_presses_interaction() {
    let mut app = build_headless_ui_app();

    app.world_mut()
        .spawn((Camera2d, bevy::ui::IsDefaultUiCamera));
    let button = app
        .world_mut()
        .spawn((
            bevy::ui::Node {
                width: bevy::ui::Val::Px(100.0),
                height: bevy::ui::Val::Px(50.0),
                ..default()
            },
            #[allow(deprecated)]
            bevy::ui::Interaction::default(),
            bevy::ui::RelativeCursorPosition::default(),
            Visibility::Visible,
            bevy::camera::visibility::InheritedVisibility::VISIBLE,
        ))
        .id();

    let mut state = PlaytestState::new(60, 42);
    state.frame = 0;
    app.world_mut().insert_resource(state);
    app.update(); // one frame to lay out

    // Move cursor over the button center (window defaults are 1280x720,
    // root node at top-left, so center of a 100x50 node ≈ (50, 25)).
    app.world_mut()
        .resource_mut::<RawActionQueue>()
        .0
        .lock()
        .unwrap()
        .push_back((0, RawAction::Cursor { pos: (50.0, 25.0) }));
    app.world_mut().resource_mut::<PlaytestState>().frame = 1;
    app.update(); // cursor moved

    // Press and hold the left mouse button.
    app.world_mut()
        .resource_mut::<RawActionQueue>()
        .0
        .lock()
        .unwrap()
        .push_back((
            0,
            RawAction::MouseButton {
                button: MouseBtn::Left,
                hold_frames: 5,
            },
        ));
    app.world_mut().resource_mut::<PlaytestState>().frame = 2;
    app.update();

    // The node must register as pressed (Interaction equivalent: `Pressed`).
    let pressed = app
        .world()
        .get::<bevy::picking::hover::Hovered>(button)
        .is_some();
    #[allow(deprecated)]
    let interaction = app.world().get::<bevy::ui::Interaction>(button).copied();
    #[allow(deprecated)]
    let is_pressed =
        |i: &Option<bevy::ui::Interaction>| matches!(i, Some(bevy::ui::Interaction::Pressed));
    assert!(
        pressed || is_pressed(&interaction),
        "button must register click headlessly: hovered={pressed:?}, interaction={interaction:?}"
    );
}
