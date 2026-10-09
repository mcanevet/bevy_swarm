//! Z2: Headless platform builder + bundled-DefaultPlugins detection.
//!
//! Provides `headless_platform()` — a `DefaultPlugins`-minus-OS/GPU variant,
//! and `headless_app::<GamePlugin>(game)` which constructs a fresh headless
//! `App` with the game's plugin installed, all platform plugins disabled
//! safely (feature-guarded, panic-free).
//!
//! ## Verified Bevy 0.20-rc.2 facts
//! - `PluginGroupBuilder::disable::<T>()` panics if `T` is absent.
//! - `PluginGroupBuilder::contains::<T>()` exists for safe gating.
//! - Type paths like `bevy::winit::WinitPlugin` only compile when the
//!   corresponding Bevy feature is enabled.
//! - `RenderPlugin { render_creation: WgpuSettings { backends: None } }`
//!   skips render sub-app creation.
//! - `WindowPlugin { primary_window: Some(..), exit_condition: ExitCondition::DontExit }`
//!   prevents auto-exit.
//!
//! ## Feature parity with Bevy
//! `bevy_swarm` mirrors Bevy's platform features (`render`, `winit`, `audio`,
//! `gilrs`, `log`). Under each `#[cfg(feature)]`, we guard disables with
//! `contains::<T>()` to avoid panics when the plugin isn't present.

use bevy::app::{App, Plugin, PluginGroupBuilder};
use bevy::prelude::*;
use bevy::window::ExitCondition;
use std::fmt;

/// Error returned when the game bundles platform plugins that conflict
/// with headless construction.
#[derive(Debug)]
pub struct GameBundlesPlatform {
    /// Name of the conflicting plugin type.
    pub plugin: &'static str,
    /// Guidance for resolving the conflict.
    pub guidance: String,
}

impl fmt::Display for GameBundlesPlatform {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Game bundles platform plugin {}: {}",
            self.plugin, self.guidance
        )
    }
}

impl std::error::Error for GameBundlesPlatform {}

/// Build a headless `DefaultPlugins` group (minus OS/GPU/process-global bits).
///
/// Configures WindowPlugin to prevent auto-exit. Other platform plugins are
/// left as-is; games that need them disabled should do so explicitly in main.rs.
pub fn headless_platform() -> PluginGroupBuilder {
    let group = DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: "bevy_swarm headless".to_string(),
            visible: false,
            ..default()
        }),
        exit_condition: ExitCondition::DontExit,
        ..default()
    });

    // FX4.6: strip OS/GPU/process-global plugins by TYPE PRESENCE in
    // the group — independent of bevy_swarm's own feature cfgs. A game
    // compiled with default bevy features pulls in WinitPlugin (needs
    // a display), AudioPlugin (rodio), GilrsPlugin (controllers), and
    // TerminalCtrlPlugin (Ctrl-C handlers) — all unusable or unwanted
    // in a headless harness. `contains` guards the disables so this
    // stays correct for any feature combination.
    // Winit: requires an event loop; without DISPLAY it panics in build.
    let group = if group.contains::<bevy::winit::WinitPlugin>() {
        group.disable::<bevy::winit::WinitPlugin>()
    } else {
        group
    };

    // Render: requires GPU; disable for headless operation.
    let group = if group.contains::<bevy::render::RenderPlugin>() {
        group.disable::<bevy::render::RenderPlugin>()
    } else {
        group
    };

    // Audio: rodio backend may block; disable for headless.
    let group = if group.contains::<bevy::audio::AudioPlugin>() {
        group.disable::<bevy::audio::AudioPlugin>()
    } else {
        group
    };

    // Gilrs: gamepad controllers; disable for headless.
    let group = if group.contains::<bevy::gilrs::GilrsPlugin>() {
        group.disable::<bevy::gilrs::GilrsPlugin>()
    } else {
        group
    };

    // TerminalCtrl: Ctrl-C handlers; disable to avoid process-global state.
    if group.contains::<bevy::app::TerminalCtrlCHandlerPlugin>() {
        group.disable::<bevy::app::TerminalCtrlCHandlerPlugin>()
    } else {
        group
    }
}
/// Construct a fresh headless `App` with the game's plugin installed.
///
/// - Calls `headless_platform()` to get a sanitized `DefaultPlugins`.
/// - Adds the game plugin `P`.
/// - Finishes the app (no pending plugins).
/// - Returns a closure that produces a fresh `App` on each call (for
///   repeated test runs without cross-contamination).
///
/// ## Panics
/// Panics if the game plugin conflicts with platform plugins (e.g., the
/// game bundles its own `DefaultPlugins`). In that case, migrate the
/// game's plugin to `main.rs` or use the subprocess tier (Z8).
pub fn headless_app<P: Plugin + Clone + Send + Sync + 'static>(
    game_plugin: P,
) -> impl Fn() -> App + Send + Sync + 'static {
    move || {
        let mut app = App::new();
        let platform = headless_platform();

        // Try to add platform plugins; detect bundling conflicts.
        if let Err(e) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            app.add_plugins(platform);
        })) {
            let plugin_name = std::any::type_name::<P>();
            panic!(
                "Failed to add headless platform plugins. The game may bundle DefaultPlugins.\n\
                 Consider moving the game plugin ({}) to main.rs and using bevy_swarm's \
                 headless_app() wrapper, or use the subprocess tier (Z8) for unmodified binaries.\n\
                 Original panic: {:?}",
                plugin_name, e
            );
        }

        app.add_plugins(game_plugin.clone());
        // FX4.1: add PlaytestPlugin once (bots + oracles). Guard against
        // double-add in case the harness is reused.
        if !app.is_plugin_added::<crate::driver::PlaytestPlugin>() {
            app.add_plugins(crate::driver::PlaytestPlugin);
        }
        // Z6: classify game-owned types from the game plugin's crate
        // path so gameplay inference can run without annotations.
        app.insert_resource(crate::game_types::GameTypes::from_plugin::<P>());
        app.finish();
        app.cleanup();
        app
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headless_platform_builds_without_panic() {
        let _platform = headless_platform();
        // If this compiles and runs without panic, the feature guards work.
    }

    #[test]
    fn headless_app_produces_fresh_apps() {
        #[derive(bevy::ecs::prelude::Resource, Default)]
        struct Marker;

        #[derive(Default, Clone)]
        struct TestGamePlugin;

        impl Plugin for TestGamePlugin {
            fn build(&self, app: &mut App) {
                app.insert_resource(Marker);
            }
        }

        let builder = headless_app(TestGamePlugin);
        let app1 = builder();
        let app2 = builder();

        assert!(app1.world().get_resource::<Marker>().is_some());
        assert!(app2.world().get_resource::<Marker>().is_some());
    }
}
