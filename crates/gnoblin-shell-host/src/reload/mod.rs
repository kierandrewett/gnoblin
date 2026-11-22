//! Native hot reload: the protocol that replaces a running `.so` shell with a freshly built one, and the handles around it.
//!
//! - [`NativeShell`] is the shell thread's side: it owns the running generation and runs a reload in the one order that is
//!   correct. **Stage**: the library is copied to a private path and opened, and the new generation's `init` runs while the
//!   old generation still serves; whatever the new `init` registers, claims, spawns or writes is held back. **Commit**: the
//!   old generation's shutdown hooks run, the new generation takes over (the old claims are released and the new ones sent
//!   behind the releases in one transaction), and the old generation retires: its registrations are removed, every later
//!   call from its code is refused with `E_RETIRED`, and its library stays mapped on the bounded retire list. **Abandon**:
//!   any failure before the commit (no file, a file that is not a shell, a wrong ABI, a failing or panicking `init`, a
//!   full retire list) leaves the old generation exactly as it was and leaves nothing of the new one behind.
//! - [`ReloadController`] is what everything else holds: the compositor edge, `gnoblinctl shell reload`, the file watcher,
//!   the shell itself (`request_reload`). It is a cheap, thread-safe handle; asking never blocks and never runs a reload,
//!   because a reload runs on the shell thread between dispatches ([`NativeShell::serve_requests`]). It also carries what
//!   `gnoblinctl shell status` shows: the running generation, what the last reload did, and the last failed build.
//! - [`BuildStep`] is the optional developer loop around it: it runs `cargo build` for a shell project and reports a failure
//!   (the compiler's stderr) to the log and the controller without touching the running generation, and [`ModuleWatch`]
//!   turns a changed `.so` (or launcher file) into a request. The compositor never runs cargo.

mod build;
mod controller;
mod native_shell;
mod watch;

pub use build::{BuildFailure, BuildStep, Built};
pub use controller::{ReloadController, ReloadRecord, ReloadStatus, ReloadTrigger};
pub use native_shell::{NativeShell, ReloadError, ReloadFailure, Reloaded};
pub use watch::ModuleWatch;
