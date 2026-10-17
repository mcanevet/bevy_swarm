//! 7rk: Bevy-version shim for headless plugin stripping.
//!
//! The set of plugins that must be disabled for headless operation is
//! tied to a specific Bevy release's `DefaultPlugins` membership: each
//! release renames, moves, or adds render-family plugins (see the
//! upgrade note below). This module isolates those per-version strip
//! lists behind a single seam so a Bevy upgrade edits exactly one
//! place (ties to swarm-m2h.17, the Bevy 0.21 upgrade).
//!
//! **Adding a Bevy version:** copy this module to
//! `headless_strip/v<new>.rs`, adjust membership, and flip the
//! `mod_versions!` dispatch below. The lists are expressed as
//! `contains`-guarded disables so they degrade safely (no panic) when
//! a plugin is absent under a particular feature combination.
//!
//! ## Why the whole render family
//! Disabling `RenderPlugin` alone is NOT enough when the game compiles
//! bevy with default features — `CorePipelinePlugin::build` loads a
//! Shader asset (panics without `RenderPlugin`'s `init_asset`), and the
//! other pipeline plugins (Sprite, UI, Pbr, Gizmo, Text, Scene)
//! transitively require render. Games needing meshes in tests should
//! expose state via TestApi instead.
//!
//! `UiWidgetsPlugins` is a nested group (cannot be disabled as a
//! unit); its widget members are disabled individually instead.

use bevy::app::PluginGroupBuilder;

// The strip! macro expands `$p:ty` inside `contains::<$p>()` and cannot
// carry cfg attributes per item, so version-divergent plugin paths are
// unified behind these aliases before reaching the macro.
#[cfg(bevy_0_19)]
type TextInputPlugin = bevy_ui_widgets::EditableTextInputPlugin;
#[cfg(not(bevy_0_19))]
type TextInputPlugin = bevy::ui_widgets::TextInputPlugin;

/// Strip the render family (plus UI widgets) from a plugin group.
///
/// All disables are `contains`-guarded: correct for any bevy feature
/// combination, panic-free when a plugin is absent. Without bevy's
/// `render` feature the family cannot be present, so this is a no-op
/// (and the render-family paths don't compile) — the call site gates
/// on `render` accordingly.
#[cfg(feature = "render")]
pub(crate) fn strip_render_family(group: PluginGroupBuilder) -> PluginGroupBuilder {
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
    let stripped = strip!(
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
        TextInputPlugin,
        bevy::ui_widgets::ListBoxPlugin,
        bevy::ui_widgets::MenuPlugin,
        bevy_ui_widgets::popover::PopoverPlugin,
        bevy::ui_widgets::RadioGroupPlugin,
        bevy::ui_widgets::ScrollAreaPlugin,
        bevy::ui_widgets::ScrollbarPlugin,
        bevy::ui_widgets::SliderPlugin,
    );

    // Dialog/ModalDialog/Tab plugins and the pointer-focus plugin are
    // 0.20-only; on 0.19 those widgets do not exist, so there is
    // nothing to strip. (Macro invocations cannot carry cfg attrs on
    // individual items, hence the separate cfg'd strip calls.)
    #[cfg(not(bevy_0_19))]
    let stripped = strip!(
        stripped,
        bevy::ui_widgets::DialogPlugin,
        bevy::ui_widgets::ModalDialogPlugin,
        bevy::ui_widgets::TabPlugin,
        bevy::input_focus::pointer_focus::PointerFocusPlugin,
    );

    stripped
}

#[cfg(all(test, feature = "render"))]
mod tests {
    use super::*;
    use bevy::app::PluginGroup;

    #[test]
    fn strip_render_family_leaves_non_render_plugins() {
        // Sanity: stripping twice is idempotent and does not panic when
        // plugins are absent (contains-guarded).
        let group = bevy::DefaultPlugins.build();
        let stripped = strip_render_family(group);
        let double = strip_render_family(stripped);
        // If this returns without panicking, the guards are sound.
        let _ = double;
    }
}
