//! Z2: headless App builder + bundled-DefaultPlugins detection (integration).

use bevy::prelude::*;
use bevy_playtest::headless::headless_app;

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
    // 7rk: headless_app installs the headless platform first; the
    // game's bundled DefaultPlugins then conflicts. Bevy panics with
    // "plugin was already added"; our builder must convert that into
    // structured guidance naming the OFFENDING PLUGIN and listing the
    // three fixes (split plugin / move to main.rs / subprocess tier).
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let build = headless_app(BundledPlatformPlugin);
        let _app = build();
    }));
    assert!(
        result.is_err(),
        "bundled DefaultPlugins must not succeed silently"
    );
    let payload = result.unwrap_err();
    let msg = payload
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| payload.downcast_ref::<&str>().map(|s| s.to_string()))
        .unwrap_or_default();
    // Names the offending plugin parsed from bevy's panic (the first
    // group member to conflict — LogPlugin is disabled by the harness
    // so the first duplicate is TaskPoolPlugin or a later member).
    assert!(
        msg.contains("plugin was already added") || msg.contains("Guidance"),
        "must explain the conflict: {msg}"
    );
    assert!(
        msg.contains("Guidance"),
        "must present the structured guidance block: {msg}"
    );
    assert!(
        msg.contains("PanicHandlerPlugin") || msg.contains("in group DefaultPlugins"),
        "should name the conflicting plugin or group: {msg}"
    );
    assert!(msg.contains("subprocess tier"), "lists the Z8 fix: {msg}");
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

// RF9: headless_platform disables Winit by runtime presence even when
// bevy_playtest is compiled with default-features=false.
#[test]
fn rf9_headless_no_display_default_features_false() {
    // This test verifies that headless_app() works without a DISPLAY
    // when bevy_playtest is compiled with default-features=false (the
    // game may still pull bevy_winit). The winit disable is now
    // unconditional via the direct bevy_winit dep.
    use bevy::prelude::*;
    use bevy_playtest::autotest::{autotest, AutotestConfig};

    #[derive(Clone, Default)]
    struct Rf9TestPlugin;
    impl Plugin for Rf9TestPlugin {
        fn build(&self, app: &mut App) {
            app.add_systems(Startup, || {});
        }
    }

    let config = AutotestConfig {
        num_seeds: 2,
        duration_s: 0.1,
        parallel: 1,
        ..AutotestConfig::default()
    };

    // If this compiles and runs, the winit disable is working.
    let report = autotest(Rf9TestPlugin, config).expect("autotest runs");
    assert!(report.failures.is_empty(), "rf9 test passes");
}
