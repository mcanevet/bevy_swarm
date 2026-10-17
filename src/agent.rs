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
    /// 716.31: key RELEASES pending for a future frame. A key pressed
    /// via the agent must stay pressed through the END of the frame it
    /// was pressed in — releasing in the same frame means ButtonInput
    /// ends the frame NOT-PRESSED (both messages land in the same
    /// frame's input buffer).
    pending_key_releases: Vec<(bevy::input::keyboard::KeyCode, u64)>,
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
    /// `BEVY_PLAYTEST_AGENT_TOKEN` env), EVERY remotely reachable method
    /// requires `params.token` to match; mismatches return
    /// INVALID_REQUEST. `playtest/*` guards inside `brp_params`;
    /// read-only `world.*` and discovery methods are wrapped with the
    /// same check; mutating `world.*` builtins are replaced with
    /// disabling stubs; the render sub-app's method table is emptied.
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
            token: std::env::var("BEVY_PLAYTEST_AGENT_TOKEN")
                .ok()
                .filter(|t| !t.is_empty()),
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
            release_guard_decision(self.effective_deny_in_release())?;
            // Not denying: warn via eprintln (the LogPlugin may not be
            // up yet during Plugin::build, so bevy::log::warn! would
            // be lost).
            eprintln!(
                "WARNING: bevy_playtest agent feature is active in a RELEASE build —                  BRP exposes arbitrary world reads and mutation."
            );
        }
        if !self.addr.is_loopback() && !self.allow_remote {
            return Err(format!(
                "refusing to bind agent BRP endpoint to non-loopback {}                  without allow_remote: true",
                self.addr
            ));
        }
        if self.allow_remote && self.token.as_deref().unwrap_or("").is_empty() {
            return Err(
                "allow_remote: true requires a non-empty token (config or BEVY_PLAYTEST_AGENT_TOKEN)"
                    .to_string(),
            );
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
/// C2 default-deny: generate a cryptographically random token from OS
/// entropy. Called at startup when no token is configured.
fn generate_random_token() -> Result<String, getrandom04::Error> {
    let mut buf = [0u8; 32];
    getrandom04::fill(&mut buf)?;
    const HEX: &[u8; 16] = b"0123456789abcdef";
    Ok(buf
        .iter()
        .flat_map(|b| [HEX[(b >> 4) as usize], HEX[(b & 0xf) as usize]])
        .map(|c| c as char)
        .collect())
}

pub(crate) fn token_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    // Length contributes to the accumulator instead of an early
    // return, so total comparison time depends only on the max length,
    // not on WHERE the first difference occurs.
    let (longer, shorter) = if a.len() >= b.len() { (a, b) } else { (b, a) };
    let mut diff: u8 = 0;
    for (i, x) in longer.iter().enumerate() {
        let y = shorter.get(i).copied().unwrap_or(0);
        diff |= x ^ y;
    }
    diff == 0 && a.len() == b.len()
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
    // C2 default-deny: a missing configured token REJECTS. The plugin
    // generates one at startup, so None here only occurs on a
    // hand-built App — refuse rather than open the method table.
    let Some(expected) = world
        .get_resource::<AgentAuthToken>()
        .and_then(|t| t.0.clone())
    else {
        return false;
    };
    // H3: reject over-long token params before scanning — avoids the
    // token_eq CPU cost on multi-MB payloads.
    let Some(provided) = params.and_then(|p| p.get("token")).and_then(Value::as_str) else {
        return false;
    };
    if provided.len() > 1024 {
        return false;
    }
    token_eq(provided, &expected)
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
    let plugin = RemotePlugin::default()
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
        // FX10: discovery/introspection methods — token-guarded so a
        // caller cannot even enumerate methods or schedules unauthenticated.
        .with_method_main(
            builtin_methods::RPC_DISCOVER_METHOD,
            with_token_guard_mut(builtin_methods::process_remote_list_methods_request),
        )
        .with_method_main(
            builtin_methods::BRP_REGISTRY_SCHEMA_METHOD,
            with_token_guard(builtin_methods::export_registry_types),
        )
        .with_method_main(
            builtin_methods::BRP_SCHEDULE_LIST,
            with_token_guard(builtin_methods::schedule_list),
        )
        .with_method_main(
            builtin_methods::BRP_SCHEDULE_GRAPH,
            with_token_guard_mut(builtin_methods::schedule_graph),
        );
    // FX10 render-world bypass: bevy_remote's bevy_render feature
    // (forced on via bevy's bevy_remote+bevy_render feature
    // unification) serves the RENDER sub-app on a second port with
    // the full default method table, unguarded. The harness has no
    // legitimate remote use for the render world — replace every
    // default render method with a rejection (later registrations
    // replace earlier same-name entries).
    let plugin = ALL_DEFAULT_METHODS.iter().fold(plugin, |p, name| {
        p.with_method_render(*name, world_method_disabled)
    });
    plugin
}

