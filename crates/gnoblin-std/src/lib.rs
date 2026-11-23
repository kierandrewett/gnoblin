//! The Rust SDK for the optional Gnoblin desktop library.
//!
//! `gnoblin-core` is the machinery: the tree, the windows, the input claims. `gnoblin-std` is the desktop built on it: [`Client`]s and
//! live [`Collection`]s of them, [`Workspace`]s, [`Layout`]s, the policies that share workspaces between outputs, and typed listeners
//! and [`Emitter`]s. A shell that wants none of it links the core alone and never loads the std library; a shell that wants it writes
//! `use gnoblin_std::prelude::*;`.
//!
//! This crate holds `extern` declarations of the installed `libgnoblin-std` and safe wrappers over its function table, and never a copy
//! of the library: the link is to the library the user has installed, so a compatible update changes what a shell runs on without
//! recompiling it. The process then maps the chain shell -> `libgnoblin-std.so.1` -> `libgnoblin-core.so.1`, once each.
//! `gnoblin-std-abi` is the contract.
//!
//! ```ignore
//! // Cargo.toml: [lib] crate-type = ["cdylib"]   [dependencies] gnoblin-core = "1"  gnoblin-std = "1"
//! use gnoblin_std::prelude::*;
//!
//! #[gnoblin::main]
//! fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
//!     // One floating workspace for every output, and every window that opens joins the one under the pointer.
//!     g.std()?.workspace_per_output()?;
//!     g.bind("Super+Return", || { g.spawn("foot").ok(); })?;
//!     Ok(())
//! }
//! ```
//!
//! ## What is where
//!
//! - [`Std`]: a generation's handle on the library (`g.std()?`); the roots [`clients`](Std::clients), [`outputs`](Std::outputs),
//!   [`workspaces`](Std::workspaces); the policies ([`OutputPolicy`], [`WorkspacePolicy`]).
//! - [`Client`]: a window, dereferencing to the core's [`Window`](gnoblin_core::Window).
//! - [`Collection`]: a live view, with snapshots (`to_vec`, `filter`, `find`, `sorted_by`), selections ([`where_`](Collection::where_)
//!   with a [`Query`]) and listeners (`on_added`, `on_title_changed`, [`ClientListener`]).
//! - [`Workspace`]: membership, layout, gaps, showing on an output.
//! - [`Layout`]: the trait a shell implements for its own layout; [`FloatingLayout`] is the library's, and
//!   [`MasterStackLayout`], [`GridLayout`], [`MonocleLayout`], [`DwindleLayout`], [`StripLayout`], [`ArcLayout`] and
//!   [`Carousel3dLayout`] name the tiling layouts the library has built in (typed options and actions; the algorithms are the
//!   library's, so every language gets the same).
//! - [`ClientMoveBinding`], [`ClientResizeBinding`] and [`DragBinding`]: pointer drags, claimed with [`Std::bind_drag`].
//! - [`Emitter`]: events of the shell's own.
//!
//! ## Threads, panics and reloads
//!
//! As for the core SDK: everything happens on the shell thread, a panic in a callback is caught where the callback meets the library,
//! and a handle that outlives its generation fails with `Retired`. The library never calls a callback inside a call you made: a change
//! is announced from the next turn of the dispatch, with the other events of that turn.

mod animation;
mod api;
mod bindings;
mod builtin;
mod client;
mod client_state;
mod collection;
mod desktop;
mod emitter;
mod error;
mod gesture;
mod layout;
mod layouts;
pub mod objects;
mod output;
mod query;
mod scene;
mod split;
mod style;
mod subscription;
mod sys;
mod workspace;

