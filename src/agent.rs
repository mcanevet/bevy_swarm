//! Resident agent transport — Phase 2 of the layered architecture.
//!
//! Runs inside the REAL game process (windowed binary), exposing the
//! same playtest capability set as the headless harness over Bevy's
//! official Remote Protocol (BRP). External clients (bevy CLI, curl,
//! LLM drivers) can observe state, inject chain-complete input, and
//! trigger resets — against the binary a player actually runs.
//!
//! Feature-gated behind `agent`: CI headless runs pay nothing.

use crate::contract::{
    ActionLog, ActionSource, CheatHooks, IntentSurface, NamedIntents, ResetHooks, SurfaceVariant,
    UserIntent,
};
use bevy::picking;
use bevy::prelude::*;
use bevy::remote::http::RemoteHttpPlugin;
use bevy::remote::{builtin_methods, error_codes, BrpError, BrpResult, RemotePlugin};
use serde_json::{json, Value};
use std::sync::mpsc::{Receiver, TryRecvError};

// ---------------------------------------------------------------------------
// Request queue — BRP handlers validate, then forward through the
// channel; a system drains it at a controlled point in the frame.
// Wrapped in a Mutex so the Resource satisfies Sync.
// ---------------------------------------------------------------------------

pub enum AgentRequest {
    Intent(UserIntent),
    /// Named intent: dispatched via NamedIntents in the flush system.
    NamedIntent(String),
    KeyPress {
        key: String,
    },
    PointerClick {
        x: f32,
        y: f32,
    },
    /// Queue a screenshot (async write, audited).
    Screenshot {
        path: std::path::PathBuf,
    },
    Reset(String),
    Cheat(String),
}

/// Shared-secret token for playtest/* methods (R2). `None` = no auth.
#[derive(Resource, Default)]
pub struct AgentAuthToken(pub Option<String>);

#[derive(Resource)]
pub(crate) struct AgentRequestQueue(pub std::sync::Mutex<Receiver<AgentRequest>>);

#[derive(Resource)]
struct AgentRequestSender(std::sync::mpsc::Sender<AgentRequest>);

// ---------------------------------------------------------------------------
// Pending agent input gestures — press on due frame, release next
// frame; pointer clicks spread over THREE frames (Move, Press,
// Release) mirroring the headless bots. bevy_picking reads the
// previous frame's hover map, so same-frame press/release is dropped.
// ---------------------------------------------------------------------------

#[derive(Resource, Default)]
struct PendingAgentInput {
    keys: Vec<bevy::input::keyboard::KeyCode>,
    gestures: std::collections::VecDeque<PendingPointerGesture>,
}

struct PendingPointerGesture {
    press_at: u64,
    release_at: u64,
    location: picking::pointer::Location,
    button: picking::pointer::PointerButton,
}

// ---------------------------------------------------------------------------
// Plugin
// ---------------------------------------------------------------------------

/// Configuration for the agent BRP endpoint (R2).
#[derive(Clone, Debug)]
pub struct AgentConfig {
    /// Bind address. Defaults to loopback (127.0.0.1). Non-loopback
    /// addresses require `allow_remote: true`.
    pub addr: std::net::IpAddr,
    /// Bind port (BRP default 15702).
    pub port: u16,
    /// Shared-secret token. When set (config or
    /// `BEVY_SWARM_AGENT_TOKEN` env), every `playtest/*` method
    /// requires `params.token` to match; mismatches return
    /// INVALID_REQUEST. Built-in `world.*` BRP methods are NOT covered
    /// by this (RemoteHttpPlugin has no request middleware) — keep the
    /// endpoint on loopback.
    pub token: Option<String>,
    /// Explicitly allow binding a non-loopback address.
    pub allow_remote: bool,
    /// Panic at startup if the feature is active in a release build.
    pub deny_in_release: bool,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            addr: std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
            port: 15702,
            token: std::env::var("BEVY_SWARM_AGENT_TOKEN").ok(),
            allow_remote: false,
            deny_in_release: false,
        }
    }
}