/// FX10: every built-in method name RemotePlugin::default() registers
/// (main AND render apps). Used to override the entire render-app
/// method table with rejections — the render sub-app shares nothing
/// with playtesting and must not be remotely callable.
const ALL_DEFAULT_METHODS: &[&str] = &[
    builtin_methods::BRP_GET_COMPONENTS_METHOD,
    builtin_methods::BRP_QUERY_METHOD,
    builtin_methods::BRP_SPAWN_ENTITY_METHOD,
    builtin_methods::BRP_INSERT_COMPONENTS_METHOD,
    builtin_methods::BRP_REMOVE_COMPONENTS_METHOD,
    builtin_methods::BRP_DESPAWN_COMPONENTS_METHOD,
    builtin_methods::BRP_REPARENT_ENTITIES_METHOD,
    builtin_methods::BRP_LIST_COMPONENTS_METHOD,
    builtin_methods::BRP_MUTATE_COMPONENTS_METHOD,
    builtin_methods::RPC_DISCOVER_METHOD,
    builtin_methods::BRP_GET_COMPONENTS_AND_WATCH_METHOD,
    builtin_methods::BRP_LIST_COMPONENTS_AND_WATCH_METHOD,
    builtin_methods::BRP_GET_RESOURCE_METHOD,
    builtin_methods::BRP_INSERT_RESOURCE_METHOD,
    builtin_methods::BRP_REMOVE_RESOURCE_METHOD,
    builtin_methods::BRP_MUTATE_RESOURCE_METHOD,
    builtin_methods::BRP_LIST_RESOURCES_METHOD,
    builtin_methods::BRP_TRIGGER_EVENT_METHOD,
    builtin_methods::BRP_WRITE_MESSAGE_METHOD,
    builtin_methods::BRP_OBSERVE_METHOD,
    builtin_methods::BRP_REGISTRY_SCHEMA_METHOD,
    builtin_methods::BRP_SCHEDULE_LIST,
    builtin_methods::BRP_SCHEDULE_GRAPH,
];

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
        // C2 default-deny: there is ALWAYS a token. When none is
        // configured (env/config), generate a random one at startup and
        // log it — loopback is not an authentication boundary, so the
        // previous None = "no auth" behavior was a full bypass.
        // le3.5 (RF1/qo7.10): an explicitly-configured empty token is
        // treated as unset — a Some("") must not authenticate
        // {token:""}.
        let token = self
            .config
            .token
            .clone()
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| {
                let generated =
                    generate_random_token().expect("agent token generation from OS randomness");
                eprintln!(
                    "bevy_playtest agent: no BEVY_PLAYTEST_AGENT_TOKEN configured; \
                 generated a random session token. Set BEVY_PLAYTEST_AGENT_TOKEN \
                 to use the agent programmatically. Token: {generated}"
                );
                generated
            });
        app.insert_resource(AgentAuthToken(Some(token)));

        let (tx, rx) = std::sync::mpsc::channel::<AgentRequest>();

        app.insert_resource(AgentRequestSender(tx))
            .insert_resource(AgentRequestQueue(std::sync::Mutex::new(rx)))
            .init_resource::<PendingAgentInput>()
            .init_resource::<AgentFrameCounter>()
            // le3.5 (RF1): agent_flush_system records into ActionLog;
            // a missing one panicked the app. Init here so the plugin
            // is standalone-safe (TestConventionsPlugin inserts its
            // own instance — identical Default value).
            .init_resource::<crate::contract::ActionLog>()
            .init_resource::<crate::contract::GameVersion>()
            // Gestures pending for THIS frame (releases, presses).
            .add_systems(
                PreUpdate,
                (
                    agent_method_allowlist_sweep,
                    agent_flush_system,
                    agent_input_system,
                )
                    .chain(),
            )
            // FX10: after ALL plugins build, sweep the method tables and
            // remove anything outside the allow-list. Covers methods
            // registered LATER than guarded_remote_plugin() (other
            // plugins' RemotePlugin extensions).
            .add_systems(Startup, agent_method_allowlist_sweep)
            // le3.24 (RF28): bevy-remote SERVES in RemoteLast — a
            // method inserted by an Update system was served (unguarded)
            // before the next PreUpdate sweep ran. Sweeping again in
            // RemoteLast (before serving) closes the one-frame window
            // permanently, even for methods re-inserted every frame.
            // le3.37 (RF47): `.before(ProcessRequests)` — bevy-remote
            // 0.20 runs process_remote_requests in the SAME schedule
            // set; unordered systems left the serve window open by
            // scheduling luck (flaky across machines). Explicit order
            // closes it deterministically.
            .add_systems(
                bevy::remote::RemoteLast,
                agent_method_allowlist_sweep.before(bevy::remote::RemoteSystems::ProcessRequests),
            )
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

        // le3.37 (RF47): snapshot OUR handler ids immediately after the
        // guarded plugin builds. Later plugins (or runtime systems) can
        // REPLACE an allow-listed method's handler by re-registering
        // the same NAME — the old sweep kept it (name matched) with the
        // game's UNGUARDED handler. The sweep now restores OUR handler
        // when the current id differs from this snapshot.
        let snapshot = snapshot_guarded_handlers(app.world_mut());
        app.insert_resource(snapshot);
    }

    fn finish(&self, app: &mut App) {
        // FX10: run AFTER all plugins built (including any that
        // extended the remote method tables). Main table: drop
        // everything outside the allow-list. Render table: emptied
        // entirely — no remote access to the render world at all.
        agent_method_allowlist_sweep(app.world_mut());
        // NOTE: no cfg gate — the agent feature always enables
        // bevy/bevy_render (needed for playtest/screenshot), and
        // get_sub_app_mut is a no-op when no render sub-app exists.
        if let Some(render_app) = app.get_sub_app_mut(bevy::render::RenderApp) {
            use bevy::remote::RemoteMethods;
            if render_app
                .world_mut()
                .remove_resource::<RemoteMethods>()
                .is_some()
            {
                render_app.world_mut().insert_resource(RemoteMethods::new());
            }
            // le3.37 (RF47): the render RemoteLast table was only
            // cleared ONCE at finish — a runtime system re-registering
            // a render method went unswept and was served. Sweep every
            // RemoteLast tick (ordered before ProcessRequests), like
            // the main-world sweep.
            render_app.add_systems(
                bevy::remote::RemoteLast,
                render_method_allowlist_sweep.before(bevy::remote::RemoteSystems::ProcessRequests),
            );
        }
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
    // C2 default-deny: a missing configured token REJECTS. The plugin
    // generates one at startup, so None here only occurs on a
    // hand-built App — refuse rather than open the method table.
    let Some(expected) = world
        .get_resource::<AgentAuthToken>()
        .and_then(|t| t.0.clone())
    else {
        return Err(BrpError {
            code: error_codes::INVALID_REQUEST,
            message: "invalid or missing token".to_string(),
            data: None,
        });
    };
    // H3: reject over-long token params before scanning.
    let Some(provided) = params
        .as_ref()
        .and_then(|p| p.get("token"))
        .and_then(Value::as_str)
    else {
        return Err(BrpError {
            code: error_codes::INVALID_REQUEST,
            message: "invalid or missing token".to_string(),
            data: None,
        });
    };
    // le3.5 (RF1/qo7.10): an empty provided token is never valid —
    // guards the Some("") configured-token edge from config paths
    // that predate the build-time filter.
    if provided.is_empty() {
        return Err(BrpError {
            code: error_codes::INVALID_REQUEST,
            message: "invalid or missing token".to_string(),
            data: None,
        });
    }
    if provided.len() > 1024 {
        return Err(BrpError {
            code: error_codes::INVALID_REQUEST,
            message: "token too long (max 1KiB)".to_string(),
            data: None,
        });
    }
    if !token_eq(provided, &expected) {
        return Err(BrpError {
            code: error_codes::INVALID_REQUEST,
            message: "invalid or missing token".to_string(),
            data: None,
        });
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
                        .get_resource::<NamedIntents>()
                        .map(|n| n.0.keys().cloned().collect::<Vec<_>>())
                        .unwrap_or_default()
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
                    .get_resource::<ResetHooks>()
                    .map(|h| h.0.keys().cloned().collect::<Vec<_>>().join(", "))
                    .unwrap_or_default()
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
                    .get_resource::<CheatHooks>()
                    .map(|h| h.0.keys().cloned().collect::<Vec<_>>().join(", "))
                    .unwrap_or_default()
            ),
            data: None,
        });
    }
    send_request(world, AgentRequest::Cheat(kind))
}

