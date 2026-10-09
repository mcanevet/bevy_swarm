//! Z6: Infer game-owned types and gameplay entities from the game's crate path.
//!
//! The `Gameplay` marker is a contract piece that many oracles depend on.
//! Z6 infers which entities are "gameplay" by classifying component/resource
//! types as game-owned (based on the game plugin's crate path), then marking
//! entities whose archetype contains at least one game-owned component.
//! The real `Gameplay` marker is inserted on inferred entities so every
//! existing `With<Gameplay>` query works unchanged; the extra
//! `InferredGameplay` marker lets reports distinguish them.

use bevy::ecs::archetype::ArchetypeId;
use bevy::ecs::world::World;
use bevy::prelude::*;
use std::collections::HashSet;

use crate::contract::Gameplay;

/// Classifies component/resource types as game-owned vs engine/harness.
/// Inserted by `autotest()` from the game plugin's type name.
#[derive(Resource, Clone, Debug)]
pub struct GameTypes {
    /// Crate path prefixes considered game-owned, e.g. `["my_game::"]`.
    /// Derived from `std::any::type_name::<GamePlugin>()`.
    pub prefixes: Vec<String>,
}

impl GameTypes {
    /// True if the type name starts with any known game crate prefix.
    pub fn is_game_type(&self, type_name: &str) -> bool {
        self.prefixes
            .iter()
            .any(|p| type_name.starts_with(p.as_str()))
    }

    /// Extract game crate prefixes from a plugin's type name.
    /// Example: "my_game::GamePlugin" → `["my_game::"]`.
    pub fn from_plugin<P: Plugin + Send + Sync + 'static>() -> Self {
        let type_name = std::any::type_name::<P>();
        let prefix = type_name
            .split("::")
            .next()
            .map(|crate_name| format!("{}::", crate_name))
            .unwrap_or_default();
        Self {
            prefixes: vec![prefix],
        }
    }

    /// Add additional crate prefixes (for workspaces with multiple game
    /// crates, e.g. a sibling `my_game_core::Health`).
    pub fn with_crate(mut self, crate_name: &str) -> Self {
        self.prefixes.push(format!("{}::", crate_name));
        self
    }
}

/// Marker for entities tagged by inference (so reports can distinguish
/// explicit vs inferred gameplay entities). Sticky: never removed.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct InferredGameplay;

/// Engine component type-name PREFIXES excluded from gameplay
/// inference (cameras, lights, windows, UI nodes, observers).
const ENGINE_EXCLUDE_PREFIXES: &[&str] = &[
    "bevy_render::camera::Camera",
    "bevy_camera::camera::Camera",
    "bevy_core_pipeline::core_2d::camera_2d_bundle",
    "bevy_light::",
    "bevy_window::window::Window",
    "bevy_ui::",
    "bevy_observers::observer::Observer",
    "bevy_ecs::observer::Observer",
    "bevy_picking::",
];

/// Infer which archetypes represent gameplay entities: those containing
/// at least one game-owned component AND none of the engine-excluded types.
pub fn infer_gameplay_archetypes(world: &World, types: &GameTypes) -> HashSet<ArchetypeId> {
    let components = world.components();
    let mut game_component_ids = HashSet::new();
    let mut exclude_component_ids = HashSet::new();

    for (_cid, info) in components.iter_registered() {
        let type_name = info.name().to_string();
        let type_name = type_name.as_str();
        if types.is_game_type(type_name) {
            game_component_ids.insert(_cid);
        }
        if ENGINE_EXCLUDE_PREFIXES
            .iter()
            .any(|ex| type_name.starts_with(ex))
        {
            exclude_component_ids.insert(_cid);
        }
    }

    // Bevy 0.20 stores resources as entities (IsResource marker) —
    // they are not gameplay entities and must never be tagged.
    let is_resource_id =
        components.get_valid_id(std::any::TypeId::of::<bevy::ecs::resource::IsResource>());

    let mut result = HashSet::new();
    for arch in world.archetypes().iter() {
        if arch.is_empty() {
            continue;
        }
        if let Some(rid) = is_resource_id {
            if arch.contains(rid) {
                continue;
            }
        }
        let has_game = arch
            .components()
            .iter()
            .any(|cid| game_component_ids.contains(cid));
        let has_exclude = arch
            .components()
            .iter()
            .any(|cid| exclude_component_ids.contains(cid));
        if has_game && !has_exclude {
            result.insert(arch.id());
        }
    }
    result
}

/// Tag entities whose archetype is inferred as gameplay but that lack
/// the explicit `Gameplay` marker. Sticky: once tagged, never untagged
/// (so I1's StableIds stay stable). Inserts BOTH the real `Gameplay`
/// marker and `InferredGameplay`.
///
/// If the game placed ANY explicit `Gameplay` marker, inference is OFF
/// (explicit markers override; the driver reports this in scope).
pub fn tag_inferred_gameplay_entities(world: &mut World, types: &GameTypes) -> usize {
    // Explicit markers override: no inference when any exist. Only
    // GAME-placed markers count — harness-inferred tags (InferredGameplay)
    // must not disable inference for entities spawned later.
    let mut existing =
        world.query_filtered::<Entity, (With<Gameplay>, Without<InferredGameplay>)>();
    if existing.iter(world).next().is_some() {
        return 0;
    }
    let inferred_archetypes = infer_gameplay_archetypes(world, types);
    let mut entities_to_tag = Vec::new();
    for arch in world.archetypes().iter() {
        if !inferred_archetypes.contains(&arch.id()) {
            continue;
        }
        for entity_ref in arch.entities() {
            entities_to_tag.push(entity_ref.id());
        }
    }
    let mut tagged = 0usize;
    for eid in entities_to_tag {
        if let Ok(mut emut) = world.get_entity_mut(eid) {
            if !emut.contains::<InferredGameplay>() {
                emut.insert((Gameplay, InferredGameplay));
                tagged += 1;
            }
        }
    }
    tagged
}

