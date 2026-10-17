//! RF11 (swarm-m2h.25): headless pointer/ClickEntity acceptance tests.

use bevy::prelude::*;
use bevy_playtest::branch::{InProcess, ScenarioRunner};
use bevy_playtest::headless::headless_app_factory;
use bevy_playtest::raw_input::RawActionQueue;
use bevy_playtest::scenario::{EntityRef, RawAction, Scenario};
use bevy_playtest::state::PlaytestState;

struct TestPlugin;
impl Plugin for TestPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, |mut commands: Commands| {
            commands.spawn((Camera2d, bevy::ui::IsDefaultUiCamera));
            commands.spawn((
                Name::new("Target"),
                Transform::default(),
                Visibility::Visible,
            ));
        });
    }
}

// RF11 critical: camera target_info is computed headlessly so
// ClickEntity on a world entity projects (NoViewportSize gone).
#[test]
fn rf11_click_entity_projects_without_renderer() {
    let factory = headless_app_factory(|| TestPlugin);

    let scenario: Scenario =
        serde_json::from_str(r#"{"bot":{"type":"chaos","seed":7},"duration_s":0.2}"#).unwrap();

    let runner = InProcess { factory };
    let report = runner.run(&scenario).unwrap();

    // The action must NOT produce a raw_click_target_offscreen violation.
    for v in &report.violations {
        assert_ne!(
            v.rule, "raw_click_target_offscreen",
            "ClickEntity must project headlessly: {v:?}"
        );
    }
}

// RF11 moderate: two consecutive ClickEntity actions both register and
// the release fires (no stuck-held button).
#[test]
fn rf11_click_entity_releases_between_actions() {
    let factory = headless_app_factory(|| TestPlugin);

    let mut app = factory();
    // Queue two ClickEntity actions on frames 1 and 8.
    app.world_mut()
        .resource_mut::<RawActionQueue>()
        .0
        .lock()
        .unwrap()
        .push_back((
            1,
            RawAction::ClickEntity {
                target: EntityRef::ByName("Target".to_string()),
            },
        ));

    // Live-scenario gating: the click resolver needs ScenarioResource
    // AND PlaytestState, otherwise the queue is silently ignored
    // (vacuous test).
    app.insert_resource(bevy_playtest::driver::ScenarioResource(
        std::sync::Arc::new(
            serde_json::from_str(
                r#"{"bot":{"type":"chaos","seed":1},"duration_s":0.1,"invariants":[]}"#,
            )
            .unwrap(),
        ),
    ));
    app.world_mut().insert_resource(PlaytestState::new(60, 42));
    app.init_resource::<bevy_playtest::choice::ChoiceStream>();
    // Oracle resources normally installed by run_scenario.
    app.init_resource::<bevy_playtest::state::Violations>();
    app.init_resource::<bevy_playtest::robustness::RobustnessTracker>();

    // Two ClickEntity actions, frames 1 and 8.
    for (frame, target) in [(1usize, "Target"), (8, "Target")] {
        app.world_mut()
            .resource_mut::<RawActionQueue>()
            .0
            .lock()
            .unwrap()
            .push_back((
                frame as u64,
                RawAction::ClickEntity {
                    target: EntityRef::ByName(target.to_string()),
                },
            ));
    }

    let mut press_count = 0;
    for frame in 0..12 {
        app.world_mut().resource_mut::<PlaytestState>().frame = frame;
        app.update();
        let mouse = app
            .world()
            .resource::<bevy::input::ButtonInput<bevy::input::mouse::MouseButton>>();
        if mouse.just_pressed(bevy::input::mouse::MouseButton::Left) {
            press_count += 1;
        }
    }
    // Non-vacuous: both clicks pressed.
    assert_eq!(press_count, 2, "two ClickEntity actions must both press");
    // Release: the button must not be stuck held.
    let left_held = app
        .world()
        .resource::<bevy::input::ButtonInput<bevy::input::mouse::MouseButton>>()
        .pressed(bevy::input::mouse::MouseButton::Left);
    assert!(!left_held, "ClickEntity must release the button");
}
