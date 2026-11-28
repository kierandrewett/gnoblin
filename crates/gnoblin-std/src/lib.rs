//! The Rust SDK for the optional Gnoblin desktop library.
//!
//! `gnoblin-core` is the machinery: the clients, outputs and input of the compositor. `gnoblin-std` is the desktop built on it:
//! [`Workspace`]s, [`Layout`]s, [`Transition`]s, styles, the scene, the window frame and the policies that share workspaces between
//! outputs, as classes with typed properties, methods and signals, the same classes the TypeScript and Python SDKs have. A shell that
//! wants none of it links the core alone and never loads the std library; a shell that wants it writes
//! `use gnoblin_std::prelude::*;`.
//!
//! This crate holds `extern` declarations of the installed `libgnoblin-std` and the typed classes of its object model, and never a
//! copy of the library: the link is to the library the user has installed, so a compatible update changes what a shell runs on without
//! recompiling it. The process then maps the chain shell -> `libgnoblin-std.so.1` -> `libgnoblin-core.so.1`, once each.
//! `gnoblin-std-abi` is the contract.
//!
//! ```ignore
//! // Cargo.toml: [lib] crate-type = ["cdylib"]   [dependencies] gnoblin-core = "1"  gnoblin-std = "1"
//! use gnoblin_std::prelude::*;
//!
//! #[gnoblin::main]
//! fn main(gnoblin: &Gnoblin) -> gnoblin_std::Result<()> {
//!     let std = gnoblin.objects()?.std()?;
//!     let workspace = Workspace::new(&std, "main")?;
//!     Workspaces::get(&std)?.add(&workspace)?;
//!     gnoblin.bind("Super+Return", || gnoblin.log("a terminal would start here"))?;
//!     Ok(())
//! }
//! ```
//!
//! ## What is where
//!
//! - The classes of the std model (`Workspace`, `Workspaces`, `MasterStackLayout`, `Transition`, `ClientStyle`, `Scene`, `Toast`, ...)
//!   are at the root of this crate and in [`objects`]. `gnoblin.objects()?.std()?` is the handle they are made from.
//! - The members the library adds to the core's classes (`gnoblin.workspaces()`, `client.workspace()`, `client.style()`) are the
//!   extension traits [`prelude`] brings in.
//! - [`Error`] and [`Result`]: the failure of a call of the library as a Rust type.
//!
//! ## Threads, panics and reloads
//!
//! As for the core SDK: everything happens on the shell thread, a panic in a callback is caught where the callback meets the library,
//! and a handle that outlives its generation fails with `Retired`. The library never calls a callback inside a call you made: a change
//! is announced from the next turn of the dispatch, with the other events of that turn.

mod api;
mod error;
pub mod objects;
mod sys;

pub use error::{Error, ErrorKind, Result};
pub use gnoblin_object::typed::{Listener, Octets};
pub use gnoblin_object::{Fail, Res, Timing, Val};
pub use objects::*;

/// The std ABI major version this SDK was built for. A library of another major refuses to load the shell's `open`.
pub const ABI_MAJOR: u32 = gnoblin_std_abi::ABI_MAJOR;

/// The std ABI minor version this SDK was built against.
pub const ABI_MINOR: u32 = gnoblin_std_abi::ABI_MINOR;

/// The ABI version of the installed std library, as `(major, minor, patch)`. Needs the library to be loaded, which it is inside a
/// std shell.
pub fn installed_abi() -> (u32, u32, u32) {
    api::Std::installed_abi()
}

/// The names a desktop shell wants in scope: `use gnoblin_std::prelude::*;`.
///
/// It re-exports the core SDK under the name `gnoblin`, so `#[gnoblin::main]` works with nothing else imported, the classes of the core
/// at its root (`Gnoblin`, `Client`, `Output`, ...), the classes of this library, and the members this library adds to the core's
/// classes: `gnoblin.workspaces()?`, `gnoblin.request_transition(..)`, `outputs.profiles()?`, `context.active_display()?`,
/// `clients.style()?`, `client.workspace()?`, `output.show(..)`. A shell that does not use this prelude has none of them.
///
/// Both libraries have a class named `Objects` (the handle the classes are made from) and a few more; the prelude brings both and
/// a use of such a name needs its path (`gnoblin_core::Objects`, `gnoblin_std::Objects`).
#[allow(ambiguous_glob_reexports)] // both libraries name a few classes alike; a use of one names its crate
pub mod prelude {
    pub use gnoblin_core as gnoblin;
    pub use gnoblin_core::*;

    pub use crate::objects::*;

    pub use crate::{Error, ErrorKind, Result};
}
