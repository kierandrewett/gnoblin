//! The Rust SDK for Gnoblin shells.
//!
//! A shell is an ordinary Rust crate built as a `cdylib`. It depends on this crate, which holds `extern` declarations of
//! the installed `libgnoblin-core` and safe wrappers over them, and never a copy of the compositor: the link is to the
//! library the user has installed, so a compatible update of that library changes what a shell runs on without
//! recompiling it. `gnoblin-core-abi` is the contract behind the wrappers.
//!
//! ```ignore
//! // Cargo.toml: [lib] crate-type = ["cdylib"]   [dependencies] gnoblin-core = "1"
//! use gnoblin_core as gnoblin;
//! use gnoblin_core::Gnoblin;
//!
//! #[gnoblin::main]
//! fn main(g: &Gnoblin) {
//!     g.log("hello from a shell");
//! }
//! ```
//!
//! Build it with `cargo build` and run it with `gnoblin ./target/debug/libmyshell.so`. The build needs a C compiler (the
//! link stub, see `gnoblin_core_abi::build_support`) and `panic = "unwind"`, which is Cargo's default.
//!
//! ## What is where
//!
//! - [`Gnoblin`]: a generation's handle on the compositor, which the entry function receives. It logs, spawns processes, binds keys,
//!   listens to events, and opens the typed object model.
//! - [`objects`]: the typed object model of the core: the root [`objects::ModelRoot`] and its windows, outputs, input, session and processes as
//!   classes with typed properties, methods and signals. `g.objects()?` is the way in.
//! - [`Window`] (also named [`Client`]) and [`Node`]: the tree of the compositor, for the windows and nodes a shell reads and moves.
//! - [`Poster`]: the handle another thread uses to log and to post work back to the shell thread.
//! - [`main`]: the attribute that marks the entry function.
//!
//! The desktop above the core (workspaces, layouts, styles and the scene) is the optional `gnoblin-std` crate. The guide to
//! [shells in Rust](https://gnoblin.org/docs/rust-shell) walks through a first shell, and the same API is also documented for
//! [TypeScript](https://gnoblin.org/reference/typescript/) and [Python](https://gnoblin.org/reference/python/).
//!
//! ## Threads
//!
//! Everything a shell does happens on one thread, the shell thread, and a [`Gnoblin`] belongs to it: it is neither `Send`
//! nor `Sync`. A thread a shell starts can hold a [`Poster`] (which is both) to log and to post work back.
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

pub use callback::CallbackResult;
pub use error::{CoreError, ErrorKind, Result};
pub use events::{NodeEvent, WindowRequest};
pub use generated::*;
pub use gnoblin::{CoreStats, Gnoblin, LogLevel, ModuleInfo, Poster, ScopedSubscription, Subscription};
pub use gnoblin_macros::main;
pub use gnoblin_object::Fail;
pub use input::{ChordEvent, ChordState, DragPhase, GestureEvent, GestureKind, GrabScope, PointerButton, PointerDrag};
pub use owned::Value;
pub use process::{Command, Process, ProcessExit, Signal, Stream};
pub use stage::WriteStage;
pub use timed::{Effect, EffectKind, Timing};
pub use tree::{Adoption, Answer, Client, Node, PendingCall, Rect};
pub use value::{CatalogMember, Class, MemberKind, NodeId, ShellValue, SignalEvent};

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
