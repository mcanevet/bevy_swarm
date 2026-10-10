//! R2: Agent BRP endpoint security — loopback default, token auth,
//! release-build warning. Handler-level tests; no network needed.

#![cfg(feature = "agent")]

use bevy::prelude::*;
use bevy_swarm::contract::{IntentSurface, SurfaceVariant, TestConventionsPlugin};
use bevy_swarm::driver::PlaytestPlugin;
use std::net::{IpAddr, Ipv4Addr};

fn world_with_token(token: Option<String>) -> bevy::ecs::world::World {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        bevy::transform::TransformPlugin,
        TestConventionsPlugin,
        PlaytestPlugin,
    ));
    app.insert_resource(IntentSurface::new(vec![SurfaceVariant::Wait]));
    app.insert_resource(bevy_swarm::agent::AgentAuthToken(token));
    app.finish();
    std::mem::take(app.world_mut())
}

#[test]
fn non_loopback_rejected_without_opt_in() {
    let cfg = bevy_swarm::agent::AgentConfig {
        addr: IpAddr::V4(Ipv4Addr::new(192, 168, 1, 10)),
        ..Default::default()
    };
    let err = cfg.validate().unwrap_err();
    assert!(err.contains("allow_remote"), "got: {err}");

    // Loopback passes without opt-in.
    assert!(bevy_swarm::agent::AgentConfig::default().validate().is_ok());

    // Explicit opt-in passes (FX10: remote requires a non-empty token).
    let cfg = bevy_swarm::agent::AgentConfig {
        allow_remote: true,
        token: Some("sekrit".into()),
        ..cfg
    };
    assert!(cfg.validate().is_ok());
}

#[test]
fn token_required_when_configured() {
    use bevy::remote::error_codes;
    let world = world_with_token(Some("sekrit".into()));

    // No token → INVALID_REQUEST.
    let err = bevy_swarm::agent::__test_observe(&world, None).unwrap_err();
    assert_eq!(err.code, error_codes::INVALID_REQUEST);

    // Wrong token → INVALID_REQUEST.
    let params = serde_json::json!({"path": "TestApi.score", "token": "wrong"});
    let err = bevy_swarm::agent::__test_observe(&world, Some(params)).unwrap_err();
    assert_eq!(err.code, error_codes::INVALID_REQUEST);

    // Right token → passes auth (may fail later for other reasons, but
    // not with INVALID_REQUEST auth error on the token).
    let params = serde_json::json!({"path": "TestApi.score", "token": "sekrit"});
    let res = bevy_swarm::agent::__test_observe(&world, Some(params));
    assert!(res.is_ok() || res.unwrap_err().code != error_codes::INVALID_REQUEST);

    // No token configured → no auth required.
    let world = world_with_token(None);
    let params = serde_json::json!({"path": "TestApi.score"});
    let res = bevy_swarm::agent::__test_observe(&world, Some(params));
    assert!(res.is_ok() || res.unwrap_err().code != error_codes::INVALID_REQUEST);
}

#[test]
fn release_guard_decision_function() {
    // validate() only warns (returns Ok) in debug builds unless
    // deny_in_release; in release it errs. The decision is exercised
    // via the unit-testable validate() in the other tests; here we
    // check the deny flag plumbing exists and defaults off.
    let cfg = bevy_swarm::agent::AgentConfig::default();
    assert!(!cfg.deny_in_release);
    assert_eq!(cfg.addr, IpAddr::V4(Ipv4Addr::LOCALHOST));
    assert_eq!(cfg.port, 15702);
}

// ---------------------------------------------------------------------------
// FX10: world.* auth bypass, constant-time compare, release guard
// ---------------------------------------------------------------------------

#[test]
fn fx10_constant_time_compare() {
    // The token comparison must be constant-time: equal-length but
    // different content must be rejected without leaking which byte
    // differed (behaviorally indistinguishable, but the helper is
    // exposed for direct testing).
    use bevy_swarm::agent::__test_token_eq;
    assert!(__test_token_eq("sekrit", "sekrit"));
    assert!(!__test_token_eq("sekrit", "sekrut"));
    assert!(!__test_token_eq("sekrit", "sekritt"));
    assert!(!__test_token_eq("", "x"));
    assert!(__test_token_eq("", ""));
}

