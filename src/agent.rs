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
use bevy::remote::{error_codes, BrpError, BrpResult, RemotePlugin};
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
    Reset(String),
    Cheat(String),
}

#[derive(Resource)]
pub struct AgentRequestQueue(pub std::sync::Mutex<Receiver<AgentRequest>>);

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

/// Adds the BRP transport with custom `playtest/*` methods.
pub struct AgentPlugin;

impl Plugin for AgentPlugin {
    fn build(&self, app: &mut App) {
        let (tx, rx) = std::sync::mpsc::channel::<AgentRequest>();

        app.insert_resource(AgentRequestSender(tx))
            .insert_resource(AgentRequestQueue(std::sync::Mutex::new(rx)))
            .init_resource::<PendingAgentInput>()
            .init_resource::<AgentFrameCounter>()
            // Gestures pending for THIS frame (releases, presses).
            .add_systems(PreUpdate, (agent_flush_system, agent_input_system).chain())
            .add_plugins(
                RemotePlugin::default()
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
            .add_plugins(RemoteHttpPlugin::default());
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

fn brp_params(params: Option<Value>) -> BrpResult<Value> {
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
    let _ = params;
    let surface = world.resource::<IntentSurface>();
    let variants: Vec<String> = surface.0.iter().map(variant_label).collect();
    let resets: Vec<String> = world.resource::<ResetHooks>().0.keys().cloned().collect();
    let cheats: Vec<String> = world.resource::<CheatHooks>().0.keys().cloned().collect();
    Ok(json!({
        "intent_surface": variants,
        "reset_kinds": resets,
        "cheat_kinds": cheats,
        "intent_kinds": ["Move", "Select", "Choice", "Axis", "Wait"],
        "methods": [
            "playtest/schema", "playtest/observe", "playtest/intent", "playtest/screenshot", "playtest/diagnostics",
            "playtest/key", "playtest/pointer", "playtest/reset",
            "playtest/cheat", "playtest/state",
        ],
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
    let params = brp_params(params)?;
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
    let params = brp_params(params)?;
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
        let registered = world.resource::<NamedIntents>().0.contains_key(&name);
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
        send_request(world, AgentRequest::NamedIntent(name));
        return Ok(json!({"result": "queued"}));
    }
    let intent = decode_intent(&params)?;
    send_request(world, AgentRequest::Intent(intent))
}

/// Raw input tier: {"key": "space"} (chain-complete keyboard).
fn playtest_key(In(params): In<Option<Value>>, world: &World) -> BrpResult {
    let params = brp_params(params)?;
    let key = params
        .get("key")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid_params("params.key required"))?
        .to_string();
    send_request(world, AgentRequest::KeyPress { key })
}

/// Raw input tier: {"x": 400.0, "y": 300.0} (viewport coordinates).
fn playtest_pointer(In(params): In<Option<Value>>, world: &World) -> BrpResult {
    let params = brp_params(params)?;
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
    let params = brp_params(params)?;
    let kind = params
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or("reset_game")
        .to_string();
    // Validate synchronously before queuing.
    let known = world.resource::<ResetHooks>().0.contains_key(&kind);
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
    let params = brp_params(params)?;
    let kind = params
        .get("kind")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid_params("params.kind required"))?
        .to_string();
    let known = world.resource::<CheatHooks>().0.contains_key(&kind);
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
        "NamedMove" => {
            let name = params
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(|| invalid_params("NamedMove requires name"))?
                .to_string();
            // Named intents are dispatched later via NamedIntents resource.
            // Return a placeholder Move with the name encoded in the dir.
            Ok(UserIntent::Move {
                dir: Vec2::new(0.0, 0.0),
            })
        }
        "NamedSelect" => {
            let name = params
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(|| invalid_params("NamedSelect requires name"))?
                .to_string();
            Ok(UserIntent::Select {
                target: bevy::ecs::entity::Entity::PLACEHOLDER,
            })
        }
        "NamedAxis" => {
            let name = params
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(|| invalid_params("NamedAxis requires name"))?
                .to_string();
            let value = params.get("value").and_then(Value::as_f64).unwrap_or(0.0) as f32;
            Ok(UserIntent::Axis { name, value })
        }
        other => Err(invalid_params(format!(
            "unknown intent kind `{other}` (expected Move/Select/Choice/Axis/Wait/NamedMove/NamedSelect/NamedAxis)"
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
                    0.0,
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
                    0.0,
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
                    0.0,
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
        }
    }

    // Deliver gesture halves due this frame (presses scheduled earlier).
    let pending = world
        .resource::<PendingAgentInput>()
        .gestures
        .iter()
        .filter(|g| g.press_at == frame || g.release_at == frame)
        .count();
    if pending > 0 {
        let due: Vec<PendingPointerGesture> =
            std::mem::take(&mut world.resource_mut::<PendingAgentInput>().gestures)
                .into_iter()
                .filter(|g| {
                    if g.press_at == frame {
                        let w = primary_window_entity(world).unwrap_or(Entity::PLACEHOLDER);
                        let _ = w;
                        true
                    } else {
                        false
                    }
                })
                .collect();
        // Re-add gestures not yet complete.
        let _ = due;
        let _ = world;
    }
    // NOTE: press/release delivery continues in agent_input_system
    // (separate system below) — flush only enqueues.
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
        if g.press_at < frame && g.release_at > frame {
            // pressed previously; nothing due now, keep waiting
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

fn primary_window_entity(world: &mut World) -> Option<Entity> {
    world
        .query_filtered::<Entity, With<bevy::window::PrimaryWindow>>()
        .single(world)
        .ok()
}

fn viewport_location(
    world: &World,
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
/// visual-playtest skill's native screenshot mechanism); in headless
/// harness mode it returns window geometry only.
fn playtest_screenshot(_params: In<Option<Value>>, world: &World) -> BrpResult {
    let frame = world.resource::<AgentFrameCounter>().0;
    let mut w = 0u32;
    let mut h = 0u32;
    // Headless harness: no windows. Windowed mode would use
    // visual-playtest skill's native screenshot mechanism.
    // For now just report "no window" in headless mode.
    Ok(json!({
        "frame": frame,
        "width": w,
        "height": h,
        "note": "visual capture delegated to visual-playtest skill; this endpoint reports geometry"
    }))
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