pub use animation::{
    Easing, PropValue, Transition, TransitionInfo, TransitionOptions, TransitionResult, TransitionState, Tx,
};
pub use api::{Stats, Std};
pub use bindings::{
    ClientMoveBinding, ClientResizeBinding, ClientSwapBinding, DRAG_THRESHOLD, DragBinding, DragConstraint, DragEvent,
    IntoDragBinding, Point, ResizeCorner, ResizeGrip, SplitResizeBinding,
};
pub use builtin::{BuiltinLayout, LayoutValue};
pub use client::{AsClient, Client, FramePolicy};
pub use client_state::{FrameInset, SnapZone};
pub use collection::{ClientListener, Collection, CollectionListener, Member};
pub use desktop::{GnoblinStdExt, OutputPolicy, PerOutput, WorkspacePolicy};
pub use emitter::{Emitter, Listener};
pub use error::{Error, ErrorKind, Result};
pub use gesture::{GestureBinding, GestureDirection, GestureInfo, GestureStage, PINCH_COMMIT, SWIPE_COMMIT};
pub use layout::{FloatPlacement, FloatingLayout, IntoLayout, Layout, LayoutContext, Placement, inset_rect};
pub use layouts::{
    ArcLayout, Carousel3dLayout, Columns, DwindleLayout, DwindleOptions, ForceSplit, GridFill, GridLayout, KioskLayout,
    MasterStackLayout, MonocleLayout, Orientation, PointerSource, Preselect, RingOptions, RingSlot, SplitBias,
    SplitMode, StripLayout,
};
pub use output::Output;
pub use query::{Query, QueryValue};
pub use scene::{
    Camera2D, Camera2DOptions, Camera3D, Camera3DOptions, InputMode, Projection, Scene, SceneNode, View, ViewOptions,
};
pub use split::{Divider, Split, SplitAxis};
pub use style::{Border, ComputedStyle, CornerRadius, Filter, MAX_STYLE_LIST, Origin, Shadow, Style, StyleProperty};
pub use subscription::{ListenerHandle, ScopedListener, ScopedSubscription, Subscription};
pub use workspace::{ShowOptions, Workspace, WorkspaceBuilder, WorkspaceInfo};

/// The std ABI major version this SDK was built for. A library of another major refuses to load the shell's `open`.
pub const ABI_MAJOR: u32 = gnoblin_std_abi::ABI_MAJOR;

/// The std ABI minor version this SDK was built against.
pub const ABI_MINOR: u32 = gnoblin_std_abi::ABI_MINOR;

/// The ABI version of the installed std library, as `(major, minor, patch)`. Needs the library to be loaded, which it is inside a
/// std shell.
pub fn installed_abi() -> (u32, u32, u32) {
    Std::installed_abi()
}

/// The names a desktop shell wants in scope: `use gnoblin_std::prelude::*;`.
///
/// It re-exports the core SDK under the name `gnoblin`, so `#[gnoblin::main]` works with nothing else imported, and the core's
/// [`Gnoblin`](gnoblin_core::Gnoblin), [`Window`](gnoblin_core::Window), [`Rect`](gnoblin_core::Rect) and
/// [`NodeId`](gnoblin_core::NodeId). The core's `Client` alias of `Window` is not re-exported: the prelude's [`Client`] is the std one.
pub mod prelude {
    pub use gnoblin_core as gnoblin;
    pub use gnoblin_core::{Gnoblin, NodeId, Rect, Window};

    pub use crate::{
        ArcLayout, AsClient, Carousel3dLayout, Client, ClientListener, ClientMoveBinding, ClientResizeBinding,
        ClientSwapBinding, Collection, CollectionListener, Columns, DragBinding, DragEvent, DwindleLayout, Emitter,
        FloatingLayout, FrameInset, FramePolicy, GnoblinStdExt, GridFill, GridLayout, Layout, LayoutContext,
        MasterStackLayout, MonocleLayout, Orientation, Output, OutputPolicy, Placement, Preselect, Query, ShowOptions,
        SnapZone, SplitResizeBinding, Std, StripLayout, Workspace, WorkspacePolicy,
    };

    pub use crate::{
        Border, Camera2D, Camera2DOptions, Camera3D, Camera3DOptions, CornerRadius, Easing, Filter, InputMode,
        Projection, Scene, SceneNode, Shadow, Style, Transition, TransitionOptions, TransitionResult, View,
        ViewOptions,
    };

    // The members this library adds to the core's typed root and output collections: `model.workspaces()?`, `model.request_transition(..)`,
    // `outputs.profiles()?`. A shell that does not use this prelude has none of them.
    pub use crate::objects::{ModelRootStdExt, OutputCollectionStdExt};
}