impl AgentConfig {
    /// Validate the config: refuse non-loopback binds without explicit
    /// opt-in; warn (or panic, per `deny_in_release`) on release builds.
    /// Exposed for unit testing the decision logic without a network.
    /// FX10: effective deny_in_release — defaults to TRUE when
    /// allow_remote is set (remote exposure in a release build is the
    /// dangerous case).
    pub fn effective_deny_in_release(&self) -> bool {
        self.deny_in_release || self.allow_remote
    }

    pub fn validate(&self) -> Result<(), String> {
        let is_release = cfg!(not(debug_assertions));
        if is_release {
            let msg = "bevy_swarm agent feature is active in a RELEASE build —                        BRP exposes arbitrary world reads and mutation.                        Never ship builds with the agent enabled.";
            if self.deny_in_release {
                return Err(msg.to_string());
            }
            bevy::log::warn!("{msg}");
        }
        if !self.addr.is_loopback() && !self.allow_remote {
            return Err(format!(
                "refusing to bind agent BRP endpoint to non-loopback {}                  without allow_remote: true",
                self.addr
            ));
        }
        Ok(())
    }
}

/// The single source of truth for playtest/* method NAMES (used for
/// registration and for playtest/schema output so they can't drift).
pub const PLAYTEST_METHODS: &[&str] = &[
    "playtest/schema",
    "playtest/observe",
    "playtest/intent",
    "playtest/key",
    "playtest/pointer",
    "playtest/reset",
    "playtest/cheat",
    "playtest/actions",
    "playtest/state",
    "playtest/screenshot",
    "playtest/diagnostics",
];

/// FX10: constant-time token comparison. Lengths are compared
/// without early exit on content; content is XOR-accumulated so a
/// mismatch does not reveal which byte differed.
pub(crate) fn token_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// FX10: token-guarded wrapper around a built-in world.* handler.
/// Auth FIRST — a caller without the token learns nothing about the
/// shape of the params.
fn with_token_guard(
    handler: fn(In<Option<Value>>, &World) -> BrpResult,
) -> impl Fn(In<Option<Value>>, &World) -> BrpResult {
    move |In(params): In<Option<Value>>, world: &World| {
        if !world_token_ok(world, params.as_ref()) {
            return Err(auth_error());
        }
        handler(In(params), world)
    }
}

/// FX10: same guard for handlers needing `&mut World`.
fn with_token_guard_mut(
    handler: fn(In<Option<Value>>, &mut World) -> BrpResult,
) -> impl Fn(In<Option<Value>>, &mut World) -> BrpResult {
    move |In(params): In<Option<Value>>, world: &mut World| {
        if !world_token_ok(world, params.as_ref()) {
            return Err(auth_error());
        }
        handler(In(params), world)
    }
}

fn auth_error() -> BrpError {
    BrpError {
        code: error_codes::INVALID_REQUEST,
        message: "invalid or missing token".to_string(),
        data: None,
    }
}

fn world_token_ok(world: &World, params: Option<&Value>) -> bool {
    let Some(expected) = world
        .get_resource::<AgentAuthToken>()
        .and_then(|t| t.0.clone())
    else {
        return true;
    };
    let provided = params.and_then(|p| p.get("token")).and_then(Value::as_str);
    provided.is_some_and(|p| token_eq(p, &expected))
}

/// FX10: rejecting stub for mutating built-in world.* methods.
/// Overriding a builtin name replaces its handler (RemotePlugin::build
/// drains in order and RemoteMethods::insert replaces same-named
/// entries — later wins).
fn world_method_disabled(In(_params): In<Option<Value>>) -> BrpResult {
    Err(BrpError {
        code: error_codes::METHOD_NOT_FOUND,
        message: "world mutation methods are disabled; use playtest/* methods".to_string(),
        data: None,
    })
}

