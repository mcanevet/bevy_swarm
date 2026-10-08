//! I1: Identity index + deterministic StableId for Gameplay entities.
//!
//! Provides O(1) entity lookup by name or stable ID, replacing per-frame
//! linear scans. StableId is assigned on Gameplay insertion and persists,
//! enabling replay of unnamed entities.
//!
//! Observers maintain the index incrementally; the same deterministic
//! scenario assigns the same StableId to the "same" entity across runs,
//! so replays can target unnamed entities by StableId.

use bevy::prelude::*;
use std::collections::HashMap;

use crate::contract::Gameplay;

/// Deterministic identity for Gameplay entities: assigned in insertion
/// order of the `Gameplay` marker. Under a deterministic run (A1–A3),
/// the same scenario assigns the same StableId to the "same" entity,
/// so replays can target unnamed entities.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct StableId(pub u64);

/// Name -> entities and StableId -> entity, maintained by observers.
/// O(1) lookups replace per-frame linear scans.
#[derive(Resource, Default)]
pub struct IdentityIndex {
    pub(crate) next_stable: u64,
    /// Map from StableId to Entity (BTreeMap: stable_ids() iteration
    /// is in StableId order, deterministic — HashMap was random order).
    by_stable: std::collections::BTreeMap<StableId, Entity>,
    /// Multiple entities may share a Name; kept in insertion (StableId)
    /// order so "first match" is deterministic.
    by_name: HashMap<String, Vec<(StableId, Entity)>>,
}

impl IdentityIndex {
    /// Lookup by name (lowest-StableId match — deterministic regardless
    /// of archetype/iteration order).
    pub fn by_name(&self, name: &str) -> Option<Entity> {
        self.by_name
            .get(name)
            .and_then(|v| v.iter().min_by_key(|(sid, _)| *sid).map(|(_, e)| *e))
    }

    /// Lookup by stable ID.
    pub fn by_stable(&self, id: StableId) -> Option<Entity> {
        self.by_stable.get(&id).copied()
    }

    /// All entities currently indexed, in StableId order.
    pub fn stable_ids(&self) -> impl Iterator<Item = (StableId, Entity)> + '_ {
        self.by_stable.iter().map(|(k, v)| (*k, *v))
    }

    /// Number of indexed entities.
    pub fn len(&self) -> usize {
        self.by_stable.len()
    }

    /// Whether the index is empty.
    pub fn is_empty(&self) -> bool {
        self.by_stable.is_empty()
    }

    /// Remove an entity from the index (despawn/remove cleanup).
    pub(crate) fn unindex(&mut self, id: StableId, entity: Entity) {
        self.by_stable.remove(&id);
        for vec in self.by_name.values_mut() {
            vec.retain(|(sid, ent)| *sid != id || *ent != entity);
        }
    }

    /// Reset the index (call at run start; ids are relative to run start).
    pub fn reset(&mut self) {
        self.next_stable = 0;
        self.by_stable.clear();
        self.by_name.clear();
    }

    /// FX6 I1: re-insert an existing (StableId, Entity) pair after a
    /// reset (entities that survived from App build / previous run).
    pub fn reinsert(&mut self, id: StableId, entity: Entity) {
        self.by_stable.insert(id, entity);
    }

    /// Index an entity under a name (observer helper).
    pub(crate) fn index_name(&mut self, id: StableId, entity: Entity, name: &str) {
        self.by_name
            .entry(name.to_string())
            .or_default()
            .push((id, entity));
    }
}

/// Observer: Gameplay added -> assign StableId + index.
pub(crate) fn on_add_gameplay(
    event: On<Add<Gameplay>>,
    mut commands: Commands,
    mut idx: ResMut<IdentityIndex>,
    names: Query<&Name>,
) {
    let e = event.entity;
    let id = StableId(idx.next_stable);
    idx.next_stable += 1;
    idx.by_stable.insert(id, e);
    if let Ok(n) = names.get(e) {
        idx.index_name(id, e, n.as_ref());
    }
    commands.entity(e).insert(id);
}

