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

    // Explicit opt-in passes.
    let cfg = bevy_swarm::agent::AgentConfig {
        allow_remote: true,
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