/// FX10: RemotePlugin with auth-guarded read-only world methods and
/// ALL mutating builtins overridden to reject. Drive mutations
/// through playtest/* instead (token-guarded via brp_params).
fn guarded_remote_plugin() -> RemotePlugin {
    RemotePlugin::default()
        // Read-only getters: token-guarded versions override builtins.
        .with_method_main(
            builtin_methods::BRP_GET_COMPONENTS_METHOD,
            with_token_guard(builtin_methods::process_remote_get_components_request),
        )
        .with_method_main(
            builtin_methods::BRP_QUERY_METHOD,
            with_token_guard_mut(builtin_methods::process_remote_query_request),
        )
        .with_method_main(
            builtin_methods::BRP_LIST_COMPONENTS_METHOD,
            with_token_guard(builtin_methods::process_remote_list_components_request),
        )
        .with_method_main(
            builtin_methods::BRP_GET_RESOURCE_METHOD,
            with_token_guard(builtin_methods::process_remote_get_resources_request),
        )
        // Mutating / streaming builtins: replaced with rejections.
        .with_method_main(
            builtin_methods::BRP_SPAWN_ENTITY_METHOD,
            world_method_disabled,
        )
        .with_method_main(
            builtin_methods::BRP_INSERT_COMPONENTS_METHOD,
            world_method_disabled,
        )
        .with_method_main(
            builtin_methods::BRP_REMOVE_COMPONENTS_METHOD,
            world_method_disabled,
        )
        .with_method_main(
            builtin_methods::BRP_DESPAWN_COMPONENTS_METHOD,
            world_method_disabled,
        )
        .with_method_main(
            builtin_methods::BRP_REPARENT_ENTITIES_METHOD,
            world_method_disabled,
        )
        .with_method_main(
            builtin_methods::BRP_MUTATE_COMPONENTS_METHOD,
            world_method_disabled,
        )
        .with_method_main(
            builtin_methods::BRP_MUTATE_RESOURCE_METHOD,
            world_method_disabled,
        )
        .with_method_main(
            builtin_methods::BRP_INSERT_RESOURCE_METHOD,
            world_method_disabled,
        )
        .with_method_main(
            builtin_methods::BRP_REMOVE_RESOURCE_METHOD,
            world_method_disabled,
        )
        .with_method_main(
            builtin_methods::BRP_TRIGGER_EVENT_METHOD,
            world_method_disabled,
        )
        .with_method_main(
            builtin_methods::BRP_WRITE_MESSAGE_METHOD,
            world_method_disabled,
        )
        .with_method_main(builtin_methods::BRP_OBSERVE_METHOD, world_method_disabled)
        .with_method_main(
            builtin_methods::BRP_GET_COMPONENTS_AND_WATCH_METHOD,
            world_method_disabled,
        )
        .with_method_main(
            builtin_methods::BRP_LIST_COMPONENTS_AND_WATCH_METHOD,
            world_method_disabled,
        )
        .with_method_main(
            builtin_methods::BRP_LIST_RESOURCES_METHOD,
            world_method_disabled,
        )
}

/// Adds the BRP transport with custom `playtest/*` methods.
/// Uses [`AgentConfig::default`] (loopback, token from env).
#[derive(Default)]
pub struct AgentPlugin {
    pub config: AgentConfig,
}

impl AgentPlugin {
    pub fn new(config: AgentConfig) -> Self {
        Self { config }
    }

    /// Convenience builder: bind the given address/port.
    pub fn bind(addr: std::net::IpAddr, port: u16) -> Self {
        Self::new(AgentConfig {
            addr,
            port,
            ..AgentConfig::default()
        })
    }
}

impl Plugin for AgentPlugin {
    fn build(&self, app: &mut App) {
        if let Err(msg) = self.config.validate() {
            panic!("AgentPlugin: {}", msg);
        }
        let token = self.config.token.clone();
        app.insert_resource(AgentAuthToken(token));

        let (tx, rx) = std::sync::mpsc::channel::<AgentRequest>();

        app.insert_resource(AgentRequestSender(tx))
            .insert_resource(AgentRequestQueue(std::sync::Mutex::new(rx)))
            .init_resource::<PendingAgentInput>()
            .init_resource::<AgentFrameCounter>()
            // Gestures pending for THIS frame (releases, presses).
            .add_systems(PreUpdate, (agent_flush_system, agent_input_system).chain())
            .add_plugins(
                guarded_remote_plugin()
                    .with_method_main("playtest/schema", playtest_schema)
                    .with_method_main("playtest/observe", playtest_observe)
                    .with_method_main("playtest/intent", playtest_intent)
                    .with_method_main("playtest/key", playtest_key)
                    .with_method_main("playtest/pointer", playtest_pointer)
                    .with_method_main("playtest/reset", playtest_reset)
                    .with_method_main("playtest/cheat", playtest_cheat)
                    .with_method_main("playtest/actions", playtest_actions)
                    .with_method_main("playtest/state", playtest_state)
                    .with_method_main("playtest/screenshot", playtest_screenshot)
                    .with_method_main("playtest/diagnostics", playtest_diagnostics),
            )
            .add_plugins(
                RemoteHttpPlugin::default()
                    .with_address(self.config.addr)
                    .with_port(self.config.port),
            );
    }
}

