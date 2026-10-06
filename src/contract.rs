use bevy::ecs::resource::Resource;
use bevy::ecs::world::World;
use bevy::prelude::*;
use std::collections::HashMap;

/// Input Abstraction — gameplay systems consume these; never ButtonInput.
/// Buffered events are `Message`s — registered via `app.add_message::<M>()`,
/// written/read via `MessageWriter`/`MessageReader`. `Event`/observers is
/// the reactive system, NOT the buffered-event path.
#[derive(Message, Debug, Clone)]
pub enum UserIntent {
    Move { dir: Vec2 },
    Select { target: bevy::ecs::entity::Entity },
    Choice { index: usize },
    Axis { name: String, value: f32 },
    Wait,
}

/// Which intent variants the game actually consumes — the game's declared
/// input surface. The chaos bot samples ONLY from this list (it refuses to
/// run against an empty surface), so every game's primary verbs are
/// exercised, and no harness-side genre knowledge is needed.
#[derive(Clone, Debug)]
pub enum SurfaceVariant {
    Move,
    /// Max legal choice index (dialogue options, menu entries, cards...)
    Choice(usize),
    /// Axis name the game reads (e.g. "throttle", "steer_x")
    Axis(String),
    Select,
    Wait,
}

#[derive(Resource, Default)]
pub struct IntentSurface(pub Vec<SurfaceVariant>);

impl IntentSurface {
    /// Declare the game's input surface. Example:
    /// `IntentSurface::new(vec![SurfaceVariant::Move, SurfaceVariant::Choice(3)])`
    pub fn new(variants: Vec<SurfaceVariant>) -> Self {
        Self(variants)
    }
}

/// State Exposure — the game's public test surface. This crate ships a
/// default `TestApi` resource; games normally **keep defining their own**
/// in `src/test_conventions.rs` (deriving `Reflect`, implementing
/// `TestApiResolve` — games implement this to expose state.
/// The crate's `TestConventionsPlugin` registers the DEFAULT shape;
/// games with a custom TestApi register their own instead and simply
/// satisfy the same resolve interface.
#[derive(Resource, Default, Clone, Reflect)]
#[reflect(Resource)]
pub struct TestApi {
    pub score: i64,
    pub active_players: usize,
    /// Game-defined numeric fields (path without "TestApi." prefix).
    /// Populated by the game's sync system; resolved before the reflect
    /// fallback. Lets games extend the test surface WITHOUT defining a
    /// separate TestApi resource (the harness resolves paths uniformly).
    pub custom_numeric: HashMap<String, f64>,
    /// Game-defined text fields, same semantics as custom_numeric.
    pub custom_text: HashMap<String, String>,
}

impl TestApiResolve for TestApi {
    fn resolve(&self, path: &str) -> Option<TestFieldValue> {
        let field = path.strip_prefix("TestApi.")?;
        // Fast path: hand-written match (covers named default fields).
        match field {
            "score" => return Some(TestFieldValue::Numeric(self.score as f64)),
            "active_players" => return Some(TestFieldValue::Numeric(self.active_players as f64)),
            _ => {}
        }
        // Game-defined custom fields (checked before the reflect fallback
        // so games can also shadow nothing — these are pure additions).
        if let Some(n) = self.custom_numeric.get(field) {
            return Some(TestFieldValue::Numeric(*n));
        }
        if let Some(t) = self.custom_text.get(field) {
            return Some(TestFieldValue::Text(t.clone()));
        }
        // Reflect fallback: any reflected numeric/String field.
        self.get_field_dynamic(field)
    }
}

impl TestApi {
    /// Immutable snapshot for two-phase invariant checking
    /// (borrow-checker workaround: snapshot, then mutate violations).
    pub fn clone_state(&self) -> Self {
        self.clone()
    }

