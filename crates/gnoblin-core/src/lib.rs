//! The Rust SDK for Gnoblin shells.
//!
//! A shell is an ordinary Rust crate built as a `cdylib`. It depends on this crate, which holds `extern` declarations of
//! the installed `libgnoblin-core` and safe wrappers over them, and never a copy of the compositor: the link is to the
//! library the user has installed, so a compatible update of that library changes what a shell runs on without
//! recompiling it. `design/modular-framework.md` section 5 is the contract behind the wrappers.
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

mod error;
mod events;
mod gnoblin;
mod input;
mod owned;
mod process;
mod sys;
mod tree;
mod value;

#[doc(hidden)]
pub mod private;

/// Not public API: the names the macro expansion refers to.
#[doc(hidden)]
pub use private as __private;

pub use error::{CoreError, ErrorKind, Result};
pub use events::{NodeEvent, WindowRequest};
pub use gnoblin::{CoreStats, Gnoblin, LogLevel, ModuleInfo, Poster, ScopedSubscription, Subscription};
pub use gnoblin_macros::main;
pub use input::{ChordEvent, ChordState, DragPhase, GestureEvent, GestureKind, GrabScope, PointerButton, PointerDrag};
pub use owned::Value;
pub use process::{Command, Process, ProcessExit, Signal, Stream};
pub use tree::{Answer, Client, Node, PendingCall, Rect, Window};
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