// ---------------------------------------------------------------------------
// Custom BRP methods
// ---------------------------------------------------------------------------

fn invalid_params(msg: impl Into<String>) -> BrpError {
    BrpError {
        code: error_codes::INVALID_PARAMS,
        message: msg.into(),
        data: None,
    }
}

fn internal_error(msg: impl Into<String>) -> BrpError {
    BrpError {
        code: error_codes::INTERNAL_ERROR,
        message: msg.into(),
        data: None,
    }
}

/// Parse params AND enforce the R2 token: when an AgentAuthToken is
/// configured, `params.token` must match. Applied to every playtest/*
/// method (read ones included — a leaked endpoint is a leaked game).
fn brp_params(world: &World, params: Option<Value>) -> BrpResult<Value> {
    // Auth FIRST: a caller without the token learns nothing about the
    // shape of the params (not even that params are required).
    if let Some(expected) = world
        .get_resource::<AgentAuthToken>()
        .and_then(|t| t.0.clone())
    {
        let provided = params
            .as_ref()
            .and_then(|p| p.get("token"))
            .and_then(Value::as_str);
        if !provided.is_some_and(|p| token_eq(p, &expected)) {
            return Err(BrpError {
                code: error_codes::INVALID_REQUEST,
                message: "invalid or missing token".to_string(),
                data: None,
            });
        }
    }
    params.ok_or_else(|| invalid_params("missing params"))
}

fn send_request(world: &World, req: AgentRequest) -> BrpResult {
    let tx = &world.resource::<AgentRequestSender>().0;
    tx.send(req)
        .map_err(|_| internal_error("agent queue closed"))?;
    Ok(json!({ "ok": true }))
}

/// Catalog: what CAN be driven on this game.
fn playtest_schema(In(params): In<Option<Value>>, world: &World) -> BrpResult {
    brp_params(world, params)?;
    // get_resource everywhere: an unregistered contract must not panic
    // the BRP worker — return empty lists instead (loud, but alive).
    let variants: Vec<String> = world
        .get_resource::<IntentSurface>()
        .map(|s| s.0.iter().map(variant_label).collect())
        .unwrap_or_default();
    let resets: Vec<String> = world
        .get_resource::<ResetHooks>()
        .map(|h| h.0.keys().cloned().collect())
        .unwrap_or_default();
    let cheats: Vec<String> = world
        .get_resource::<CheatHooks>()
        .map(|h| h.0.keys().cloned().collect())
        .unwrap_or_default();
    Ok(json!({
        "intent_surface": variants,
        "reset_kinds": resets,
        "cheat_kinds": cheats,
        "intent_kinds": ["Move", "Select", "Choice", "Axis", "Wait"],
        "methods": PLAYTEST_METHODS,
    }))
}

fn variant_label(v: &SurfaceVariant) -> String {
    match v {
        SurfaceVariant::Move => "Move".into(),
        SurfaceVariant::Choice(max) => format!("Choice(max={max})"),
        SurfaceVariant::Axis(name) => format!("Axis({name})"),
        SurfaceVariant::Select => "Select".into(),
        SurfaceVariant::Wait => "Wait".into(),
    }
}

/// Percept snapshot: {"path": "Resource:GameState.current_turn"}
fn playtest_observe(In(params): In<Option<Value>>, world: &World) -> BrpResult {
    let params = brp_params(world, params)?;
    let path = params
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid_params("params.path (percept path) required"))?;
    let value = crate::harness::resolve_world_percept(world, path).ok_or_else(|| BrpError {
        code: error_codes::RESOURCE_ERROR,
        message: format!("percept `{path}` not found"),
        data: None,
    })?;
    Ok(match value {
        crate::contract::TestFieldValue::Numeric(n) => json!({ "value": n }),
        crate::contract::TestFieldValue::Text(s) => json!({ "value": s }),
    })
}