/// Observer: Gameplay removed/despawned -> unindex.
pub(crate) fn on_remove_gameplay(
    event: On<Remove<Gameplay>>,
    mut idx: ResMut<IdentityIndex>,
    ids: Query<&StableId>,
) {
    let e = event.entity;
    if let Ok(&id) = ids.get(e) {
        idx.unindex(id, e);
    }
}

/// Observer: Name added -> index (entity must already have a StableId).
pub(crate) fn on_add_name(
    event: On<Add<Name>>,
    mut idx: ResMut<IdentityIndex>,
    ids: Query<&StableId>,
    names: Query<&Name>,
) {
    let e = event.entity;
    if let (Ok(&id), Ok(n)) = (ids.get(e), names.get(e)) {
        idx.index_name(id, e, n.as_ref());
    }
}

/// Observer: Name replaced -> reindex under the new name.
/// Note: On<Discard<Name>> fires BEFORE the old value is overwritten;
/// the Query sees the OLD name, so we unindex the old and index the
/// new value is observed by the subsequent On<Insert<Name>>.
pub(crate) fn on_replace_name(
    event: On<Discard<Name>>,
    mut idx: ResMut<IdentityIndex>,
    ids: Query<&StableId>,
    names: Query<&Name>,
) {
    let e = event.entity;
    if let (Ok(&id), Ok(old)) = (ids.get(e), names.get(e)) {
        // Remove the old name entry.
        if let Some(vec) = idx.by_name_name_mut(old.as_ref()) {
            vec.retain(|(sid, ent)| *sid != id || *ent != e);
        }
        // The new value arrives via on_insert_name below.
    }
}

impl IdentityIndex {
    pub(crate) fn by_name_name_mut(&mut self, name: &str) -> Option<&mut Vec<(StableId, Entity)>> {
        self.by_name.get_mut(name)
    }
}

/// Observer: Name inserted (covers replacement new-value too).
pub(crate) fn on_insert_name(
    event: On<Insert<Name>>,
    mut idx: ResMut<IdentityIndex>,
    ids: Query<&StableId>,
    names: Query<&Name>,
) {
    let e = event.entity;
    if let (Ok(&id), Ok(n)) = (ids.get(e), names.get(e)) {
        // Deduplicate (Insert also fires for fresh Adds).
        if let Some(vec) = idx.by_name_name_mut(n.as_ref()) {
            if vec.iter().any(|(sid, ent)| *sid == id && *ent == e) {
                return;
            }
        }
        idx.index_name(id, e, n.as_ref());
    }
}

/// Marker: observers already installed (install_identity is idempotent).
#[derive(Resource, Default)]
pub struct IdentityInstalled;

/// Register identity observers + init resources on an App.
/// Called idempotently from PlaytestPlugin and TestConventionsPlugin.
pub fn install_identity(app: &mut App) {
    if app.world().contains_resource::<IdentityInstalled>() {
        return;
    }
    if !app.world().contains_resource::<IdentityIndex>() {
        app.init_resource::<IdentityIndex>();
    }
    app.add_observer(on_add_gameplay);
    app.add_observer(on_remove_gameplay);
    app.add_observer(on_add_name);
    app.add_observer(on_replace_name);
    app.add_observer(on_insert_name);
    app.insert_resource(IdentityInstalled);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_lookups_work() {
        let mut idx = IdentityIndex::default();
        let e1 = Entity::from_bits(1);
        let e2 = Entity::from_bits(2);
        let id1 = StableId(0);
        let id2 = StableId(1);

        idx.by_stable.insert(id1, e1);
        idx.by_stable.insert(id2, e2);
        idx.index_name(id1, e1, "Player");
        idx.index_name(id2, e2, "Player");

        assert_eq!(idx.by_name("Player"), Some(e1));
        assert_eq!(idx.by_stable(id1), Some(e1));
        assert_eq!(idx.by_stable(StableId(99)), None);
        assert_eq!(idx.len(), 2);
    }

    #[test]
    fn unindex_removes_entries() {
        let mut idx = IdentityIndex::default();
        let e1 = Entity::from_bits(1);
        let id1 = StableId(0);
        idx.by_stable.insert(id1, e1);
        idx.index_name(id1, e1, "Player");
        idx.unindex(id1, e1);
        assert!(idx.is_empty());
        assert_eq!(idx.by_name("Player"), None);
    }
}
