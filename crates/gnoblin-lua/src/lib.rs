//! Lua host: sandbox, loader, generated bindings, operations, timers and tweens.
//!
//! The runtime evaluates `init.lua`, keeps a mirror of the object graph, answers
//! property reads locally, turns method calls into `Operation`s, and runs the
//! handlers a signal, a completion or a timer wakes. It is an IPC client
//! (design/rust-compositor.md, D1): it links [`gnoblin_client`] and
//! [`gnoblin_config`] and no engine crate, so the compositor runs no Lua and a
//! fault in a script is a restart of this process rather than the end of the
//! session.
//!
//! ```no_run
//! use std::path::Path;
//!
//! use gnoblin_client::Server;
//! use gnoblin_config::SearchPaths;
//! use gnoblin_lua::{Host, Limits};
//! use gnoblin_model::{Graph, Tick};
//! use gnoblin_proto::Version;
//! use gnoblin_schema::Catalog;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let catalog = Catalog::load_dir(Path::new("crates/gnoblin-schema/schema"))?;
//! let version = Version::new(0, 2, 0);
//! let mut server = Server::new(Graph::new(catalog), version);
//! let client = server.connect(version)?;
//! let mut host = Host::new(client, Limits::default())?;
//!
//! let report = host.run_config(&SearchPaths::from_env())?;
//! if let Err(problems) = &report.config {
//!     eprintln!("{problems}");
//! }
//! for error in &report.errors {
//!     eprintln!("{error}");
//! }
//!
//! // The caller owns the clock: one pump per frame, at the tick it already has.
//! let pumped = host.pump(Tick::new(16))?;
//! eprintln!("{} handlers ran", pumped.handlers);
//! # Ok(())
//! # }
//! ```
//!
//! # The pieces
//!
//! - The sandbox (`gnoblin-lua-sandbox`): the Lua state, the library whitelist, the
//!   instruction budget, the memory limit and the per-entry deadline, sized by
//!   [`Limits`].
//! - [`Host::run_config`] and the `gnoblin.load` binding: the loader, confined to
//!   the configuration directory.
//! - `gnoblin.configure`: the neutral [`gnoblin_config::Value`] tree, validated by
//!   the configuration layer, so a problem arrives with its path.
//! - the core object types on `gnoblin` (`gnoblin.window`), and `require` for extension
//!   namespaces (`require("example.ext")`): the generated bindings, one name per schema
//!   member, with `gnoblin.d.lua` from the same schema.
//! - [`Host::pump`]: the only thing that runs a handler, on the caller's ticks.
//! - [`RoleRunner`]: the runtime role as a process — adopt the privileged connection, load and
//!   commit the configuration, then serve the handlers until the guardian stops it. It is
//!   what `gnoblin --role runtime` runs.
//!
//! # Failure modes first
//!
//! Each entry names the test that covers it, in `tests/failure_modes.rs` unless it
//! says otherwise.
//!
//! 1. **A script escapes the sandbox.** No `io`, `os`, `debug`, `package`,
//!    `dofile`, `loadfile` or `coroutine` exists; `require` answers only for the
//!    extension namespaces in the generated catalog, and a binary chunk is refused
//!    rather than run:
//!    `a_script_cannot_reach_the_filesystem_or_a_library_the_sandbox_withholds`.
//! 2. **An infinite loop hangs the host.** The instruction budget, and the
//!    wall-clock deadline a tick budget names, abort the entry; the host keeps its
//!    state and reports the failure:
//!    `an_infinite_loop_is_stopped_by_the_instruction_budget` and
//!    `a_handler_that_outlives_its_deadline_is_stopped`.
//! 3. **A script allocates without bound.** The memory limit refuses the
//!    allocation instead of growing the process:
//!    `a_script_that_exceeds_the_memory_limit_is_stopped`.
//! 4. **A handler error kills its subscribers.** A handler that raises is reported
//!    and the pump continues, so the next handler still runs:
//!    `an_erroring_handler_does_not_stop_the_other_handlers`.
//! 5. **A load glob escapes the configuration directory.** `..`, an absolute path,
//!    a file that links outside and a directory that links outside are all refused
//!    or not walked: `a_load_pattern_cannot_leave_the_configuration_directory`.
//! 6. **A method returns two completions.** One operation gets one callback:
//!    `an_operation_completes_its_callback_once`.
//! 7. **`gnoblin.d.lua` drifts from the schema.** `tests/bindings.rs` re-reads the
//!    schema and checks the stub and the installed tables against it member by
//!    member: `the_stub_matches_the_schema` and `the_installed_tables_match_the_schema`.
//! 8. **A failing handler floods the log.** A repeat at the same site is reported
//!    once: `a_repeated_error_is_reported_once_per_site`.
//! 9. **A timer fires early or after it was cancelled.** Timers run on the caller's
//!    tick and only while they exist:
//!    `a_timer_fires_on_its_tick_and_never_after_cancel`.
//!
//! # Boundaries of this slice
//!
//! The slice is real and tested, and it is not the whole API:
//!
//! - `gnoblin.configure` is available while the configuration loads. IPC 0.2.0
//!   declares no runtime configuration method yet, so a call at run time is
//!   refused with that reason rather than mutating a document nothing applies.
//! - `require` also answers for layout modules, read from the shipped and user layout
//!   roots by `layout::LayoutModuleLoader`, which holds one layout VM per configuration evaluation.
//!   `gnoblin.load` stays the one loader of configuration files.
//! - Requests (`RequestOffered` and `RequestFinished`) are dropped: prompts and
//!   input policy are later lanes, and an unanswered request already fails closed
//!   in the broker.
//! - An empty Lua table converts to an empty [`gnoblin_config::Value::Map`].
//!   Lua cannot distinguish `{}` from an empty list, so clearing a list-valued
//!   setting needs the list itself rather than `{}`.
//! - Lua 5.5 integers are signed 64-bit, so an unsigned property above
//!   `i64::MAX` cannot be read and raises rather than arriving wrapped.

mod bindings;
pub mod check;
mod collections;
mod configure;
mod derived;
mod error;
mod extension_bindings;
mod extensions;
mod glob;
mod host;
pub mod layout;
mod lifecycle;
mod loader;
#[cfg(test)]
#[path = "../lua_names.rs"]
mod lua_names;
mod process;
mod requests;
pub mod role;
mod session_api;
mod session_status;
pub mod tree_mirror;

/// The `gnoblin.d.lua` stub, and the writer that copies it out for an editor.
pub use bindings::{stub, write_stub};
/// The error a host returns when it cannot start, and the error a script failure is
/// reported as.
pub use error::{HostError, ScriptError};
/// The registry of loaded extension namespaces, and the reasons one is refused.
pub use extensions::{ExtensionRegistry, RegistryError};
/// How the runtime role reads its environment, re-exported from the client that reads a
/// descriptor out of one.
pub use gnoblin_client::Environment;
/// The host, the report of one pump and the report of one configuration run, and the
/// limit on nested loads.
pub use host::{Host, MAX_LOADS, PUMP_EVENT_BATCH, PumpReport, ReloadOutcome, RunReport};

/// The sandbox limits a host is sized with: the instruction budget, the memory limit and
/// the per-entry deadline.
pub use gnoblin_lua_sandbox::Limits;
/// The runtime role and the error it ends with.
pub use role::{RoleError, RoleRunner};

gnoblin_log::logger!(log = "lua");
