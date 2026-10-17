//! Cross-version compatibility shims between Bevy 0.19 and 0.20.
//!
//! Bevy 0.19 dispatches picking events as `Pointer<E>` wrapping unprefixed
//! marker structs (`Over`, `Click`, ...); 0.20 flattened this into
//! standalone `PointerE` structs (`PointerOver`, `PointerClick`, ...).
//! The `picking` module presents the 0.20 spelling on both versions via
//! cfg'd aliases, so the rest of the crate uses one name.
//!
//! Other differences handled here:
//! - lifecycle `On` triggers: `On<Add<T>>` (0.20) vs `On<Add, T>` (0.19)

pub mod picking {
    //! Picking events under their 0.20 names on both Bevy versions.

    #[cfg(bevy_0_19)]
    pub type Cancel = bevy::picking::events::Pointer<bevy::picking::events::Cancel>;
    #[cfg(not(bevy_0_19))]
    pub use bevy::picking::events::PointerCancel as Cancel;

    #[cfg(bevy_0_19)]
    pub type Click = bevy::picking::events::Pointer<bevy::picking::events::Click>;
    #[cfg(not(bevy_0_19))]
    pub use bevy::picking::events::PointerClick as Click;

    #[cfg(bevy_0_19)]
    pub type Drag = bevy::picking::events::Pointer<bevy::picking::events::Drag>;
    #[cfg(not(bevy_0_19))]
    pub use bevy::picking::events::PointerDrag as Drag;

    #[cfg(bevy_0_19)]
    pub type DragDrop = bevy::picking::events::Pointer<bevy::picking::events::DragDrop>;
    #[cfg(not(bevy_0_19))]
    pub use bevy::picking::events::PointerDragDrop as DragDrop;

    #[cfg(bevy_0_19)]
    pub type DragEnd = bevy::picking::events::Pointer<bevy::picking::events::DragEnd>;
    #[cfg(not(bevy_0_19))]
    pub use bevy::picking::events::PointerDragEnd as DragEnd;

    #[cfg(bevy_0_19)]
    pub type DragEnter = bevy::picking::events::Pointer<bevy::picking::events::DragEnter>;
    #[cfg(not(bevy_0_19))]
    pub use bevy::picking::events::PointerDragEnter as DragEnter;

    #[cfg(bevy_0_19)]
    pub type DragLeave = bevy::picking::events::Pointer<bevy::picking::events::DragLeave>;
    #[cfg(not(bevy_0_19))]
    pub use bevy::picking::events::PointerDragLeave as DragLeave;

    #[cfg(bevy_0_19)]
    pub type DragOver = bevy::picking::events::Pointer<bevy::picking::events::DragOver>;
    #[cfg(not(bevy_0_19))]
    pub use bevy::picking::events::PointerDragOver as DragOver;

    #[cfg(bevy_0_19)]
    pub type DragStart = bevy::picking::events::Pointer<bevy::picking::events::DragStart>;
    #[cfg(not(bevy_0_19))]
    pub use bevy::picking::events::PointerDragStart as DragStart;

    #[cfg(bevy_0_19)]
    pub type Enter = bevy::picking::events::Pointer<bevy::picking::events::Enter>;
    #[cfg(not(bevy_0_19))]
    pub use bevy::picking::events::PointerEnter as Enter;

    #[cfg(bevy_0_19)]
    pub type Leave = bevy::picking::events::Pointer<bevy::picking::events::Leave>;
    #[cfg(not(bevy_0_19))]
    pub use bevy::picking::events::PointerLeave as Leave;

    #[cfg(bevy_0_19)]
    pub type Move = bevy::picking::events::Pointer<bevy::picking::events::Move>;
    #[cfg(not(bevy_0_19))]
    pub use bevy::picking::events::PointerMove as Move;

    #[cfg(bevy_0_19)]
    pub type Out = bevy::picking::events::Pointer<bevy::picking::events::Out>;
    #[cfg(not(bevy_0_19))]
    pub use bevy::picking::events::PointerOut as Out;

    #[cfg(bevy_0_19)]
    pub type Over = bevy::picking::events::Pointer<bevy::picking::events::Over>;
    #[cfg(not(bevy_0_19))]
    pub use bevy::picking::events::PointerOver as Over;

    #[cfg(bevy_0_19)]
    pub type Press = bevy::picking::events::Pointer<bevy::picking::events::Press>;
    #[cfg(not(bevy_0_19))]
    pub use bevy::picking::events::PointerPress as Press;

    #[cfg(bevy_0_19)]
    pub type Release = bevy::picking::events::Pointer<bevy::picking::events::Release>;
    #[cfg(not(bevy_0_19))]
    pub use bevy::picking::events::PointerRelease as Release;

    #[cfg(bevy_0_19)]
    pub type Scroll = bevy::picking::events::Pointer<bevy::picking::events::Scroll>;
    #[cfg(not(bevy_0_19))]
    pub use bevy::picking::events::PointerScroll as Scroll;

    pub use bevy::picking::events::PointerState;
}

pub mod ecs {
    //! Lifecycle-trigger `On` parameter aliases: 0.20 nests the bundle
    //! inside the event type (`On<Add<T>>`); 0.19 takes the bundle as a
    //! second type parameter (`On<Add, T>`).

    #[cfg(bevy_0_19)]
    pub type OnAdd<'w, 't, B> = bevy::ecs::observer::On<'w, 't, bevy::ecs::lifecycle::Add, B>;
    #[cfg(not(bevy_0_19))]
    pub type OnAdd<'w, 't, B> = bevy::ecs::observer::On<'w, 't, bevy::ecs::lifecycle::Add<B>>;

    #[cfg(bevy_0_19)]
    pub type OnInsert<'w, 't, B> = bevy::ecs::observer::On<'w, 't, bevy::ecs::lifecycle::Insert, B>;
    #[cfg(not(bevy_0_19))]
    pub type OnInsert<'w, 't, B> = bevy::ecs::observer::On<'w, 't, bevy::ecs::lifecycle::Insert<B>>;

    #[cfg(bevy_0_19)]
    pub type OnDiscard<'w, 't, B> =
        bevy::ecs::observer::On<'w, 't, bevy::ecs::lifecycle::Discard, B>;
    #[cfg(not(bevy_0_19))]
    pub type OnDiscard<'w, 't, B> =
        bevy::ecs::observer::On<'w, 't, bevy::ecs::lifecycle::Discard<B>>;

    #[cfg(bevy_0_19)]
    pub type OnRemove<'w, 't, B> = bevy::ecs::observer::On<'w, 't, bevy::ecs::lifecycle::Remove, B>;
    #[cfg(not(bevy_0_19))]
    pub type OnRemove<'w, 't, B> = bevy::ecs::observer::On<'w, 't, bevy::ecs::lifecycle::Remove<B>>;
}