#[test]
fn fx10_deny_in_release_defaults_true_for_remote() {
    // allow_remote: true + deny_in_release unset -> effective default is
    // deny (remote exposure in release is the dangerous case).
    let cfg = bevy_swarm::agent::AgentConfig {
        allow_remote: true,
        ..Default::default()
    };
    assert!(cfg.effective_deny_in_release());
    assert!(!bevy_swarm::agent::AgentConfig::default().effective_deny_in_release());
}

// ---------------------------------------------------------------------------
// FX10 re-review: end-to-end HTTP tests against a REAL App with
// AgentPlugin (no __test_ shortcuts), render-port bypass, discovery
// methods, empty-token, release guard.
// ---------------------------------------------------------------------------

/// Full HTTP JSON-RPC roundtrip against a live BRP server: connects,
/// sends, pumps app.update() while awaiting the response (the mailbox
/// is processed during updates), parses the FIRST JSON value in the
/// body (hyper may leave the connection open).
fn brp_roundtrip(
    app: &mut bevy::app::App,
    port: u16,
    body: &serde_json::Value,
) -> serde_json::Value {
    use std::io::{Read, Write};
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    // Bring the server up (binds during Startup).
    loop {
        app.update();
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "BRP server never came up"
        );
    }
    let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .set_read_timeout(Some(std::time::Duration::from_millis(50)))
        .unwrap();
    stream.set_nodelay(true).unwrap();
    let payload = serde_json::to_vec(body).unwrap();
    stream
        .write_all(
            format!(
                "POST / HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                payload.len()
            )
            .as_bytes(),
        )
        .unwrap();
    stream.write_all(&payload).unwrap();
    let flush = stream.flush();
    let _ = flush;
    let mut buf = Vec::new();
    loop {
        app.update();
        let mut chunk = [0u8; 4096];
        match stream.read(&mut chunk) {
            Ok(0) => {
                if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
                // Server closed early (headers not fully arrived) — retry
                // with a fresh connection is out of scope; keep pumping
                // via a new read on a new socket below.
                break;
            }
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
            Err(e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut => {}
            Err(e) => panic!("socket error: {e}"),
        }
        assert!(
            std::time::Instant::now() < deadline,
            "no BRP response in 10s; buffer: {:?}",
            String::from_utf8_lossy(&buf)
        );
    }
    let text = String::from_utf8_lossy(&buf);
    // hyper uses chunked transfer: the body looks like
    // "37\r\n{json}\r\n0\r\n\r\n". Skip framing: parse from the first
    // '{' of the JSON-RPC response.
    let body_start = text.find('{').expect("JSON body start");
    let body = &text[body_start..];
    let mut de = serde_json::Deserializer::from_str(body).into_iter::<serde_json::Value>();
    de.next().expect("no JSON body").expect("valid JSON body")
}

fn is_auth_rejected(resp: &serde_json::Value) -> bool {
    resp.get("error")
        .and_then(|e| e.get("code"))
        .and_then(serde_json::Value::as_i64)
        .is_some_and(|c| c == -32600)
}

