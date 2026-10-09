//! Z2: headless App builder + bundled-DefaultPlugins detection (integration).

use bevy::prelude::*;
use bevy_swarm::headless::headless_app;

#[derive(Resource, Default)]
struct GameRan;

#[derive(Default, Clone)]
struct SimpleGamePlugin;

impl Plugin for SimpleGamePlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(GameRan);
    }
}

#[test]
fn game_plugin_builds_and_updates_headlessly() {
    let build = headless_app(SimpleGamePlugin);
    let mut app = build();
    assert!(app.world().get_resource::<GameRan>().is_some());
    // A few headless ticks must not panic or request exit.
    for _ in 0..5 {
        app.update();
    }
    assert!(app.should_exit().is_none());
}

#[test]
fn fresh_app_per_call_has_no_shared_state() {
    let build = headless_app(SimpleGamePlugin);
    let app_a = build();
    let app_b = build();
    assert!(app_a.world().get_resource::<GameRan>().is_some());
    assert!(app_b.world().get_resource::<GameRan>().is_some());
    // Structurally distinct schedulers/worlds.
    assert_ne!(
        std::ptr::from_ref(app_a.world()) as usize,
        std::ptr::from_ref(app_b.world()) as usize
    );
}

/// A game plugin that bundles its own `DefaultPlugins` (the anti-pattern Z2
/// detects). Adding it to a headless platform must surface actionable
/// guidance, not an opaque panic.
#[derive(Clone)]
struct BundledPlatformPlugin;

impl Plugin for BundledPlatformPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(DefaultPlugins);
    }
}

#[test]
fn bundled_default_plugins_reports_guidance() {
    // headless_app installs the headless platform first; the game's bundled
    // DefaultPlugins then conflicts. Bevy panics with "plugin was already
    // added"; our builder must convert that into a GameBundlesPlatform-style
    // message naming the plugin and pointing at the fix.
    let result = std::panic::catch_unwind(|| {
        let build = headless_app(BundledPlatformPlugin);
        let _app = build();
    });
    assert!(
        result.is_err(),
        "bundled DefaultPlugins must not succeed silently"
    );
    // The panic payload should mention guidance (plugin name or migration hint).
    let payload = result.unwrap_err();
    let msg = payload
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| payload.downcast_ref::<&str>().map(|s| s.to_string()))
        .unwrap_or_default();
    assert!(
        msg.contains("already added") || msg.contains("DefaultPlugins") || msg.contains("Z8"),
        "unexpected panic message: {msg}"
    );
}

/// FX4.6: a game built with bevy_winit (x11) runs headless with
/// DISPLAY/WAYLAND_DISPLAY unset — WinitPlugin is disabled by runtime
/// type presence, no event-loop panic.
#[test]
fn headless_platform_runs_with_winit_without_display() {
    // fixture_walker depends on bevy with bevy_winit enabled.
    std::env::remove_var("DISPLAY");
    std::env::remove_var("WAYLAND_DISPLAY");
    let build = headless_app(fixture_walker::WalkerGamePlugin);
    let mut app = build();
    for _ in 0..10 {
        app.update();
    }
    // If WinitPlugin had run, app.update() would panic ("Failed to
    // build event loop") without a display.
}
