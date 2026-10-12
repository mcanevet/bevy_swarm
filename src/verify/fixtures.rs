use crate::contract::*;
use crate::harness::*;
use bevy::prelude::*;

/// Stub game score resource — verifies the score-related invariants.
#[derive(bevy::ecs::resource::Resource, Default)]
pub(crate) struct Score(pub i64);

/// Mirror Score into TestApi each frame (what a real game's sync does).
pub(crate) fn sync_score_to_test_api(mut api: ResMut<TestApi>, score: Res<Score>) {
    api.score = score.0;
}

pub(crate) fn build_app() -> App {
    let mut app = build_app_no_scoring();
    app.insert_resource(Score(0));
    app.add_systems(
        Update,
        (handle_choice_awards_points, sync_score_to_test_api),
    );
    app
}

/// Like `build_app` but without the healthy scoring handler — the mutation
/// benchmark replaces it with a buggy variant.
pub(crate) fn build_app_no_scoring() -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        bevy::transform::TransformPlugin,
        bevy::diagnostic::FrameTimeDiagnosticsPlugin::default(),
        TestConventionsPlugin,
        PlaytestPlugin,
    ));
    // Declare an intent surface (a real game would declare its own verbs)
    app.insert_resource(IntentSurface::new(vec![
        SurfaceVariant::Choice(3),
        SurfaceVariant::Wait,
    ]));
    // Spawn a minimal Ball entity for bounds-check tests
    app.add_systems(Startup, spawn_minimal_ball);
    app.insert_resource(Score(0));
    app.add_systems(Update, sync_score_to_test_api);
    app
}

pub(crate) fn spawn_minimal_ball(mut commands: Commands) {
    commands.spawn((Name::new("Ball"), Gameplay, Transform::default()));
}

/// Stub gameplay: each Choice intent awards +5 points to the game Score
/// resource (sync_test_api mirrors it into TestApi each frame).
pub(crate) fn handle_choice_awards_points(
    mut reader: bevy::ecs::message::MessageReader<UserIntent>,
    mut score: ResMut<Score>,
) {
    for intent in reader.read() {
        if let UserIntent::Choice { .. } = intent {
            score.0 += 5;
        }
    }
}

// ---------------------------------------------------------------------------
// Mutation benchmark: seeded bug archetypes
// ---------------------------------------------------------------------------

/// Bug 1: bounds pass-through — at frame 10, teleport every transformed
/// Gameplay entity far out of bounds (as if it flew through a wall).
pub(crate) fn inject_bounds_bug(
    mut frame: Local<u64>,
    mut q: Query<&mut Transform, With<Gameplay>>,
) {
    *frame += 1;
    if *frame == 10 {
        for mut t in &mut q {
            t.translation.x = 10_001.0;
        }
    }
}

/// Bug 2: runaway score — handler re-fires every frame (timer-leak proxy).
/// Adds +100/sec to exceed the 5/sec ceiling.
pub(crate) fn inject_rampant_score_bug(mut score: ResMut<Score>) {
    score.0 += 100; // +6000/sec, far above the 5/sec legal ceiling
}

/// Bug 3: missing-points — point awards silently suppressed after frame 1.
pub(crate) fn inject_missing_points_bug(
    mut reader: bevy::ecs::message::MessageReader<UserIntent>,
    mut score: ResMut<Score>,
    mut frame: Local<u64>,
) {
    *frame += 1;
    for intent in reader.read() {
        if let UserIntent::Choice { .. } = intent {
            if *frame <= 1 {
                score.0 += 5; // Normal behavior: +5 on the first Choice
            } else {
                // After frame 1: awards don't just vanish — a buggy
                // rollback/clamp path actively DROPS the score so the
                // differential no_decrease invariant can see it.
                score.0 -= 1;
            }
        }
    }
}

pub(crate) fn replay_from_action_log(
    log: &[crate::contract::ActionEntry],
    duration_s: f32,
) -> Scenario {
    let timed = crate::harness::action_log_to_timed_actions(log);
    let inputs: Vec<serde_json::Value> = timed
        .iter()
        .filter_map(|ta| {
            crate::harness::action_to_replay_intent(ta)
                .map(|intent| timed_replay_input(ta.frame, intent))
        })
        .collect();
    let json = serde_json::json!({
        "bot": {"type": "replay", "inputs": inputs},
        "duration_s": duration_s,
        "invariants": [],
    });
    serde_json::from_value(json).expect("replay scenario JSON")
}

pub(crate) fn timed_replay_input(
    frame: u64,
    intent: crate::harness::ReplayIntent,
) -> serde_json::Value {
    let inner = match intent {
        crate::harness::ReplayIntent::Move { dir } => {
            serde_json::json!({"intent": "move", "dir": [dir.0, dir.1]})
        }
        crate::harness::ReplayIntent::Choice { index } => {
            serde_json::json!({"intent": "choice", "index": index})
        }
        crate::harness::ReplayIntent::Select { target } => {
            serde_json::json!({"intent": "select", "target": target})
        }
        crate::harness::ReplayIntent::Axis { name, value } => {
            serde_json::json!({"intent": "axis", "name": name, "value": value})
        }
        crate::harness::ReplayIntent::Wait => serde_json::json!({"intent": "wait"}),
        crate::harness::ReplayIntent::Raw { raw } => {
            serde_json::json!({"intent": "raw", "raw": raw})
        }
    };
    serde_json::json!({ "frame": frame, "intent": inner })
}