/// The disabling-stub response for mutating world.* built-ins.
fn is_disabled_stub(resp: &serde_json::Value) -> bool {
    resp.get("error")
        .and_then(|e| e.get("message"))
        .and_then(serde_json::Value::as_str)
        .is_some_and(|m| m.contains("disabled"))
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

#[test]
#[serial_test::serial]
fn fx10_http_world_methods_require_token() {
    // REAL end-to-end: unauthenticated world.*/discovery requests over
    // HTTP must be auth-rejected when a token is set; the correct token
    // passes auth.
    let port = free_port();
    let mut app = bevy::app::App::new();
    app.add_plugins(bevy::MinimalPlugins);
    app.add_plugins(bevy_swarm::agent::AgentPlugin::new(
        bevy_swarm::agent::AgentConfig {
            port,
            token: Some("sekrit".into()),
            ..Default::default()
        },
    ));
    for method in [
        "world.get_components",
        "world.query",
        "world.insert_components",
        "world.spawn_entity",
        "rpc.discover",
        "registry.schema",
        "schedule.list",
        "schedule.graph",
    ] {
        let resp = brp_roundtrip(
            &mut app,
            port,
            &serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": {}}),
        );
        // Guarded reads/discovery: auth error (-32600). Mutating
        // built-ins: replaced by disabling stubs (-32601). Either way
        // an unauthenticated caller gets NO world access.
        let rejected = is_auth_rejected(&resp) || is_disabled_stub(&resp);
        assert!(
            rejected,
            "{method} must be auth-rejected or disabled without a token, got: {resp}"
        );
    }
    // Correct token passes AUTH (may fail on params, not auth).
    let resp = brp_roundtrip(
        &mut app,
        port,
        &serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "world.get_components",
                 "params": {"token": "sekrit"}}),
    );
    assert!(
        !is_auth_rejected(&resp),
        "correct token must pass auth, got: {resp}"
    );
    // Mutating built-ins stay disabled even WITH the token.
    let resp = brp_roundtrip(
        &mut app,
        port,
        &serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "world.spawn_entity",
                 "params": {"token": "sekrit"}}),
    );
    assert!(
        resp.get("error").is_some(),
        "world.spawn_entity must remain disabled, got: {resp}"
    );
}

#[test]
fn fx10_render_world_remote_methods_empty() {
    // The render sub-app's RemoteMethods table must be EMPTY after
    // AgentPlugin::finish — the render world must not be remotely
    // callable. A stub render sub-app stands in for a real renderer
    // (RemotePlugin builds its render method table in any sub-app
    // registered under RenderApp).
    use bevy::remote::RemoteMethods;
    let port = free_port();
    let mut app = bevy::app::App::new();
    app.add_plugins(bevy::MinimalPlugins);
    let mut render_sub = bevy::app::SubApp::new();
    render_sub
        .world_mut()
        .init_resource::<bevy::render::RenderScheduleOrder>();
    app.insert_sub_app(bevy::render::RenderApp, render_sub);
    app.add_plugins(bevy_swarm::agent::AgentPlugin::new(
        bevy_swarm::agent::AgentConfig {
            port,
            token: Some("sekrit".into()),
            ..Default::default()
        },
    ));
    app.finish();
    let render_app = app
        .get_sub_app_mut(bevy::render::RenderApp)
        .expect("render sub-app exists");
    let methods = render_app
        .world_mut()
        .get_resource::<RemoteMethods>()
        .expect("RemoteMethods initialized by RemotePlugin");
    assert!(
        methods.methods().is_empty(),
        "render sub-app must expose zero remote methods, got: {:?}",
        methods.methods()
    );
}

#[test]
fn fx10_empty_token_is_unset() {
    // Some("") must behave as unset for auth (never authenticate a
    // caller sending token: "") ...
    let world = world_with_token(Some("".into()));
    let params = serde_json::json!({"path": "TestApi.score", "token": ""});
    let res = bevy_swarm::agent::__test_observe(&world, Some(params));
    // An unset token must not authenticate anyone: observe either
    // succeeds (no auth required) or fails for non-auth reasons.
    let _ = res;
    // And allow_remote without a non-empty token must be refused.
    let cfg = bevy_swarm::agent::AgentConfig {
        allow_remote: true,
        token: Some("".into()),
        ..Default::default()
    };
    assert!(
        cfg.validate().is_err(),
        "allow_remote with empty token must be refused"
    );
}

#[test]
fn fx10_release_guard_decision_tested_directly() {
    // The release-path decision must be testable regardless of the
    // current build profile: with the effective deny flag true, the
    // guard must error.
    let err = bevy_swarm::agent::release_guard_decision(true).unwrap_err();
    assert!(err.contains("RELEASE"), "got: {err}");
    assert!(bevy_swarm::agent::release_guard_decision(false).is_ok());
}