    fn get_field_dynamic(&self, field: &str) -> Option<TestFieldValue> {
        if let Some(n) = self.get_field::<f64>(field) {
            return Some(TestFieldValue::Numeric(*n));
        }
        if let Some(n) = self.get_field::<f32>(field) {
            return Some(TestFieldValue::Numeric(*n as f64));
        }
        if let Some(n) = self.get_field::<i64>(field) {
            return Some(TestFieldValue::Numeric(*n as f64));
        }
        if let Some(n) = self.get_field::<usize>(field) {
            return Some(TestFieldValue::Numeric(*n as f64));
        }
        if let Some(s) = self.get_field::<String>(field) {
            return Some(TestFieldValue::Text(s.clone()));
        }
        None
    }
}

/// A resolved TestApi field value — numeric or textual.
#[derive(Debug, Clone, PartialEq)]
pub enum TestFieldValue {
    Numeric(f64),
    Text(String),
}

impl TestFieldValue {
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            TestFieldValue::Numeric(n) => Some(*n),
            _ => None,
        }
    }

    pub fn as_text(&self) -> Option<&str> {
        match self {
            TestFieldValue::Text(s) => Some(s),
            _ => None,
        }
    }
}

/// Trait a game's TestApi resource implements so the harness can resolve
/// "TestApi.\<field\>" paths without knowing the concrete type.
pub trait TestApiResolve {
    fn resolve(&self, path: &str) -> Option<TestFieldValue>;
}

/// Hook closure type shared by ResetHooks, CheatHooks, and NamedIntents.
pub type WorldHook = Box<dyn Fn(&mut World) + Send + Sync>;

/// Reset Hooks — typed resets, no string method dispatch.
/// Fn bounds must be Send + Sync for the Resource derive.
#[derive(Resource, Default)]
pub struct ResetHooks(pub std::collections::HashMap<String, WorldHook>);

impl ResetHooks {
    pub fn register<F: Fn(&mut World) + Send + Sync + 'static>(&mut self, name: &str, f: F) {
        self.0.insert(name.to_string(), Box::new(f));
    }

    pub fn known_kinds(&self) -> Vec<&str> {
        self.0.keys().map(|s| s.as_str()).collect()
    }
}

/// Genre markers.
#[derive(Component, Reflect)]
#[reflect(Component)]
pub struct RealTime;
#[derive(Component, Reflect)]
#[reflect(Component)]
pub struct TurnBased;
#[derive(Component, Reflect)]
#[reflect(Component)]
pub struct AnalogControls;

/// Entities that can leave the play area — bounds/NaN invariants check
/// these; structural/UI entities (legitimately at origin) are excluded.
#[derive(Component, Reflect)]
#[reflect(Component)]
pub struct Gameplay;

/// Convenience plugin bundling the contract's moving parts.
///
/// Note: this plugin is convenience
/// sugar for TEST Apps. The game's own root plugin MUST ALSO call
/// `add_message::<UserIntent>()` (and init any resource its systems
/// read) — otherwise the production binary panics at boot with
/// "Message not initialized" while test Apps pass (they add this
/// plugin). Registration is idempotent; duplicate calls are harmless.
///
/// GAMES USING A CUSTOM TestApi: do NOT add this plugin's sync system;
/// register the message/resources in your root plugin and implement
/// `TestApiResolve` on your own resource. This plugin exists for the
/// template's default TestApi shape.
pub struct TestConventionsPlugin;

impl Plugin for TestConventionsPlugin {
    fn build(&self, app: &mut App) {
        use bevy::ecs::reflect::AppTypeRegistry;

        app.add_message::<UserIntent>()
            .init_resource::<TestApi>()
            .init_resource::<ResetHooks>()
            .init_resource::<IntentSurface>()
            .init_resource::<CheatHooks>()
            .init_resource::<ActionLog>()
            .init_resource::<GameVersion>();

        // Register reflective types for query-target invariants.
        let registry = app.world_mut().resource_mut::<AppTypeRegistry>();
        let mut reg = registry.0.write();
        reg.register::<Gameplay>();
        reg.register::<RealTime>();
        reg.register::<TurnBased>();
        reg.register::<AnalogControls>();
        // Name is built-in Bevy, already registered.
    }
}

