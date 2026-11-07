//! The shared frame every compatibility adapter builds on.
//!
//! This crate is `gnoblin-compat-core` of `design/rust-compositor.md`
//! ("Compatibility crates", "Compatibility adapters" and D13). It holds what
//! every adapter shares and none of what any adapter maps:
//!
//! - [`SupportMatrix`]: one adapter's declared fidelity per interface and
//!   member, with a builder, a stable order and text and JSON renderings.
//! - [`Mode`] and [`allow`]: the `gnoblin.configure.compat.capture` gate over
//!   the four operation classes, the sandbox rule and the lock embargo.
//! - [`Adapter`] and [`Adapter::serve`]: the trait that turns one upstream name
//!   into the Gnoblin object protocol, and the only entry point a transport may
//!   use, which applies every refusal before an adapter sees a call.
//! - [`Refusal`] and [`Rule`]: the one error type, naming the adapter, the
//!   interface, the member, the operation and the rule that refused.
//! - [`DesktopSettings`], [`ColourScheme`] and [`Switch`]: the values D15
//!   publishes to other toolkits and the per-target switch that decides whether
//!   a target hears them. They live here because two compat crates publish the
//!   same values and a compat crate never depends on another.
//!
//! [`CallerWatcher`] is the one D-Bus helper: the caller-lifetime rule, that a watch or
//! session ends with the bus connection that asked for it. Nothing else here touches a
//! bus, a socket, a clock or a compositor: a call and a gate are plain data, so
//! [`Adapter::serve`] is a pure function of both and the same inputs always answer the same way.
//!
//! # What an adapter crate does
//!
//! 1. Check in `support-matrix.json` beside its `Cargo.toml`, in the form
//!    [`SupportMatrix::render_json`] writes, and load it with
//!    [`SupportMatrix::from_json`] at start-up. That file is both the reviewed
//!    declaration and what `cargo xtask compat-report` prints.
//! 2. Declare its calls as one type implementing [`Call`], one variant per
//!    member, each with the [`OperationClass`] the gate screens.
//! 3. Implement [`Adapter`]: [`Adapter::handle`] is the only mapping code.
//! 4. Call [`Adapter::serve`] from its transport, which owns the bus name.
//!
//! The frame deliberately does not decide: which bus names an adapter owns
//! (that is the transport), whether an adapter is built at all (a Cargo
//! feature), or per-adapter enable switches (`gnoblin.configure.compat`, which
//! [`Mode`] is only the shared default of). Input injection is also not refused
//! by the lock here: injected input travels the normal object-protocol path, and
//! `gnoblin-lock`'s input gate is what keeps it away from a covered client.
//!
//! # Failure modes first
//!
//! The plan's list for `compat/` plus this crate's own edges, each with the test
//! that covers it:
//!
//! 1. **An adapter call skips the lock embargo.** [`Adapter::serve`] refuses a
//!    capture while the session is locking or locked, and [`Adapter::handle`]
//!    never runs. `failure_modes::capture_is_refused_while_locked`,
//!    `capture_during_the_lock_transition_is_refused`, and the lock invariant in
//!    `tests/model.rs`.
//! 2. **A sandboxed caller reaches capture or input injection.** Both are
//!    refused in every mode, including `unmediated`.
//!    `a_sandboxed_caller_never_reaches_capture`,
//!    `a_sandboxed_caller_never_reaches_input_injection`.
//! 3. **A caller depends on an unsupported method and fails silently.** An
//!    unsupported member is refused with its declared reason and an undeclared
//!    member is refused outright, instead of being handed to the adapter.
//!    `an_unsupported_member_is_refused_with_its_reason`,
//!    `an_undeclared_member_is_refused`,
//!    `an_adapter_with_no_declared_members_refuses_everything`.
//! 4. **Two adapters disagree about output names.** Every refusal and every
//!    rendered report names the adapter and the interface, so two adapters that
//!    both serve `Start` cannot be confused for each other.
//!    `two_adapters_disagree_visibly`.
//! 5. **The default mode allows a write.** [`Mode::default`] is `portal`, so a
//!    state change is refused until a user enables it.
//!    `the_default_mode_refuses_writes`.
//! 6. **A manifest says something the frame cannot honour.** An unknown support
//!    word, an unknown key, a `partial` without a note, an `unsupported`
//!    without a reason, a repeated interface or a repeated member is an error,
//!    never a silent default: the manifest tests in `tests/failure_modes.rs`.
//!
//! `tests/model.rs` adds the invariants no single scenario can: over random
//! gates, calls and declarations, a call is served exactly when the mode, the
//! sandbox rule and the lock embargo allow it and its member is declared, and a
//! refused call never reaches [`Adapter::handle`].

#![forbid(unsafe_code)]

mod adapter;
mod caller;
mod matrix;
mod mode;
mod refusal;
mod settings;

/// The adapter type, its upstream handle and the gate a call passes through.
pub use adapter::{Adapter, Call, Gate, Upstream};
/// Watches the caller's identity so the gate can tell who is asking.
pub use caller::CallerWatcher;
/// Builds and reads the support matrix: which upstream members an adapter serves, and how faithfully.
pub use matrix::{Builder, Interface, InterfaceBuilder, Kind, MatrixError, Member, Support, SupportMatrix};
/// The mode gate: the operating mode, who is calling, and the allow or refuse decision.
pub use mode::{CallerContext, Decision, LockState, Mode, OperationClass, Sandbox, allow};
/// Why a call was refused, and the rule that refused it.
pub use refusal::{Refusal, Rule};
/// Desktop settings an adapter publishes, read from configuration or the environment.
pub use settings::{ColourScheme, DesktopSettings, Switch, settings_from_config, settings_from_environment};

gnoblin_log::logger!(log = "compat-core");