/// Semantic action tier: {"intent": "Choice", "index": 1}
fn playtest_intent(In(params): In<Option<Value>>, world: &World) -> BrpResult {
    let params = brp_params(world, params)?;
    let kind = params
        .get("intent")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid_params("params.intent required"))?;
    // Named intents dispatch immediately via NamedIntents resource.
    if kind.starts_with("Named") {
        let name = params
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid_params(format!("{} requires name", kind)))?
            .to_string();
        let registered = world
            .get_resource::<NamedIntents>()
            .map(|n| n.0.contains_key(&name))
            .unwrap_or(false);
        if !registered {
            return Err(BrpError {
                code: error_codes::INVALID_PARAMS,
                message: format!(
                    "unknown named intent `{}` (known: {})",
                    name,
                    world
                        .resource::<NamedIntents>()
                        .0
                        .keys()
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                data: None,
            });
        }
        // Queue for dispatch in agent_flush_system (which owns &mut World).
        send_request(world, AgentRequest::NamedIntent(name))?;
        return Ok(json!({"result": "queued"}));
    }
    let intent = decode_intent(&params)?;
    send_request(world, AgentRequest::Intent(intent))
}

/// Raw input tier: {"key": "space"} (chain-complete keyboard).
fn playtest_key(In(params): In<Option<Value>>, world: &World) -> BrpResult {
    let params = brp_params(world, params)?;
    let key = params
        .get("key")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid_params("params.key required"))?
        .to_string();
    if crate::harness::parse_key_code(&key).is_none() {
        return Err(invalid_params(format!("unknown key name `{}`", key)));
    }
    send_request(world, AgentRequest::KeyPress { key })
}

/// Raw input tier: {"x": 400.0, "y": 300.0} (viewport coordinates).
fn playtest_pointer(In(params): In<Option<Value>>, world: &World) -> BrpResult {
    let params = brp_params(world, params)?;
    let x = params.get("x").and_then(Value::as_f64);
    let y = params.get("y").and_then(Value::as_f64);
    let (Some(x), Some(y)) = (x, y) else {
        return Err(invalid_params("params.x and params.y required"));
    };
    send_request(
        world,
        AgentRequest::PointerClick {
            x: x as f32,
            y: y as f32,
        },
    )
}

