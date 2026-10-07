//! I1: identity index + deterministic StableId integration tests.

use bevy::prelude::*;
use bevy_swarm::contract::Gameplay;
use bevy_swarm::driver::{run_scenario, PlaytestPlugin};
use bevy_swarm::identity::{IdentityIndex, StableId};
use bevy_swarm::scenario::Scenario;

fn scenario_json(bot: &str) -> Scenario {
    serde_json::from_str(bot).unwrap()
}

fn build_app() -> App {
    let mut app = App::new();
    app.add_plugins((
        bevy::MinimalPlugins,
        bevy::transform::TransformPlugin,
        PlaytestPlugin,
    ));
    app
}

#[test]
fn stable_ids_deterministic() {
    // Two fresh runs of the same scenario assign identical
    // (StableId -> Name) maps, including entities spawned mid-run.
    let scenario =
        scenario_json(r#"{"bot":{"type":"chaos","seed":9},"duration_s":0.2,"invariants":[]}"#);

    let run = || {
        let mut app = build_app();
        app.add_systems(Startup, |mut commands: Commands| {
            commands.spawn((Gameplay, Name::new("A"), Transform::default()));
            commands.spawn((Gameplay, Transform::default())); // unnamed
        });
        // Mid-run spawner.
        app.add_systems(
            Update,
            |mut commands: Commands, frame: Res<bevy_swarm::harness::PlaytestState>| {
                if frame.frame == 3 {
                    commands.spawn((Gameplay, Name::new("MidRun"), Transform::default()));
                }
            },
        );
        let _ = run_scenario(&mut app, &scenario).unwrap();
        let idx = app.world().resource::<IdentityIndex>();
        let mut map: Vec<(u64, Option<String>)> = idx
            .stable_ids()
            .map(|(id, e)| {
                let name = app.world().get::<Name>(e).map(|n| n.as_str().to_string());
                (id.0, name)
            })
            .collect();
        map.sort();
        map
    };

    let m1 = run();
    let m2 = run();
    assert_eq!(m1, m2, "same scenario must assign identical StableId maps");
    assert!(m1.len() >= 2, "expected indexed entities, got {:?}", m1);
    // Entities include at least one unnamed (None) entry.
    assert!(
        m1.iter().any(|(_, n)| n.is_none()),
        "unnamed entity should still get a StableId: {:?}",
        m1
    );
}

#[test]
fn replay_select_unnamed_via_stable_id() {
    // Chaos selects an unnamed entity; the resulting audit log must
    // round-trip to a replay intent (stable_id form), i.e. replay of an
    // unnamed entity's select works.
    let scenario =
        scenario_json(r#"{"bot":{"type":"chaos","seed":11},"duration_s":0.2,"invariants":[]}"#);

    let mut app = build_app();
    app.add_plugins(bevy_swarm::contract::TestConventionsPlugin);
    app.insert_resource(bevy_swarm::contract::IntentSurface::new(vec![
        bevy_swarm::contract::SurfaceVariant::Select,
    ]));
    app.add_systems(Startup, |mut commands: Commands| {
        commands.spawn((Gameplay, Transform::default())); // unnamed only
    });
    let rep = run_scenario(&mut app, &scenario).unwrap();

    // Find select intents in the audit log.
    let selects: Vec<&bevy_swarm::contract::ActionEntry> = rep
        .action_log
        .iter()
        .filter(|e| e.action == "intent:select")
        .collect();
    assert!(!selects.is_empty(), "chaos made no selects");
    // Every select over the unnamed Gameplay entity must carry a stable_id.
    for e in &selects {
        let d: serde_json::Value =
            serde_json::from_str(e.details.as_deref().unwrap_or("{}")).unwrap();
        assert!(
            d.get("stable_id").is_some(),
            "select over Gameplay entity lacks stable_id: {}",
            e.details.as_deref().unwrap_or("")
        );
    }
    // And they are replayable: no unreplayable actions.
    assert_eq!(rep.unreplayable_actions, 0, "{:#?}", rep.warnings);
}

#[test]
fn name_index_tracks_rename_and_despawn() {
    let mut app = build_app();
    use bevy_swarm::contract::TestConventionsPlugin;
    app.add_plugins(TestConventionsPlugin);

    let world = app.world_mut();
    let e1 = world
        .spawn((Gameplay, Name::new("Hero"), Transform::default()))
        .id();
    // commands auto-flush at end of frame; skip explicit flush
    let idx = world.resource::<IdentityIndex>();
    assert_eq!(idx.by_name("Hero"), Some(e1));

    // Rename.
    world.entity_mut(e1).insert(Name::new("Villain"));
    world.flush();
    let idx = world.resource::<IdentityIndex>();
    assert_eq!(idx.by_name("Hero"), None, "old name gone after rename");
    assert_eq!(idx.by_name("Villain"), Some(e1), "new name indexed");

    // Despawn.
    world.despawn(e1);
    world.flush();
    let idx = world.resource::<IdentityIndex>();
    assert_eq!(idx.by_name("Villain"), None, "name gone after despawn");
    assert!(idx.is_empty(), "index empty after despawn");
}

#[test]
fn index_lookup_matches_scan() {
    // Property-style loop over seeded random spawn/despawn/rename
    // sequences, comparing IdentityIndex::by_name with a brute-force scan.
    let mut rng: u64 = 0x5EED;
    let mut next = move || {
        // xorshift64*
        rng ^= rng >> 12;
        rng ^= rng << 25;
        rng ^= rng >> 27;
        rng.wrapping_mul(0x2545F4914F6CDD1D)
    };

    let mut app = build_app();
    use bevy_swarm::contract::TestConventionsPlugin;
    app.add_plugins(TestConventionsPlugin);
    let world = app.world_mut();

    let mut live: Vec<(bevy::ecs::entity::Entity, String)> = Vec::new();
    let names = ["A", "B", "C"];

    for _ in 0..200 {
        let roll = next() % 10;
        if roll < 4 {
            // Spawn with a random name (possibly duplicated).
            let name = names[(next() % names.len() as u64) as usize].to_string();
            let e = world
                .spawn((Gameplay, Name::new(name.clone()), Transform::default()))
                .id();
            live.push((e, name));
        } else if roll < 7 && !live.is_empty() {
            // Despawn a random live entity.
            let i = (next() % live.len() as u64) as usize;
            let (e, _) = live.swap_remove(i);
            world.despawn(e);
        } else if !live.is_empty() {
            // Rename a random live entity.
            let i = (next() % live.len() as u64) as usize;
            let new_name = names[(next() % names.len() as u64) as usize].to_string();
            let (e, _) = live[i];
            world.entity_mut(e).insert(Name::new(new_name.clone()));
            live[i].1 = new_name;
        }
        // commands auto-flush at end of frame; skip explicit flush

        // Compare index vs brute-force scan for every name.
        // Brute force: query the world for all Gameplay entities with
        // that Name and pick the lowest StableId — the deterministic
        // answer the index must reproduce.
        world.flush();
        let mut q = world.query::<(&Name, &StableId, bevy::ecs::entity::Entity)>();
        let idx = world.resource::<IdentityIndex>();
        for n in names {
            let scan = q
                .iter(world)
                .filter(|(nm, _, _)| nm.as_str() == n)
                .min_by_key(|(_, sid, _)| **sid)
                .map(|(_, _, e)| e);
            let got = idx.by_name(n);
            assert_eq!(
                got, scan,
                "index mismatch for '{}' after iteration: live={:?}",
                n, live
            );
        }
    }
}
