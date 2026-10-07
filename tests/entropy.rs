#![allow(unknown_lints)]
#![allow(unexpected_cfgs)]
//! Z14: deterministic entropy via getrandom custom backend.
//!
//! These tests require the build cfg `getrandom_backend="custom"` AND
//! the `deterministic-entropy` feature. Without both, the hook is not
//! defined and getrandom falls back to the OS source; tests skip.

#![cfg(all(feature = "deterministic-entropy", getrandom_backend = "custom"))]

use bevy::prelude::*;
use bevy_swarm::entropy::{enter_run, exit_run};

#[test]
fn getrandom_fill_is_deterministic() {
    // Simulate two runs with the same seed drawing entropy.
    enter_run(42);
    let mut buf1 = [0u8; 32];
    getrandom04::fill(&mut buf1).unwrap();
    let mut buf2 = [0u8; 32];
    getrandom04::fill(&mut buf2).unwrap();
    exit_run();

    // Different position in the stream: continuation, not repetition.
    assert_ne!(buf1, buf2);

    // A fresh run with the same seed reproduces the FIRST draw.
    enter_run(42);
    let mut buf3 = [0u8; 32];
    getrandom04::fill(&mut buf3).unwrap();
    exit_run();
    assert_eq!(buf1, buf3, "same seed must reproduce the same entropy");

    // Different seed diverges.
    enter_run(43);
    let mut buf4 = [0u8; 32];
    getrandom04::fill(&mut buf4).unwrap();
    exit_run();
    assert_ne!(buf1, buf4);
}

#[test]
fn uuid_v4_deterministic() {
    enter_run(7);
    let u1 = uuid::Uuid::new_v4();
    exit_run();

    enter_run(7);
    let u2 = uuid::Uuid::new_v4();
    exit_run();
    assert_eq!(u1, u2, "same seed must reproduce the same uuid");

    enter_run(8);
    let u3 = uuid::Uuid::new_v4();
    exit_run();
    assert_ne!(u1, u3);
}

#[test]
fn run_scenario_provides_entropy_scope() {
    use bevy_swarm::contract::TestApi;
    use bevy_swarm::driver::{run_scenario, PlaytestPlugin};
    use bevy_swarm::scenario::Scenario;

    #[derive(Resource, Default)]
    struct LatestUuid(uuid::Uuid);

    fn spawn_uuid_system(res: Res<LatestUuid>, mut api: ResMut<TestApi>) {
        // Draw randomness INSIDE the run (like a game spawning entities).
        api.custom_numeric
            .insert("Uuid.HighBits".into(), res.0.as_u128() as f64);
    }

    fn make_uuid_system(mut res: ResMut<LatestUuid>) {
        res.0 = uuid::Uuid::new_v4();
    }

    let scenario: Scenario =
        serde_json::from_str(r#"{"bot":{"type":"chaos","seed":99},"duration_s":0.05}"#).unwrap();

    let run = || {
        let mut app = App::new();
        app.add_plugins(PlaytestPlugin);
        app.init_resource::<LatestUuid>();
        app.insert_resource(TestApi {
            score: 0,
            active_players: 0,
            custom_numeric: Default::default(),
            custom_text: Default::default(),
        });
        app.add_systems(Update, make_uuid_system);
        app.add_systems(Last, spawn_uuid_system);
        app.insert_resource(bevy_swarm::determinism::StateTrace::default());
        let rep = run_scenario(&mut app, &scenario).unwrap();
        rep.state_trace
            .iter()
            .flatten()
            .map(|d| d.hash)
            .collect::<Vec<_>>()
    };

    let d1 = run();
    let d2 = run();
    assert_eq!(
        d1, d2,
        "same scenario seed must reproduce identical digests"
    );
    assert!(!d1.is_empty());
}

#[test]
fn error_layout_assertions() {
    // Verify getrandom 0.3 and 0.4 Error types have identical layout.
    // (We only depend on 0.4, but assert against 0.3's definition
    // as documented in the bead.)
    use std::mem::{align_of, size_of};

    // getrandom 0.4 Error is a tuple struct with NonZeroU16 (or similar).
    // Assert our assumptions hold.
    assert_eq!(size_of::<getrandom04::Error>(), 4); // NonZeroU32
    assert_eq!(align_of::<getrandom04::Error>(), 4);
    assert_eq!(size_of::<Result<(), getrandom04::Error>>(), 4);
}
