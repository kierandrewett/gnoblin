//! The Rust SDK for Gnoblin shells.
//!
//! A shell is an ordinary Rust crate built as a `cdylib`. It depends on this crate, which holds `extern` declarations of
//! the installed `libgnoblin-core` and the typed classes of its object model, and never a copy of the compositor: the link is to
//! the library the user has installed, so a compatible update of that library changes what a shell runs on without
//! recompiling it. `gnoblin-core-abi` is the contract behind the classes.
//!
//! ```ignore
//! // Cargo.toml: [lib] crate-type = ["cdylib"]   [dependencies] gnoblin-core = "1"
//! use gnoblin_core as gnoblin;
//! use gnoblin_core::Gnoblin;
//!
//! #[gnoblin::main]
//! fn main(gnoblin: &Gnoblin) {
//!     gnoblin.log("hello from a shell");
//! }
//! ```
//!
//! Build it with `cargo build` and run it with `gnoblin ./target/debug/libmyshell.so`. The build needs a C compiler (the
//! link stub, see `gnoblin_core_abi::build_support`) and `panic = "unwind"`, which is Cargo's default.
//!
//! ## What is where
//!
//! - [`Gnoblin`]: the root of the object model, which the entry function receives: `clients`, `outputs`, `input`, `session`,
//!   `process` and the other services of the core, as classes with typed properties, methods and signals. It also logs
//!   ([`Gnoblin::log`]), binds key chords ([`Gnoblin::bind`]), keeps state across a reload ([`Gnoblin::keep_state`]) and gives the handle
//!   the classes are made from ([`Gnoblin::objects`]).
//! - [`Client`], [`Output`] and the rest of the classes of the model, at the root of this crate. They live in [`objects`] too.
//! - [`Fail`] and [`Res`]: the error a call of a class returns, and its `Result`. [`Listener`]: the handle of a listener a class
//!   made.
//! - [`main`]: the attribute that marks the entry function.
//!
//! The desktop above the core (workspaces, layouts, styles and the scene) is the optional `gnoblin-std` crate. The guide to
//! [shells in Rust](https://gnoblin.org/docs/rust-shell) walks through a first shell, and the same API is also documented for
//! [TypeScript](https://gnoblin.org/reference/typescript/) and [Python](https://gnoblin.org/reference/python/).
//!
//! [`host`] is the layer the language runtimes stand on (the node tree, signals, verbs and processes by name). It is plumbing for
//! them, not API for a shell.
//!
//! ## Threads
//!
//! Everything a shell does happens on one thread, the shell thread. A thread a shell starts can hold a
//! [`host::Poster`] (which is both `Send` and `Sync`) to log and to post work back.
//!
//! ## Panics
//!
//! A panic inside a callback is caught where the callback meets the core, reported to the core's log with the generation,
//! and the next event is still delivered. A panic in the shell's entry function fails the load with the message.

#[cfg(panic = "abort")]
compile_error!(
    "gnoblin shells and the core need panic = \"unwind\", and this build uses panic = \"abort\". A panic in a callback is \
     caught at the ABI boundary and logged; with abort it would end the compositor. Remove `panic = \"abort\"` from the \
     [profile.*] section that builds this crate."
);

mod callback;
mod error;
mod events;
// Written by `gnoblin-sdkgen` from `build.rs` into `OUT_DIR` (nothing of it is committed): the schema's names, the typed node
// classes and the error kinds.
mod generated {
    include!(concat!(env!("OUT_DIR"), "/rust-core/generated.rs"));
}
mod gnoblin;
mod input;
pub mod objects;
mod owned;
mod process;
mod stage;
mod sys;
mod timed;
mod tree;
mod value;
mod verbs;

#[doc(hidden)]
pub mod private;

/// Not public API: the names the macro expansion refers to.
#[doc(hidden)]
pub use private as __private;

pub use gnoblin_macros::main;
pub use gnoblin_object::typed::{Listener, Octets};
pub use gnoblin_object::{Fail, Res, Timing, Val};
pub use objects::*;

/// The layer the language runtimes (Python, JavaScript) stand on: the node tree of the compositor, signals and verbs by name,
/// processes, the write stage and the host handle. It is plumbing for them, not API for a shell: a shell uses the classes at the
/// root of this crate, and the documentation never teaches a path under `host`.
pub mod host {
    pub use crate::callback::CallbackResult;
    pub use crate::error::{CoreError, ErrorKind, Result};
    pub use crate::events::{NodeEvent, WindowRequest};
    pub use crate::generated::*;
    pub use crate::gnoblin::{CoreStats, Host, LogLevel, ModuleInfo, Poster, ScopedSubscription, Subscription};
    pub use crate::input::{
        ChordEvent, ChordState, DragPhase, GestureEvent, GestureKind, GrabScope, PointerButton, PointerDrag,
    };
    pub use crate::owned::Value;
    pub use crate::process::{Command, Process, ProcessExit, Signal, Stream};
    pub use crate::stage::WriteStage;
    pub use crate::timed::{Effect, EffectKind, Timing};
    pub use crate::tree::{Adoption, Answer, Client, Node, PendingCall, Rect};
    pub use crate::value::{CatalogMember, Class, MemberKind, NodeId, ShellValue, SignalEvent};
}

/// The ABI major version this SDK was built for. A core of another major refuses to load the shell.
pub const ABI_MAJOR: u32 = gnoblin_core_abi::ABI_MAJOR;

/// The ABI minor version this SDK was built against. An older core runs the shell; a function added in a later minor is
/// reported as unsupported by the wrapper that needs it.
pub const ABI_MINOR: u32 = gnoblin_core_abi::ABI_MINOR;

/// The ABI version of the installed core, as `(major, minor, patch)`. Needs the core to be loaded, which it is inside a
/// shell.
pub fn installed_abi() -> (u32, u32, u32) {
    gnoblin::installed_abi_version()
}

/// A hash of the schema catalog the installed core was built with, for a quick check that generated constants match.
#[allow(unsafe_code)]
pub fn catalog_hash() -> u64 {
    // SAFETY: `gnoblin_catalog_hash` takes no arguments and only reads a constant.
    unsafe { sys::gnoblin_catalog_hash() }
}