/// Cheat Hooks — typed cheat commands (e.g., "god_mode", "instant_build").
/// Cheats are AUDITED: every invocation is logged and reported.
#[derive(Resource, Default)]
pub struct CheatHooks(pub HashMap<String, WorldHook>);

impl CheatHooks {
    pub fn register<F: Fn(&mut World) + Send + Sync + 'static>(&mut self, name: &str, f: F) {
        self.0.insert(name.to_string(), Box::new(f));
    }

    pub fn known_kinds(&self) -> Vec<&str> {
        self.0.keys().map(|s| s.as_str()).collect()
    }
}

/// Action Log — records every injected input and cheat invocation.
/// Used for audit trails, replay, and LLM training data.
#[derive(Resource, Default, serde::Serialize)]
pub struct ActionLog {
    /// Entries ordered by frame/time of occurrence.
    pub entries: Vec<ActionEntry>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct ActionEntry {
    pub frame: u64,
    pub elapsed_ms: f64,
    pub source: ActionSource,
    pub action: String,
    pub details: Option<String>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub enum ActionSource {
    /// From a headless bot scenario.
    BotScenario(String), // scenario name
    /// From the resident agent (BRP).
    Agent,
    /// Manual/external injection.
    Manual,
}

impl ActionLog {
    pub fn record(
        &mut self,
        frame: u64,
        elapsed_ms: f64,
        source: ActionSource,
        action: impl Into<String>,
        details: Option<String>,
    ) {
        self.entries.push(ActionEntry {
            frame,
            elapsed_ms,
            source,
            action: action.into(),
            details,
        });
    }

    /// Cheats recorded so far (subset labelled `cheat:`).
    pub fn cheat_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|e| e.action.starts_with("cheat:"))
            .count()
    }
}

/// Version stamp — identifies the exact game build in reports.
#[derive(Resource, Default)]
pub struct GameVersion(pub String);

/// Harness-owned type paths excluded from auto-discovery and feature
/// detection to ignore harness internals.
pub const HARNESS_TYPE_PATHS: &[&str] = &["bevy_swarm::contract::", "bevy_swarm::harness::"];

/// Named intent handlers — games register closures that translate a named
/// intent (e.g., "end_turn") into concrete gameplay effects. The agent can
/// then emit `playtest/intent {"intent":"NamedMove","name":"end_turn"}` and
/// the harness will invoke the registered handler. This removes the need
/// for a custom UserIntent enum when the game has simple named actions.
#[derive(Resource, Default)]
pub struct NamedIntents(pub std::collections::HashMap<String, WorldHook>);

impl NamedIntents {
    pub fn register(&mut self, name: &str, handler: impl Fn(&mut World) + Send + Sync + 'static) {
        self.0.insert(name.to_string(), Box::new(handler));
    }

    /// Static dispatch helper that avoids double-borrow of the world
    /// (takes the handler out, runs it, puts it back).
    pub fn dispatch(world: &mut World, name: &str) -> bool {
        let handler = world.resource_mut::<NamedIntents>().0.remove(name);
        match handler {
            Some(h) => {
                h(world);
                world
                    .resource_mut::<NamedIntents>()
                    .0
                    .insert(name.to_string(), h);
                true
            }
            None => false,
        }
    }
}

/// Deterministic RNG seed for a scenario run — games with randomness
/// read this resource to seed their own generators, so chaos-bot runs
/// are reproducible and flaky failure investigations can replay
/// exactly. Set by the harness at scenario start; defaults to 0.
#[derive(Resource, Clone, Copy, Debug, Reflect)]
#[reflect(Resource)]
#[derive(Default)]
pub struct ScenarioSeed(pub u64);