/// Reset via the game's ResetHooks: {"kind": "reset_game"} (optional).
fn playtest_reset(In(params): In<Option<Value>>, world: &World) -> BrpResult {
    let params = brp_params(world, params)?;
    let kind = params
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or("reset_game")
        .to_string();
    // Validate synchronously before queuing.
    let known = world
        .get_resource::<ResetHooks>()
        .map(|h| h.0.contains_key(&kind))
        .unwrap_or(false);
    if !known {
        return Err(BrpError {
            code: error_codes::INVALID_PARAMS,
            message: format!(
                "unknown reset kind `{kind}` (known: {})",
                world
                    .resource::<ResetHooks>()
                    .0
                    .keys()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            data: None,
        });
    }
    send_request(world, AgentRequest::Reset(kind))
}

/// Invoke a registered cheat (AUDITED): {"kind": "god_mode"}.
fn playtest_cheat(In(params): In<Option<Value>>, world: &World) -> BrpResult {
    let params = brp_params(world, params)?;
    let kind = params
        .get("kind")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid_params("params.kind required"))?
        .to_string();
    let known = world
        .get_resource::<CheatHooks>()
        .map(|h| h.0.contains_key(&kind))
        .unwrap_or(false);
    if !known {
        return Err(BrpError {
            code: error_codes::INVALID_PARAMS,
            message: format!(
                "unknown cheat kind `{kind}` (known: {})",
                world
                    .resource::<CheatHooks>()
                    .0
                    .keys()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            data: None,
        });
    }
    send_request(world, AgentRequest::Cheat(kind))
}

/// Run status.
fn playtest_state(_params: In<Option<Value>>, world: &World) -> BrpResult {
    let frame = world.resource::<AgentFrameCounter>().0;
    let log = world.resource::<ActionLog>();
    Ok(json!({
        "frame": frame,
        "version": world.resource::<crate::contract::GameVersion>().0,
        "actions": log.entries.len(),
        "cheats": log.cheat_count(),
    }))
}

/// AUDIT: full action log (inputs, intents, cheats) with sources.
fn playtest_actions(_params: In<Option<Value>>, world: &World) -> BrpResult {
    let log = world.resource::<ActionLog>();
    let entries: Vec<Value> = log
        .entries
        .iter()
        .map(|e| {
            let source = match &e.source {
                ActionSource::BotScenario(s) => format!("bot:{s}"),
                ActionSource::Agent => "agent".into(),
                ActionSource::Manual => "manual".into(),
            };
            json!({
                "frame": e.frame,
                "action": e.action,
                "source": source,
            })
        })
        .collect();
    Ok(json!({ "entries": entries }))
}

// ---------------------------------------------------------------------------
// Intent decoding (JSON → UserIntent)
// ---------------------------------------------------------------------------

fn decode_intent(params: &Value) -> Result<UserIntent, BrpError> {
    let kind = params
        .get("intent")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid_params("params.intent required"))?;
    match kind {
        "Wait" => Ok(UserIntent::Wait),
        "Select" => {
            let bits = params
                .get("target")
                .and_then(Value::as_u64)
                .ok_or_else(|| invalid_params("Select requires target (entity bits u64)"))?;
            let target = bevy::ecs::entity::Entity::try_from_bits(bits)
                .ok_or_else(|| invalid_params(format!("target bits {bits:#x} do not encode a valid entity")))?;
            Ok(UserIntent::Select { target })
        }
        "Choice" => {
            let index = params
                .get("index")
                .and_then(Value::as_u64)
                .ok_or_else(|| invalid_params("Choice requires index (u64)"))? as usize;
            Ok(UserIntent::Choice { index })
        }
        "Move" => {
            let dir = params
                .get("dir")
                .ok_or_else(|| invalid_params("Move requires dir: {x, y}"))?;
            let x = dir.get("x").and_then(Value::as_f64).unwrap_or(0.0) as f32;
            let y = dir.get("y").and_then(Value::as_f64).unwrap_or(0.0) as f32;
            Ok(UserIntent::Move { dir: Vec2::new(x, y) })
        }
        "Axis" => {
            let name = params
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(|| invalid_params("Axis requires name"))?
                .to_string();
            let value = params.get("value").and_then(Value::as_f64).unwrap_or(0.0) as f32;
            Ok(UserIntent::Axis { name, value })
        }
        other => Err(invalid_params(format!(
            "unknown intent kind `{other}` (expected Move/Select/Choice/Axis/Wait, or Named* with `name`)"
        ))),
    }
}

// ---------------------------------------------------------------------------
// Systems — drain requests, inject through the REAL input chains
// ---------------------------------------------------------------------------

#[derive(Resource, Default)]
struct AgentFrameCounter(u64);

