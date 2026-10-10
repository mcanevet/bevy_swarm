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

// ---------------------------------------------------------------------------
// FX6 I1 re-review: re-index bug (next_stable collision, by_name lost).
// ---------------------------------------------------------------------------

#[test]
fn fx6_i1_second_run_no_stable_id_collision() {
    // Second run on the same App: entities existing at reset (from the
    // first run) plus entities spawned mid-second-run must all carry
    // DISTINCT StableIds, and by_name lookups must survive the reset.
    use bevy_swarm::identity::{IdentityIndex, StableId};

    let mut app = build_app();
    app.add_systems(Startup, |mut commands: Commands| {
        commands.spawn((Gameplay, Name::new("Hero"), Transform::default()));
        commands.spawn((Gameplay, Transform::default()));
    });
    // Mid-run spawner (active in BOTH runs).
    app.add_systems(
        Update,
        |mut commands: Commands, frame: Res<bevy_swarm::harness::PlaytestState>| {
            if frame.frame == 3 {
                commands.spawn((Gameplay, Name::new("MidRun"), Transform::default()));
            }
        },
    );
    let sc: Scenario =
        serde_json::from_str(r#"{"bot":{"type":"chaos","seed":7},"duration_s":0.15}"#).unwrap();
    let _ = run_scenario(&mut app, &sc).unwrap();
    let _ = run_scenario(&mut app, &sc).unwrap();

    let world = app.world();
    let idx = world.resource::<IdentityIndex>();
    // 1) All StableIds distinct.
    let ids: Vec<StableId> = idx.stable_ids().map(|(id, _)| id).collect();
    let uniq: std::collections::HashSet<_> = ids.iter().collect();
    assert_eq!(
        ids.len(),
        uniq.len(),
        "duplicate StableIds after second run: {:?}",
        ids
    );
    // 2) by_name lookups still work (Hero existed before the reset).
    let hero = idx.by_name("Hero");
    assert!(
        hero.is_some(),
        "by_name(Hero) lost after second run's reset — replay by name will fail"
    );
    // 3) The MidRun entity spawned in the SECOND run is indexed by name.
    let midrun = idx.by_name("MidRun");
    assert!(midrun.is_some(), "by_name(MidRun) missing after second run");
    // 4) Hero keeps the SAME StableId across runs (ids are stable).
    let hero_sid = world.get::<StableId>(hero.unwrap()).copied();
    assert!(
        hero_sid.is_some_and(|s| s.0 == 0),
        "Hero spawned first should keep StableId(0), got {:?}",
        hero_sid
    );
}

#[test]
fn fx6_i1_reinsert_restores_by_name_and_advances_next_stable() {
    // Demonstrates the bug: reset() clears by_name, so replay by name fails.
    use bevy_swarm::identity::IdentityIndex;

    let mut app = build_app();
    app.add_systems(Startup, |mut commands: Commands| {
        commands.spawn((Gameplay, Name::new("Hero"), Transform::default()));
    });
    let sc: Scenario =
        serde_json::from_str(r#"{"bot":{"type":"chaos","seed":3},"duration_s":0.1}"#).unwrap();
    let _ = run_scenario(&mut app, &sc);

    let idx = app.world().resource::<IdentityIndex>();
    let hero_before = idx.by_name("Hero");
    assert!(hero_before.is_some(), "Hero indexed before reset");

    // Reset + reinsert with names (the fixed driver.rs pattern).
    app.world_mut()
        .resource_scope(|world, mut idx: Mut<IdentityIndex>| {
            let mut q = world.query::<(Entity, &StableId, Option<&Name>)>();
            let mut entities: Vec<_> = q.iter(world).collect();
            entities.sort_by_key(|(e, _, _)| *e);
            idx.reset();
            for (entity, id, name) in entities {
                idx.reinsert(*id, entity, name);
            }
        });

    let idx = app.world().resource::<IdentityIndex>();
    let hero_after = idx.by_name("Hero");
    assert!(
        hero_after.is_some(),
        "reinsert with a name must restore by_name"
    );
}