/// Run status.
fn playtest_state(_params: In<Option<Value>>, world: &World) -> BrpResult {
    // le3.5 (RF1): token-guard EVERY playtest/* method.
    brp_params(world, _params.0.clone())?;
    // le3.5 (RF1): ActionLog/GameVersion are TestConventionsPlugin
    // resources — absent on apps without it. Missing = 0/default,
    // NOT a panic (an unauthenticated probe must not crash the app).
    let frame = world
        .get_resource::<AgentFrameCounter>()
        .map(|f| f.0)
        .unwrap_or(0);
    let log = world.get_resource::<ActionLog>();
    Ok(json!({
        "frame": frame,
        "version": world
            .get_resource::<crate::contract::GameVersion>()
            .map(|v| v.0.clone())
            .unwrap_or_default(),
        "actions": log.map(|l| l.entries.len()).unwrap_or(0),
        "cheats": log.map(|l| l.cheat_count()).unwrap_or(0),
    }))
}

/// AUDIT: full action log (inputs, intents, cheats) with sources.
fn playtest_actions(_params: In<Option<Value>>, world: &World) -> BrpResult {
    // le3.5 (RF1): token-guard.
    brp_params(world, _params.0.clone())?;
    // Absent ActionLog (no TestConventionsPlugin) = empty log, not a
    // panic.
    let Some(log) = world.get_resource::<ActionLog>() else {
        return Ok(json!({ "entries": [] }));
    };
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
        let rx = queue.0.lock().unwrap_or_else(|e| e.into_inner());
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
                // 716.31: release ONE FRAME LATER — pressing and
                // releasing in the same frame leaves ButtonInput
                // NOT-PRESSED at frame end.
                let release_at = world.resource::<AgentFrameCounter>().0 + 1;
                world
                    .resource_mut::<PendingAgentInput>()
                    .pending_key_releases
                    .push((code, release_at));
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
                    // 716.34: take-run-put — a BRP cheat deletes its
                    // own hook; re-insert so repeat cheats work.
                    hooks.0.insert(kind.clone(), hook);
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
                    // 716.34: take-run-put — after one reset of kind K,
                    // K was permanently gone. Re-insert so repeated
                    // resets work.
                    hooks.0.insert(kind.clone(), hook);
                }
                world.insert_resource(hooks);
            }
            AgentRequest::Screenshot { path } => {
                // Ensure the sandbox directory exists before the async
                // write (save_to_disk does not create parents).
                if let Some(parent) = path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
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
    // frame LATER — 716.31: fire only when frame >= release_at so a
    // key pressed at frame F stays pressed through the end of frame F.
    let pending: Vec<(bevy::input::keyboard::KeyCode, u64)> = std::mem::take(
        &mut world
            .resource_mut::<PendingAgentInput>()
            .pending_key_releases,
    );
    let window = primary_window_entity(world);
    let mut still_pending = Vec::new();
    for (code, release_at) in pending {
        if frame >= release_at {
            world.write_message(key_input(code, bevy::input::ButtonState::Released, window));
        } else {
            still_pending.push((code, release_at));
        }
    }
    world
        .resource_mut::<PendingAgentInput>()
        .pending_key_releases = still_pending;

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

/// le3.5 (RF1): pure screenshot-path sanitizer. Hardened against
/// Windows escapes: a path is accepted ONLY if every component is
/// `Component::Normal` (no `..`, no absolute/root/prefix components
/// — `\Users\x` has a RootDir and `C:x` has a Prefix, both
/// rejected), contains no `:` or `\` characters, and ends in `.png`.
/// Result is confined to `screenshots/`.
fn sanitize_screenshot_path(user: &std::path::Path) -> Result<std::path::PathBuf, BrpError> {
    let as_str = user.to_string_lossy();
    if !as_str.ends_with(".png") {
        return Err(invalid_params("screenshot path must end with '.png'"));
    }
    for comp in user.components() {
        match comp {
            std::path::Component::Normal(part) => {
                // Defensive on all platforms: reject Windows drive
                // prefixes ('C:') and backslash separators smuggled
                // into a Normal component (possible when the raw
                // string was 'C:a.png' — parsed as one Normal
                // component on Unix).
                let s = part.to_string_lossy();
                if s.contains(':') || s.contains('\\') {
                    return Err(invalid_params(
                        "screenshot path contains forbidden characters",
                    ));
                }
            }
            _ => {
                return Err(invalid_params(
                    "screenshot path must be a plain relative path (no '..', absolute paths, or drive prefixes)",
                ));
            }
        }
    }
    Ok(std::path::PathBuf::from("screenshots").join(user))
}

/// Screenshot capture for visual regression testing.
/// In windowed mode this captures the real rendered frame (via the
// native screenshot's native screenshot mechanism); in headless
/// harness mode it returns window geometry only.
fn playtest_screenshot(In(params): In<Option<Value>>, world: &World) -> BrpResult {
    // le3.5 (RF1): token-guard — screenshot is an unauthenticated
    // disk write otherwise.
    let params = brp_params(world, params)?;
    let Some(window) = primary_window_read(world) else {
        return Err(invalid_params(
            "no primary window — screenshots need a rendering app",
        ));
    };
    let _ = window;
    let frame = world.resource::<AgentFrameCounter>().0;

    // C1 sandbox: sanitize path — reject .., absolute paths, non-.png suffixes.
    let user_path: Option<std::path::PathBuf> = params
        .get("path")
        .and_then(Value::as_str)
        .map(std::path::PathBuf::from);

    let path = match user_path {
        Some(p) => sanitize_screenshot_path(&p)?,
        None => std::path::PathBuf::from(format!("screenshots/screenshot-{frame}.png")),
    };

    send_request(world, AgentRequest::Screenshot { path: path.clone() })?;
    Ok(json!({ "queued": true, "path": path }))
}

/// Contract diagnostics over BRP: detect contract pieces, report gaps.
fn playtest_diagnostics(_params: In<Option<Value>>, world: &World) -> BrpResult {
    // le3.5 (RF1): token-guard — diagnostics leak reflected resource
    // paths.
    brp_params(world, _params.0.clone())?;
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

/// Release-path decision, testable regardless of build profile: when
/// `deny` is the EFFECTIVE flag, building with the agent feature in
/// release must error out.
#[doc(hidden)]
pub fn release_guard_decision(deny: bool) -> Result<(), String> {
    let msg = "bevy_playtest agent feature is active in a RELEASE build —                        BRP exposes arbitrary world reads and mutation.                        Never ship builds with the agent enabled.";
    if deny {
        Err(msg.to_string())
    } else {
        Ok(())
    }
}

/// FX10: enforce the method allow-list at Startup (after all plugins
/// finished building their RemotePlugin method tables). Anything not in
/// PLAYTEST_METHODS, the guarded world.* read-only trio, or the
/// token-guarded discovery set is removed from the MAIN table; the
/// render table is emptied entirely (render-world BRP is disabled).
const ALLOWED: &[&str] = &[
    // playtest/* (token-guarded via brp_params)
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
    // token-guarded read-only world introspection
    "world.get_components",
    "world.query",
    "world.list_components",
    "world.get_resources",
    // token-guarded discovery
    "rpc.discover",
    "registry.schema",
    "schedule.list",
    "schedule.graph",
];

const MUTATION_STUBS: &[&str] = &[
    "world.spawn_entity",
    "world.insert_components",
    "world.remove_components",
    "world.despawn_entity",
    "world.reparent_entities",
    "world.mutate_components",
    "world.insert_resources",
    "world.remove_resources",
    "world.mutate_resources",
    "world.list_resources",
    "world.trigger_event",
    "world.write_message",
    "world.observe+watch",
    "world.get_components+watch",
    "world.list_components+watch",
];

/// le3.37 (RF47): snapshot our guarded handlers immediately after
/// plugin build. The sweep restores these handlers when a later
/// plugin/runtime system replaces an allow-listed method's handler.
fn snapshot_guarded_handlers(world: &mut World) -> GuardedMethodSnapshot {
    use bevy::remote::RemoteMethods;
    let mut map = std::collections::HashMap::new();
    if let Some(methods) = world.get_resource::<RemoteMethods>() {
        for name in methods.methods() {
            if let Some(id) = methods.get(&name) {
                // Allow-list + stubs are OUR handlers.
                if ALLOWED.contains(&name.as_str()) || MUTATION_STUBS.contains(&name.as_str()) {
                    map.insert(name, *id);
                }
            }
        }
    }
    GuardedMethodSnapshot(map)
}

#[derive(Resource, Default)]
struct GuardedMethodSnapshot(std::collections::HashMap<String, bevy::remote::RemoteMethodSystemId>);

/// le3.37 (RF47): render-world sweep — re-empty the render method
/// table every RemoteLast tick (ordered before ProcessRequests). The
/// finish()-time clear missed runtime re-registrations.
fn render_method_allowlist_sweep(world: &mut World) {
    use bevy::remote::RemoteMethods;
    if world
        .remove_resource::<RemoteMethods>()
        .replace(RemoteMethods::new())
        .is_some()
    {
        // Table had entries — cleared; re-insert the empty table.
        world.insert_resource(RemoteMethods::new());
    }
}

fn agent_method_allowlist_sweep(world: &mut World) {
    use bevy::remote::RemoteMethods;

    // Disabling stubs stay REGISTERED (they return METHOD_NOT_FOUND +
    // a clear message); sweeping them out entirely would leak less
    // info but also hides the error contract. The stubs are the
    // enforcement; unknown methods (added by other plugins) are dropped.

    if let Some(methods) = world.remove_resource::<RemoteMethods>() {
        // le3.37 (RF47): restore OUR handler for allow-listed names when
        // the current one is NOT the snapshotted guarded handler (a
        // game re-registering "world.query" or "playtest/observe" used
        // to keep its unguarded handler — the filter was name-only).
        let snapshot = world
            .remove_resource::<GuardedMethodSnapshot>()
            .unwrap_or_default();
        let mut kept = RemoteMethods::new();
        for name in methods.methods() {
            let is_allowed =
                ALLOWED.contains(&name.as_str()) || MUTATION_STUBS.contains(&name.as_str());
            if !is_allowed {
                continue;
            }
            // Fail closed: keep ONLY the snapshotted guarded handler.
            // If the current handler is someone else's re-registration
            // of an allow-listed name, OURS is restored (or the entry is
            // dropped when we never had one).
            if let Some(ours) = snapshot.0.get(&name) {
                kept.insert(name.clone(), *ours);
            }
        }
        world.insert_resource(snapshot);
        world.insert_resource(kept);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screenshot_path_sandbox_rejects_traversal() {
        // le3.5 (RF1): the OLD test called playtest_screenshot on
        // World::new() — the window error fired BEFORE path
        // validation, so every assertion was vacuous. Now tests the
        // pure sanitizer directly (mutation-proof: disabling any
        // check in sanitize_screenshot_path fails this test).
        let reject = |path: &str| sanitize_screenshot_path(std::path::Path::new(path)).is_err();
        // traversal / absolute / prefixes
        assert!(reject("../etc/passwd.png"));
        assert!(reject("/absolute/path.png"));
        assert!(reject("foo/../bar.png"));
        assert!(reject("..\\..\\evil.png"));
        // Windows root-relative and drive prefixes
        assert!(reject("\\Users\\victim\\x.png"));
        assert!(reject("C:evil.png"));
        assert!(reject("C:\\evil.png"));
        // wrong suffix
        assert!(reject("foo.txt"));
        assert!(reject("screenshot.jpg"));
        assert!(reject("foo/bar.baz"));
        // empty / dot-only components
        assert!(reject("."));
        assert!(reject(""));
    }

    #[test]
    fn screenshot_path_sandbox_accepts_valid_relative_png() {
        // le3.5 (RF1): accept check on the pure sanitizer — asserts
        // the RESULT (confined under screenshots/), not just Ok.
        let accept = |path: &str| {
            sanitize_screenshot_path(std::path::Path::new(path))
                .ok()
                .filter(|p| p.starts_with("screenshots"))
        };
        assert!(accept("screenshot-1.png").is_some());
        assert!(accept("a/b/c.png").is_some());
        assert!(accept("deep/nested/dir/shot.png").is_some());
        // result confinement
        assert_eq!(
            sanitize_screenshot_path(std::path::Path::new("x.png")).unwrap(),
            std::path::PathBuf::from("screenshots/x.png")
        );
    }
}

#[cfg(test)]
mod tests_716_34_agent {
    use super::*;
    use crate::contract::{ActionLog, ResetHooks};
    use std::collections::HashMap;

    #[test]
    fn brp_reset_twice_works() {
        // 716.34: after one reset of kind K, K was permanently gone.
        // Repeated resets must work — drive agent_flush_system twice.
        use crate::agent::AgentRequest;

        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.init_resource::<ActionLog>();
        app.insert_resource(crate::agent::AgentFrameCounter(0));
        app.init_resource::<crate::agent::PendingAgentInput>();

        let counter = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let c = counter.clone();
        let hook: Box<dyn Fn(&mut World) + Send + Sync> = Box::new(move |_: &mut World| {
            c.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        });
        app.insert_resource(ResetHooks(HashMap::from([("my_reset".to_string(), hook)])));

        // Wire the request channel.
        let (tx, rx) = std::sync::mpsc::channel::<AgentRequest>();
        tx.send(AgentRequest::Reset("my_reset".to_string()))
            .unwrap();
        tx.send(AgentRequest::Reset("my_reset".to_string()))
            .unwrap();
        app.insert_resource(crate::agent::AgentRequestQueue(std::sync::Mutex::new(rx)));
        app.insert_resource(crate::agent::AgentRequestSender(tx));

        // Flush once — processes both queued resets.
        crate::agent::agent_flush_system(app.world_mut());
        assert_eq!(
            counter.load(std::sync::atomic::Ordering::SeqCst),
            2,
            "two resets, hook survives first use"
        );
    }
}

#[cfg(test)]
mod tests_716_31 {
    use super::*;
    use bevy::input::keyboard::KeyCode;

    #[test]
    fn key_stays_pressed_through_end_of_frame() {
        // 716.31: a key pressed at frame F must remain PRESSED at the
        // end of frame F (not same-frame released), and be released at
        // frame F+1.
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_message::<bevy::input::keyboard::KeyboardInput>();
        app.insert_resource(AgentFrameCounter(0));
        app.init_resource::<PendingAgentInput>();

        // Simulate a KeyPress request at frame 0 (via the flush path).
        let frame = 0u64;
        let code = KeyCode::KeyA;
        let window = primary_window_entity(app.world_mut());
        app.world_mut()
            .write_message(key_input(code, bevy::input::ButtonState::Pressed, window));
        app.world_mut()
            .resource_mut::<PendingAgentInput>()
            .pending_key_releases
            .push((code, frame + 1));

        // Frame 0: agent_input_system must NOT release the key yet.
        agent_input_system(app.world_mut());
        let released_at_f = count_released(app.world_mut());
        assert_eq!(
            released_at_f, 0,
            "key must remain pressed through end of frame F"
        );

        // Frame 1: the release fires.
        app.world_mut().resource_mut::<AgentFrameCounter>().0 = 1;
        agent_input_system(app.world_mut());
        let released_at_f1 = count_released(app.world_mut());
        assert_eq!(released_at_f1, 1, "release must fire at frame F+1");
    }

    /// Count Released KeyboardInput messages via a one-off reader
    /// system (World has no direct message-read API).
    fn count_released(world: &mut World) -> usize {
        #[derive(bevy::prelude::Resource)]
        struct Count(pub usize);
        world.insert_resource(Count(0));
        let sid = world.register_system(
            |mut reader: bevy::ecs::message::MessageReader<
                bevy::input::keyboard::KeyboardInput,
            >,
             mut count: bevy::ecs::system::ResMut<Count>| {
                count.0 = reader
                    .read()
                    .filter(|e| e.state == bevy::input::ButtonState::Released)
                    .count();
            },
        );
        world.run_system(sid).unwrap();
        let n = world.resource::<Count>().0;
        world.remove_resource::<Count>().unwrap();
        n
    }
}

#[cfg(test)]
mod tests_b2e_1 {
    use super::*;

    #[test]
    fn poisoned_queue_mutex_recovers_next_frame() {
        // b2e.1: poison the AgentRequestQueue mutex (panic while
        // holding), then verify agent_flush_system does NOT panic on
        // the next frame — it recovers the guard via
        // unwrap_or_else(|e| e.into_inner()).
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.init_resource::<ActionLog>();
        app.insert_resource(crate::agent::AgentFrameCounter(0));
        app.init_resource::<PendingAgentInput>();

        let (tx, rx) = std::sync::mpsc::channel::<AgentRequest>();
        tx.send(AgentRequest::Reset("k".to_string())).ok();
        app.insert_resource(crate::agent::AgentRequestQueue(std::sync::Mutex::new(rx)));

        // Poison: panic while holding the lock.
        {
            let queue = app.world().resource::<crate::agent::AgentRequestQueue>();
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _g = queue.0.lock().unwrap();
                panic!("deliberate poison");
            }));
        }
        assert!(
            app.world()
                .resource::<crate::agent::AgentRequestQueue>()
                .0
                .is_poisoned(),
            "precondition: mutex must be poisoned"
        );

        // Next frame: must NOT panic (poison recovered gracefully).
        let flushed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            crate::agent::agent_flush_system(app.world_mut());
        }));
        assert!(
            flushed.is_ok(),
            "agent_flush_system must survive a poisoned queue mutex"
        );
    }
}
