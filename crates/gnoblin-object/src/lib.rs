//! The object protocol of Gnoblin's libraries, written once.
//!
//! `libgnoblin-core` and `libgnoblin-std` serve classes, properties, methods and signals from a catalog through one small table
//! (`GnoblinObjectApi`, in `gnoblin-core-abi`), so a class added to the catalog is a catalog file and a Rust implementation and no
//! new function in a table. This crate is both ends of that table:
//!
//! - [`Val`] is the value that crosses, and [`wire`] converts it to and from the ABI's `GnoblinValue`;
//! - [`server`] gives a library its classes: a [`server::Builder`] to implement them, the object table of a generation, argument
//!   checks, references, listeners and the turn that tells them, and the `extern "C"` functions of the table;
//! - [`client`] is how an SDK or a script runtime calls a table: [`client::Api`] for the calls and [`client::Object`] for a reference
//!   that releases itself;
//! - [`catalog`] is the static form of a namespace's schema files that a library is built from (`gnoblin-sdkgen` writes it).
//!
//! `design/native-std-port.md` section 4.2 is the contract and `docs/abi-compatibility.md` states what may change.

pub mod catalog;
pub mod client;
mod error;
pub mod server;
pub mod typed;
mod value;
pub mod wire;

pub use error::{Fail, Res};
pub use value::{Callable, ClassId, Closure, MemberId, Obj, Timing, Val};