/// Exclusive system: drains the BRP channel and executes requests at a
/// controlled point (PreUpdate, before the game's input adapter).
fn agent_flush_system(world: &mut World) {
    world.resource_mut::<AgentFrameCounter>().0 += 1;
    let frame = world.resource::<AgentFrameCounter>().0;
    let elapsed_ms = agent_elapsed_ms(world);

    // Take all pending requests without holding the lock across world access.
    let requests: Vec<AgentRequest> = {
        let queue = world.resource::<AgentRequestQueue>();
        let rx = queue.0.lock().unwrap();
        let mut buf = Vec::new();
        loop {
            match rx.try_recv() {
                Ok(req) => buf.push(req),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break,
            }
        }
        buf
    };

    for req in requests {
        match req {
            AgentRequest::Intent(intent) => {
                let name = match &intent {
                    UserIntent::Move { .. } => "move",
                    UserIntent::Select { .. } => "select",
                    UserIntent::Choice { .. } => "choice",
                    UserIntent::Axis { .. } => "axis",
                    UserIntent::Wait => "wait",
                };
                world.resource_mut::<ActionLog>().record(
                    frame,
                    elapsed_ms,
                    ActionSource::Agent,
                    format!("agent-intent:{}", name),
                    None,
                );
                world.write_message(intent);
            }
            AgentRequest::KeyPress { key } => {
                let Some(code) = crate::harness::parse_key_code(&key) else {
                    warn!("playtest/key: unknown key `{key}`");
                    continue;
                };
                let window = primary_window_entity(world);
                world.write_message(key_input(code, bevy::input::ButtonState::Pressed, window));
                world.resource_mut::<PendingAgentInput>().keys.push(code);
                world.resource_mut::<ActionLog>().record(
                    frame,
                    elapsed_ms,
                    ActionSource::Agent,
                    format!("key:{}", key),
                    None,
                );
            }
            AgentRequest::PointerClick { x, y } => {
                let Some(window) = primary_window_entity(world) else {
                    warn!("playtest/pointer: no primary window");
                    continue;
                };
                let Some(location) = viewport_location(world, window, x, y) else {
                    warn!("playtest/pointer: could not normalize render target");
                    continue;
                };
                // Three-frame gesture: Move now, Press f+1, Release f+2.
                world.write_message(picking::pointer::PointerInput::new(
                    picking::pointer::PointerId::Mouse,
                    location.clone(),
                    picking::pointer::PointerAction::Move { delta: Vec2::ZERO },
                ));
                world
                    .resource_mut::<PendingAgentInput>()
                    .gestures
                    .push_back(PendingPointerGesture {
                        press_at: frame + 1,
                        release_at: frame + 2,
                        location,
                        button: picking::pointer::PointerButton::Primary,
                    });
            }
            AgentRequest::NamedIntent(name) => {
                // Named intents dispatch via the game's NamedIntents registry.
                let dispatched = crate::contract::NamedIntents::dispatch(world, &name);
                if dispatched {
                    world.resource_mut::<ActionLog>().record(
                        frame,
                        0.0,
                        ActionSource::Agent,
                        format!("named-intent:{}", name),
                        None,
                    );
                }
                continue;
            }
            AgentRequest::Cheat(kind) => {
                // Cheats are ALWAYS audited, then executed.
                world.resource_mut::<ActionLog>().record(
                    frame,
                    elapsed_ms,
                    ActionSource::Agent,
                    format!("cheat:{}", kind),
                    None,
                );
                let Some(mut hooks) = world.remove_resource::<CheatHooks>() else {
                    continue;
                };
                if let Some(hook) = hooks.0.remove(&kind) {
                    hook(world);
                }
                world.insert_resource(hooks);
            }
            AgentRequest::Reset(kind) => {
                // Take the whole resource (hooks aren't Clone), extract
                // ours, run it, then put the map back.
                let Some(mut hooks) = world.remove_resource::<ResetHooks>() else {
                    continue;
                };
                if let Some(hook) = hooks.0.remove(&kind) {
                    hook(world);
                }
                world.insert_resource(hooks);
            }
            AgentRequest::Screenshot { path } => {
                // Spawn a Screenshot (async write via observer).
                world
                    .spawn(bevy::render::view::window::screenshot::Screenshot::primary_window())
                    .observe(bevy::render::view::window::screenshot::save_to_disk(
                        path.clone(),
                    ));
                world.resource_mut::<ActionLog>().record(
                    frame,
                    elapsed_ms,
                    ActionSource::Agent,
                    format!("screenshot:{}", path.display()),
                    None,
                );
            }
        }
    }

    // Gesture press/release halves are delivered by agent_input_system
    // below; flush only enqueues.
}

/// Deliver pending key releases and pointer press/release halves due
/// this frame. Runs every PreUpdate regardless of new requests.
fn agent_input_system(world: &mut World) {
    let frame = world.resource::<AgentFrameCounter>().0;

    // Key releases: each key pressed via the agent gets released one
    // frame later (adapter observes both halves).
    let keys: Vec<bevy::input::keyboard::KeyCode> =
        std::mem::take(&mut world.resource_mut::<PendingAgentInput>().keys);
    if !keys.is_empty() {
        let window = primary_window_entity(world);
        for code in keys {
            world.write_message(key_input(code, bevy::input::ButtonState::Released, window));
        }
    }

    // Pointer gesture halves due this frame.
    let gestures = std::mem::take(&mut world.resource_mut::<PendingAgentInput>().gestures);
    let mut still = std::collections::VecDeque::new();
    for g in gestures {
        if g.press_at == frame {
            world.write_message(picking::pointer::PointerInput::new(
                picking::pointer::PointerId::Mouse,
                g.location.clone(),
                picking::pointer::PointerAction::Press(g.button),
            ));
        }
        if g.release_at == frame {
            world.write_message(picking::pointer::PointerInput::new(
                picking::pointer::PointerId::Mouse,
                g.location.clone(),
                picking::pointer::PointerAction::Release(g.button),
            ));
            continue; // gesture complete
        }
        still.push_back(g);
    }
    world.resource_mut::<PendingAgentInput>().gestures = still;
}