/// Incremental inference: re-tag when the archetype count grew since
/// last run (archetypes are append-only). Cache the last seen length.
pub fn apply_gameplay_inference(world: &mut World) {
    let Some(types) = world.get_resource::<GameTypes>() else {
        return;
    };
    let types = types.clone();
    let last = world
        .resource::<crate::state::PlaytestState>()
        .inference_last_archetype_len;
    let entities = world.entities().len();
    let last_entities = world
        .resource::<crate::state::PlaytestState>()
        .inference_last_entity_len;
    let current = world.archetypes().len();
    if current == last && entities == last_entities {
        return;
    }
    let tagged = tag_inferred_gameplay_entities(world, &types);
    let _ = tagged;
    let mut state = world.resource_mut::<crate::state::PlaytestState>();
    state.inference_last_archetype_len = current;
    state.inference_last_entity_len = entities;
}

/// True if gameplay inference is active (GameTypes present and no
/// explicit Gameplay marker exists yet).
pub fn inference_active(world: &World) -> bool {
    world.get_resource::<GameTypes>().is_some()
        && world
            .components()
            .get_valid_id(std::any::TypeId::of::<Gameplay>())
            .is_none_or(|gid| {
                // Only GAME-placed Gameplay markers disable inference.
                // Harness-inferred tags live in archetypes that also
                // carry InferredGameplay.
                let inferred = world
                    .components()
                    .get_valid_id(std::any::TypeId::of::<InferredGameplay>());
                world
                    .archetypes()
                    .iter()
                    .all(|a| !a.contains(gid) || inferred.is_some_and(|iid| a.contains(iid)))
            })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Local crate-simulating components: names resolve to
    // "bevy_swarm::game_types::tests::<Name>" — a "game" prefix here.
    #[derive(Component)]
    struct PlayerHealth;

    #[derive(Component)]
    struct EnemyDamage;

    #[derive(Component)]
    struct Cam2d;

    fn types() -> GameTypes {
        GameTypes {
            prefixes: vec!["bevy_swarm::game_types".to_string()],
        }
    }

    #[test]
    fn infers_player_and_enemy_not_camera_or_ui() {
        let mut world = World::new();
        world.spawn(PlayerHealth);
        world.spawn(EnemyDamage);
        world.spawn((Cam2d, bevy::camera::Camera::default()));
        let archetypes = infer_gameplay_archetypes(&world, &types());
        // Two gameplay archetypes (player, enemy); camera excluded.
        assert_eq!(archetypes.len(), 2);
    }

    #[test]
    fn explicit_marker_disables_inference() {
        let mut world = World::new();
        world.spawn((PlayerHealth, Gameplay));
        world.spawn(EnemyDamage); // would be inferred normally
        let tagged = tag_inferred_gameplay_entities(&mut world, &types());
        assert_eq!(tagged, 0, "explicit Gameplay marker disables inference");
    }

    #[test]
    fn non_reflect_components_classified() {
        // PlayerHealth has no Reflect derive; classification works via
        // ComponentInfo::name() anyway.
        let types = types();
        assert!(types.is_game_type("bevy_swarm::game_types::PlayerHealth"));
        assert!(!types.is_game_type("bevy_transform::Transform"));
    }

    #[test]
    fn inference_tags_new_entities_after_first_pass() {
        let mut world = World::new();
        world.spawn(PlayerHealth);
        let tagged = tag_inferred_gameplay_entities(&mut world, &types());
        assert_eq!(tagged, 1);
        let mut q = world.query_filtered::<bevy::ecs::entity::Entity, With<PlayerHealth>>();
        let e = q.single(&world).unwrap();
        assert!(world.entity(e).contains::<Gameplay>());
        assert!(world.entity(e).contains::<InferredGameplay>());
        // Second run: NEW entities spawned in the same archetype ARE tagged.
        // (Sticky only means we don't untag already-tagged entities.)
        world.spawn(PlayerHealth);
        let tagged2 = tag_inferred_gameplay_entities(&mut world, &types());
        let arch_count = world.archetypes().len();
        assert_eq!(
            tagged2, 1,
            "new entity in same archetype gets tagged (archetypes={})",
            arch_count
        );
    }

    #[test]
    fn inference_skips_is_resource_archetypes() {
        // Spawn a resource entity (Bevy 0.20 stores resources as entities)
        // and ensure it never gets tagged as Gameplay.
        use bevy::ecs::resource::IsResource;
        let mut world = World::new();
        let cid = world.register_component::<PlayerHealth>();
        let res_e = world.spawn(IsResource::new(cid)).id();
        let tagged = tag_inferred_gameplay_entities(&mut world, &types());
        assert_eq!(tagged, 0);
        assert!(!world.entity(res_e).contains::<Gameplay>());
    }
}
