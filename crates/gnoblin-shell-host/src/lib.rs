//! The language-neutral shell host.
//!
//! A shell is a generation: one load of one module, in whatever language. The host owns what is the same for every
//! language, and the language libraries own what is not:
//!
//! - [`generation`]: generation contexts, the registrations each one owns (callbacks, listeners, shutdown hooks, posted
//!   work), the event queue and the dispatch loop. A retiring generation's registrations are removed with it, each release
//!   runs exactly once, and every later call it makes is refused.
//! - [`native`] and [`retire`]: the loader of `.so` shells (unique copy, `dlopen`, entry check, `init`), and the bounded,
//!   visible list of the libraries it cannot unmap. One loader: the core library instantiates it over its function table.
//! - [`runtime`]: language dispatch: what a path is (a native shell, or a script and the runtime library that runs it by its
//!   extension) and the runtime libraries the loader keeps open (`gnoruntime_entry_v1`).
//! - [`stage`]: the stage, commit, abandon and swap protocol, with rollback marks and failure recovery that keeps the old
//!   generation. A language plugs its own stage and swap steps in through two traits.
//! - [`reload`]: the native reload protocol over the loader ([`reload::NativeShell`]: stage, commit, abandon), the
//!   [`reload::ReloadController`] the compositor edge, `gnoblinctl` and the watcher call, the build step and the module
//!   watch of the developer loop.
//! - [`supervisor`]: the strike policy that decides between restaging a faulted generation and giving up.
//! - [`watch`]: the debounced file watcher, with a caller-supplied test of which files matter.
//! - [`process`]: the programs a shell starts, owned by generation, adopted across a reload, ended with a report.
//! - [`guard`] and [`handle`]: panic containment with the per-thread last-error text, and handle encoding.
//!
//! Nothing here names QuickJS, Python or the compositor's tree. `gnoblin-js` consumes the supervisor, the watcher, the
//! process table and the reload protocol; `gnoblin-core-impl` consumes the generations, the loader and the guard.

gnoblin_log::logger!(pub(crate) log = "shell-host");

pub mod generation;
pub mod guard;
pub mod handle;
pub mod native;
pub mod process;
pub mod reload;
pub mod retire;
pub mod runtime;
pub mod stage;
pub mod supervisor;
pub mod watch;
