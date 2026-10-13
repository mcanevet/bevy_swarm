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
    // RF9: gated on passthrough-winit — a weak dep enabled by
    // render/audio/gamepad, or explicitly by a consumer whose game pulls
    // bevy_winit while bevy_swarm is default-features=false.
    #[cfg(feature = "passthrough-winit")]
    let group = if group.contains::<bevy_winit::WinitPlugin>() {
        group.disable::<bevy_winit::WinitPlugin>()
    } else {
        group
    };

    // Render: requires GPU; disable for headless operation.
    // FX4 re-review (Z10 e2e): disabling RenderPlugin alone is NOT
    // enough when the game compiles bevy with default features —
    // CorePipelinePlugin::build loads a Shader asset (panics without
    // RenderPlugin's init_asset), and the other pipeline plugins
    // (Sprite, UI, Pbr, Gizmo, Text, Scene) transitively require
    // render. Disable the whole render family; games needing meshes
    // in tests should expose state via TestApi instead.
    #[cfg(feature = "render")]
    let group = if group.contains::<bevy_render::RenderPlugin>() {
        // Disable the entire render family (Z10 e2e): individual
        // pipeline plugins (CorePipeline, PostProcess, ...) load
        // Shader assets during build and panic without RenderPlugin's
        // init_asset; render is impossible headless anyway. `contains`
        // guards keep this correct for any bevy feature combination.
        // UiWidgetsPlugins is a nested GROUP (cannot be disabled as a
        // unit); its widget members are disabled individually instead.
        macro_rules! strip {
            ($g:expr, $($p:ty),+ $(,)?) => {{
                let mut g = $g;
                $(
                    g = if g.contains::<$p>() {
                        g.disable::<$p>()
                    } else {
                        g
                    };
                )+
                g
            }};
        }
        strip!(
            group,
            bevy_render::RenderPlugin,
            bevy::image::ImagePlugin,
            bevy::mesh::MeshPlugin,
            bevy::camera::CameraPlugin,
            bevy::light::LightPlugin,
            bevy_render::pipelined_rendering::PipelinedRenderingPlugin,
            bevy::core_pipeline::CorePipelinePlugin,
            bevy::post_process::PostProcessPlugin,
            bevy::anti_alias::AntiAliasPlugin,
            bevy::sprite::SpritePlugin,
            bevy::sprite_render::SpriteRenderPlugin,
            bevy::text::TextPlugin,
            bevy::ui::UiPlugin,
            bevy::ui_render::UiRenderPlugin,
            bevy::gltf::GltfPlugin,
            bevy::pbr::PbrPlugin,
            bevy::animation::AnimationPlugin,
            bevy::gizmos::GizmoPlugin,
            bevy::gizmos_render::GizmoRenderPlugin,
            bevy::scene::ScenePlugin,
            bevy::ui_widgets::ButtonPlugin,
            bevy::ui_widgets::CheckboxPlugin,
            bevy::ui_widgets::TextInputPlugin,
            bevy::ui_widgets::ListBoxPlugin,
            bevy::ui_widgets::MenuPlugin,
            bevy::ui_widgets::DialogPlugin,
            bevy::ui_widgets::ModalDialogPlugin,
            bevy_ui_widgets::popover::PopoverPlugin,
            bevy::ui_widgets::RadioGroupPlugin,
            bevy::ui_widgets::ScrollAreaPlugin,
            bevy::ui_widgets::ScrollbarPlugin,
            bevy::ui_widgets::SliderPlugin,
            bevy::ui_widgets::TabPlugin,
            bevy::input_focus::pointer_focus::PointerFocusPlugin,
        )
    } else {
        group
    };

    // Audio: rodio backend may block; disable for headless.
    #[cfg(feature = "audio")]
    let group = if group.contains::<bevy_audio::AudioPlugin>() {
        group.disable::<bevy_audio::AudioPlugin>()
    } else {
        group
    };

    // Gilrs: gamepad controllers; disable for headless.
    #[cfg(feature = "gamepad")]
    let group = if group.contains::<bevy_gilrs::GilrsPlugin>() {
        group.disable::<bevy_gilrs::GilrsPlugin>()
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
/// ## Example
/// ```no_run
/// use bevy::prelude::*;
/// use bevy_swarm::headless::headless_app;
/// use bevy_swarm::harness::{run_scenario, Scenario};
///
/// #[derive(Clone, Default)]
/// struct MyGamePlugin;
/// impl Plugin for MyGamePlugin {
///     fn build(&self, _: &mut App) {}
/// }
///
/// let build = headless_app(MyGamePlugin);
/// let scenario: Scenario = serde_json::from_str(
///     r#"{"bot":{"type":"chaos","seed":1},"duration_s":0.5,"invariants":[]}"#
/// ).unwrap();
/// let mut app = build(); // fresh App per run
/// let report = run_scenario(&mut app, &scenario).unwrap();
/// assert_eq!(report.status, bevy_swarm::enums::PlaytestStatus::Pass);
/// ```
///
/// ## Panics
/// Panics if the game plugin conflicts with platform plugins (e.g., the
/// game bundles its own `DefaultPlugins`). In that case, migrate the
/// game's plugin to `main.rs` or use the subprocess tier (Z8).
pub fn headless_app<P: Plugin + Clone + Send + Sync + 'static>(
    game_plugin: P,
) -> impl Fn() -> App + Send + Sync + 'static {
    headless_app_factory(move || game_plugin.clone())
}

/// RF8: factory-based variant — games without a `Clone` plugin (the
/// common case for game plugins holding non-Clone state) can pass a
/// constructor closure instead. Game-type inference uses the type the
/// FACTORY constructs, so wrapping the plugin no longer breaks Z6.
pub fn headless_app_factory<P, F>(make_plugin: F) -> impl Fn() -> App + Send + Sync + 'static
where
    P: Plugin + Send + Sync + 'static,
    F: Fn() -> P + Send + Sync + 'static,
{
    move || {
        let mut app = App::new();
        let platform = headless_platform();

        // le3.11 (RF7): disable LogPlugin's default subscriber and
        // install our capture layer via custom_layer so log capture
        // works even when the harness runs in-process (LogPlugin no
        // longer beats ensure_global_subscriber).
        let platform = platform.disable::<bevy::log::LogPlugin>();
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

        let plugin = make_plugin();
        // FX4.1: add PlaytestPlugin EARLY (before the game plugin) so
        // its panic hook catches panics in the game's Startup systems.
        if !app.is_plugin_added::<crate::driver::PlaytestPlugin>() {
            app.add_plugins(crate::driver::PlaytestPlugin);
        }
        app.add_plugins(plugin);
        // le3.11 (RF7): install LogPlugin with the swarm capture layer
        // as custom_layer — integrates with the game's logging while
        // ensuring our capture layer sees all events.
        let log_plugin = bevy::log::LogPlugin {
            custom_layer: crate::sinks::log_custom_layer,
            ..default()
        };
        if !app.is_plugin_added::<bevy::log::LogPlugin>() {
            app.add_plugins(log_plugin);
        }
        // Z6: classify game-owned types from the game plugin's crate
        // path so gameplay inference can run without annotations.
        app.insert_resource(crate::game_types::GameTypes::from_plugin::<P>());
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