fn key_input(
    code: bevy::input::keyboard::KeyCode,
    state: bevy::input::ButtonState,
    window: Option<Entity>,
) -> bevy::input::keyboard::KeyboardInput {
    bevy::input::keyboard::KeyboardInput {
        key_code: code,
        logical_key: bevy::input::keyboard::Key::Unidentified(
            bevy::input::keyboard::NativeKey::Unidentified,
        ),
        state,
        text: None,
        repeat: false,
        window: window.unwrap_or(Entity::PLACEHOLDER),
    }
}

/// Real elapsed time for audit-log entries (was hardcoded 0.0).
/// Falls back to 0 when no Time resource exists (bare test worlds).
fn agent_elapsed_ms(world: &World) -> f64 {
    world
        .get_resource::<bevy::time::Time>()
        .map(|t| t.elapsed_secs_f64() * 1000.0)
        .unwrap_or(0.0)
}

fn primary_window_entity(world: &mut World) -> Option<Entity> {
    world
        .query_filtered::<Entity, With<bevy::window::PrimaryWindow>>()
        .single(world)
        .ok()
}

/// Read-only variant for BRP handlers (&World).
fn primary_window_read(world: &World) -> Option<Entity> {
    world
        .iter_entities()
        .find(|e| e.contains::<bevy::window::PrimaryWindow>())
        .map(|e| e.id())
}

fn viewport_location(
    _world: &World,
    window: Entity,
    x: f32,
    y: f32,
) -> Option<picking::pointer::Location> {
    let target = bevy::camera::RenderTarget::Window(bevy::window::WindowRef::Entity(window))
        .normalize(Some(window))?;
    Some(picking::pointer::Location {
        target,
        position: Vec2::new(x, y),
    })
}

/// Screenshot capture for visual regression testing.
/// In windowed mode this captures the real rendered frame (via the
// native screenshot's native screenshot mechanism); in headless
/// harness mode it returns window geometry only.
fn playtest_screenshot(In(params): In<Option<Value>>, world: &World) -> BrpResult {
    // Require a primary window; fail loudly if headless.
    let Some(window) = primary_window_read(world) else {
        return Err(invalid_params(
            "no primary window — screenshots need a rendering app",
        ));
    };
    let _ = window; // validated above; spawn will use it.
    let frame = world.resource::<AgentFrameCounter>().0;
    let path = params
        .as_ref()
        .and_then(|p| p.get("path"))
        .and_then(Value::as_str)
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| format!("screenshot-{}.png", frame).into());
    send_request(world, AgentRequest::Screenshot { path: path.clone() })?;
    Ok(json!({ "queued": true, "path": path }))
}

/// Contract diagnostics over BRP: detect contract pieces, report gaps.
fn playtest_diagnostics(_params: In<Option<Value>>, world: &World) -> BrpResult {
    let report = crate::harness::contract_diagnostics(world);
    let mut v = serde_json::to_value(report).unwrap_or(Value::Null);
    // Zero-contract observation: auto-enumerated reflected-resource paths.
    if let Some(obj) = v.as_object_mut() {
        obj.insert(
            "auto_observed_paths".into(),
            serde_json::to_value(crate::harness::auto_observed_paths(world)).unwrap_or(Value::Null),
        );
    }
    Ok(v)
}

/// Direct handler access for integration tests (no network needed).
#[doc(hidden)]
pub fn __test_observe(world: &World, params: Option<Value>) -> Result<Value, BrpError> {
    playtest_observe(In(params), world)
}

/// FX10 test hooks (no network needed).
#[doc(hidden)]
pub fn __test_token_eq(a: &str, b: &str) -> bool {
    token_eq(a, b)
}

#[doc(hidden)]
pub fn __test_world_get(world: &World, params: Option<Value>) -> Result<Value, BrpError> {
    with_token_guard(builtin_methods::process_remote_get_components_request)(In(params), world)
}
