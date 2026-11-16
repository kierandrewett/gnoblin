//! The ABI's types, constants, callbacks and exports.
//!
//! Everything a shell and the core exchange is declared here and nowhere else. The prefix `Gnoblin` on a type and
//! `GNOBLIN_` on a constant is the C header's namespace; the Rust SDK renames them to idiomatic types.
//!
//! The rules every declaration follows (`design/modular-framework.md`, section 5.2):
//!
//! - No Rust type crosses the boundary. Strings are `(pointer, length)` UTF-8, never NUL terminated.
//! - Inputs are borrowed for the call; the core copies what it keeps.
//! - Each side frees what it allocated: a `GnoblinBuf` and an owned `GnoblinValue` go back through the core's
//!   `buf_free` and `value_free`; a `user` pointer goes back through the `drop_user` the shell registered with it.
//! - A struct a caller passes in starts with `size: u32`, set by the caller; the core reads only that many bytes and
//!   defaults the rest. A struct the core fills in starts with `size` too. This is what makes appending a field
//!   compatible in both directions.

use core::ffi::c_void;

abi_aliases! {
    /// A generation context: one load of one shell module. Made by the loader, passed to the shell's `init`, and the
    /// first argument of every table function. After the generation retires every call with it returns `E_RETIRED`.
    pub type GnoblinCtx = u64;
    /// A registration (a callback the shell handed the core). Owned by the context that made it.
    pub type GnoblinReg = u64;
    /// A node of the compositor's tree: the schema's node id. Zero is nil.
    pub type GnoblinNode = u64;
    /// A call whose answer arrives later through a callback.
    pub type GnoblinPending = u64;
    /// A process a shell started.
    pub type GnoblinProc = u64;
}

abi_callbacks! {
    /// Frees a `user` pointer the shell handed to the core. Called exactly once, on the shell thread, when the
    /// registration it belongs to ends, while the generation's code is still mapped.
    pub type GnoblinDropFn = unsafe extern "C" fn(user: *mut c_void);
    /// A shutdown hook. Runs when the generation retires and at exit, before its registrations are removed.
    pub type GnoblinShutdownFn = unsafe extern "C" fn(user: *mut c_void);
    /// Work a non-shell thread asked to run on the shell thread (`post`).
    pub type GnoblinPostFn = unsafe extern "C" fn(user: *mut c_void);
    /// A signal callback. `event` is valid until the callback returns. Returns `E_OK`, or `E_PANIC` when the SDK
    /// trampoline caught a panic.
    pub type GnoblinEventFn = unsafe extern "C" fn(user: *mut c_void, event: *const GnoblinEvent) -> i32;
    /// One catalog member, from `catalog_each`. `member` is valid until the callback returns.
    pub type GnoblinMemberFn = unsafe extern "C" fn(user: *mut c_void, member: *const GnoblinMember) -> i32;
}

abi_consts! {
    /// Success.
    pub const E_OK: i32 = 0;
    /// Failure; see `gnoblin_last_error`.
    pub const E_ERR: i32 = -1;
    /// A panic was caught at this boundary (either side).
    pub const E_PANIC: i32 = -2;
    /// The function or catalog member is not in this core (an older minor, or removed from the schema).
    pub const E_UNSUPPORTED: i32 = -3;
    /// Major mismatch or a malformed `size`.
    pub const E_ABI: i32 = -4;
    /// A module failed to load or initialise.
    pub const E_LOAD: i32 = -5;
    /// A handle that was removed, or never existed.
    pub const E_STALE: i32 = -6;
    /// The calling generation has been retired; the call did nothing.
    pub const E_RETIRED: i32 = -7;
    /// Node id not in the mirror.
    pub const E_NO_SUCH_NODE: i32 = -8;
    /// Class, property, method or signal not in the catalog, or not a member of that class.
    pub const E_NO_SUCH_MEMBER: i32 = -9;
    /// A value of the wrong kind for the member.
    pub const E_TYPE: i32 = -10;
    /// The compositor refused the request.
    pub const E_REFUSED: i32 = -11;
    /// A shell-thread-only function was called from another thread.
    pub const E_THREAD: i32 = -12;
    /// A single-holder resource is taken.
    pub const E_BUSY: i32 = -13;
    /// A deadline passed.
    pub const E_TIMEOUT: i32 = -14;

    /// `log` level: error.
    pub const LOG_ERROR: u32 = 1;
    /// `log` level: warning.
    pub const LOG_WARN: u32 = 2;
    /// `log` level: information.
    pub const LOG_INFO: u32 = 3;
    /// `log` level: debug.
    pub const LOG_DEBUG: u32 = 4;
    /// `log` level: trace.
    pub const LOG_TRACE: u32 = 5;

    /// `GnoblinValue.kind`: a boolean, in `bits` (0 or 1).
    pub const VALUE_BOOL: u32 = 1;
    /// `GnoblinValue.kind`: a signed 64-bit integer, in `bits` (two's complement).
    pub const VALUE_INT: u32 = 2;
    /// `GnoblinValue.kind`: an unsigned 64-bit integer, in `bits`.
    pub const VALUE_UINT: u32 = 3;
    /// `GnoblinValue.kind`: a double, in `bits` (IEEE 754 bit pattern).
    pub const VALUE_DOUBLE: u32 = 4;
    /// `GnoblinValue.kind`: UTF-8 text in `ptr` and `len`.
    pub const VALUE_TEXT: u32 = 5;
    /// `GnoblinValue.kind`: a node id, in `bits`.
    pub const VALUE_NODE: u32 = 6;
    /// `GnoblinValue.kind`: a list of values: `ptr` points at `len` `GnoblinValue`s.
    pub const VALUE_LIST: u32 = 7;
    /// `GnoblinValue.kind`: a list of strings: `ptr` points at `len` `GnoblinStr`s.
    pub const VALUE_TEXTS: u32 = 8;
    /// `GnoblinValue.kind`: a list of effect records (a later minor defines the record).
    pub const VALUE_EFFECTS: u32 = 9;
    /// `GnoblinValue.kind`: opaque bytes in `ptr` and `len`.
    pub const VALUE_BYTES: u32 = 10;
    /// `GnoblinValue.kind`: secret text in `ptr` and `len`. Never logged; the receiver copies it out.
    pub const VALUE_SECRET: u32 = 11;
    /// `GnoblinValue.kind`: a record value (a struct-like object reference); `ptr` points at `len` `GnoblinValue`s,
    /// the record's properties in schema order.
    pub const VALUE_RECORD: u32 = 12;

    /// `GnoblinValue.flags`: `ptr` was allocated by the core and is freed with `value_free`.
    pub const VALUE_OWNED: u32 = 1;

    /// A catalog member kind: a property.
    pub const MEMBER_PROPERTY: u32 = 1;
    /// A catalog member kind: a method.
    pub const MEMBER_METHOD: u32 = 2;
    /// A catalog member kind: a signal.
    pub const MEMBER_SIGNAL: u32 = 3;

    /// `GnoblinMember.flags`: a `write_set` may carry `seconds` to animate the property.
    pub const MEMBER_ANIMATED: u32 = 1;
    /// `GnoblinMember.flags`: the property cannot be set.
    pub const MEMBER_READONLY: u32 = 2;
    /// `GnoblinMember.flags`: the member's values are secret.
    pub const MEMBER_SECRET: u32 = 4;

    /// `GnoblinCtxInfo.language`: a native shell module.
    pub const LANG_NATIVE: u32 = 1;
    /// `GnoblinCtxInfo.language`: a Python shell.
    pub const LANG_PYTHON: u32 = 2;
    /// `GnoblinCtxInfo.language`: a JavaScript shell.
    pub const LANG_JS: u32 = 3;
    /// `GnoblinCtxInfo.language`: a TypeScript shell.
    pub const LANG_TS: u32 = 4;

    /// `GnoblinShellEntry.flags`: the shell runs its own event loop and drives `dispatch` itself.
    pub const SHELL_FLAG_FOREIGN_LOOP: u32 = 1;
}

abi_structs! {
    /// UTF-8 text borrowed for the duration of a call: a pointer and a byte length, not NUL terminated.
    pub struct GnoblinStr {
        /// First byte, or null when `len` is zero.
        pub ptr: *const u8,
        /// Length in bytes.
        pub len: usize,
    }

    /// A buffer the core allocated and the caller owns; free it with `buf_free`.
    pub struct GnoblinBuf {
        /// First byte.
        pub ptr: *mut u8,
        /// Length in bytes.
        pub len: usize,
        /// Capacity of the allocation, for the core's use.
        pub cap: usize,
    }

    /// A tagged value. The schema has no unions, so `kind` selects which fields are read.
    pub struct GnoblinValue {
        /// Size of this struct, set by whoever builds it.
        pub size: u32,
        /// One of the `VALUE_*` kinds. A kind a reader does not know is `E_UNSUPPORTED`.
        pub kind: u32,
        /// `VALUE_OWNED` when `ptr` belongs to the core.
        pub flags: u32,
        /// Zero.
        pub reserved: u32,
        /// Scalar payload (bool, int, uint, double bit pattern, node id).
        pub bits: u64,
        /// Pointer payload (text, bytes, secret, list, texts).
        pub ptr: *const c_void,
        /// Element or byte count for `ptr`.
        pub len: usize,
    }

    /// One signal delivery. Valid until the callback it is passed to returns.
    pub struct GnoblinEvent {
        /// Size of this struct, set by the core.
        pub size: u32,
        /// The class id of the node the signal came from.
        pub class: u32,
        /// The signal id within the class.
        pub signal: u32,
        /// Zero.
        pub reserved: u32,
        /// The node it came from, or zero for a signal that has no node.
        pub node: GnoblinNode,
        /// The signal's payload fields, in schema order.
        pub values: *const GnoblinValue,
        /// Number of payload fields.
        pub n_values: usize,
    }

    /// One member of a catalog class.
    pub struct GnoblinMember {
        /// Size of this struct, set by the core.
        pub size: u32,
        /// The member id within its class and kind.
        pub id: u32,
        /// `MEMBER_PROPERTY`, `MEMBER_METHOD` or `MEMBER_SIGNAL`.
        pub kind: u32,
        /// The value kind of a property, `VALUE_SECRET` for a secret one; zero for methods and signals.
        pub value_kind: u32,
        /// `MEMBER_*` flags.
        pub flags: u32,
        /// Number of arguments (a method's parameters, a signal's fields).
        pub arg_count: u32,
        /// `arg_count` value kinds. Static storage owned by the core, valid for the life of the process.
        pub arg_kinds: *const u32,
    }

    /// What the core knows about a generation context. Filled by `ctx_info`.
    pub struct GnoblinCtxInfo {
        /// Size of this struct, set by the caller.
        pub size: u32,
        /// One of the `LANG_*` constants.
        pub language: u32,
        /// The generation number of this load.
        pub generation: u64,
        /// How many times this shell has been reloaded before this load.
        pub reload_count: u32,
        /// Nonzero when the shell host runs in the isolated child process.
        pub isolate: u32,
        /// The path of the module as it was asked for. Valid for the life of the process.
        pub module_path: GnoblinStr,
        /// The directory a shell may keep state in across reloads; empty when there is none.
        pub state_dir: GnoblinStr,
    }

    /// Counters the core keeps. Filled by `stats`.
    pub struct GnoblinStats {
        /// Size of this struct, set by the caller.
        pub size: u32,
        /// Zero.
        pub reserved: u32,
        /// Live registrations across every generation.
        pub registrations: u64,
        /// Callbacks the core has run.
        pub callbacks: u64,
        /// Panics the core caught at the boundary, on either side.
        pub panics: u64,
        /// Generations retired so far.
        pub retired_generations: u64,
        /// Calls refused because the calling generation had retired.
        pub stale_calls: u64,
    }

    /// Counters a shell module keeps about itself, returned through `GnoblinShellEntry.stats`. The lifetime tests read
    /// them to prove a retired generation is never called again.
    pub struct GnoblinShellStats {
        /// Registrations the shell made.
        pub registered: u64,
        /// Callbacks of the shell that ran.
        pub calls: u64,
        /// `drop_user` calls of the shell that ran.
        pub drops: u64,
    }

    /// What a native shell module exports through its one symbol, `gnoshell_entry_v1`.
    pub struct GnoblinShellEntry {
        /// Size of this struct, set by the shell.
        pub size: u32,
        /// The ABI major the shell was built against.
        pub abi_major: u32,
        /// The ABI minor the shell was built against.
        pub abi_minor_built: u32,
        /// `SHELL_FLAG_*` flags.
        pub flags: u32,
        /// Runs once per generation, on the shell thread, with the context made for it. Returns `E_OK` or an error
        /// code; on failure everything it registered is removed.
        pub init: unsafe extern "C" fn(api: *const GnoblinApi, ctx: GnoblinCtx) -> i32,
        /// Runs when the generation retires, after its shutdown hooks and before its registrations are removed.
        pub shutdown: unsafe extern "C" fn(ctx: GnoblinCtx),
        /// Optional counters, for tests and `gnoblinctl shell status`.
        pub stats: Option<unsafe extern "C" fn(out: *mut GnoblinShellStats)>,
        /// A label for logs: not NUL terminated, valid for the life of the module.
        pub label: GnoblinStr,
    }

    /// The versioned function table. `size` is the byte size of the table the core filled in; a field is present when
    /// `offset_of(field) + size_of(field) <= size`. Fields are appended in later minors, never moved, retyped or
    /// removed within a major.
    pub struct GnoblinApi {
        /// Byte size of this table.
        pub size: u32,
        /// ABI major of the core.
        pub major: u32,
        /// ABI minor of the core.
        pub minor: u32,
        /// Zero.
        pub reserved: u32,

        /// The generation number, module path, language and reload count of `ctx`. Shell thread.
        pub ctx_info: unsafe extern "C" fn(ctx: GnoblinCtx, out: *mut GnoblinCtxInfo) -> i32,
        /// Writes one line to the core's log, tagged with the generation. Any thread.
        pub log: unsafe extern "C" fn(ctx: GnoblinCtx, level: u32, msg: *const u8, len: usize) -> i32,
        /// Reports a panic the shell caught, so the core logs it with the generation. Any thread.
        pub report_panic: unsafe extern "C" fn(ctx: GnoblinCtx, msg: *const u8, len: usize) -> i32,
        /// Sets the text `gnoblin_last_error` returns on this thread. A shell's `init` calls it to say why it is
        /// returning a failure, so the loader can show the reason. Any thread.
        pub set_last_error: unsafe extern "C" fn(ctx: GnoblinCtx, msg: *const u8, len: usize) -> i32,
        /// Registers a hook that runs when the generation retires and at exit. Shell thread. A registering function
        /// returns zero on failure; the core then has not adopted `user` and does not call `drop_user`.
        pub on_shutdown: unsafe extern "C" fn(ctx: GnoblinCtx, cb: GnoblinShutdownFn, user: *mut c_void, drop_user: Option<GnoblinDropFn>) -> GnoblinReg,
        /// Asks the host to reload this shell. Any thread.
        pub request_reload: unsafe extern "C" fn(ctx: GnoblinCtx) -> i32,
        /// Asks the host to exit the compositor with `code`. Any thread.
        pub request_exit: unsafe extern "C" fn(ctx: GnoblinCtx, code: i32) -> i32,

        /// Frees a buffer the core returned. Any thread.
        pub buf_free: unsafe extern "C" fn(buf: *mut GnoblinBuf) -> i32,
        /// Frees the owned pointer of a value the core returned. Any thread.
        pub value_free: unsafe extern "C" fn(value: *mut GnoblinValue) -> i32,

        /// Interns a class name (`window`, or `namespace.object`) and returns its id; zero when it is not in the
        /// catalog. Shell thread.
        pub catalog_class: unsafe extern "C" fn(ctx: GnoblinCtx, name: *const u8, len: usize) -> u32,
        /// Looks up a property, method or signal of a class by name. Shell thread.
        pub catalog_member: unsafe extern "C" fn(ctx: GnoblinCtx, class: u32, kind: u32, name: *const u8, len: usize, out: *mut GnoblinMember) -> i32,
        /// Enumerates the members of one kind of a class. Shell thread.
        pub catalog_each: unsafe extern "C" fn(ctx: GnoblinCtx, class: u32, kind: u32, cb: GnoblinMemberFn, user: *mut c_void) -> i32,

        /// Registers a callback for a signal of a class: for one node, or for every node of the class when `node` is
        /// zero. Shell thread. Zero on failure, with `user` not adopted.
        pub on_signal: unsafe extern "C" fn(ctx: GnoblinCtx, node: GnoblinNode, class: u32, signal: u32, cb: GnoblinEventFn, user: *mut c_void, drop_user: Option<GnoblinDropFn>) -> GnoblinReg,
        /// Removes a registration; `drop_user` runs now, or after the running callback returns. Shell thread.
        pub remove: unsafe extern "C" fn(reg: GnoblinReg) -> i32,
        /// Runs `f(user)` on the shell thread at its next `dispatch`. `drop_user` runs instead if the context retires
        /// first. Any thread.
        pub post: unsafe extern "C" fn(ctx: GnoblinCtx, f: GnoblinPostFn, user: *mut c_void, drop_user: Option<GnoblinDropFn>) -> i32,
        /// Runs queued work and delivers queued events, waiting up to `timeout_ms` for the first. Returns the number
        /// of callbacks run. Shell thread.
        pub dispatch: unsafe extern "C" fn(ctx: GnoblinCtx, timeout_ms: u32) -> i32,

        /// Fills the core's counters. Any thread.
        pub stats: unsafe extern "C" fn(ctx: GnoblinCtx, out: *mut GnoblinStats) -> i32,

        /// ABI 1.1 (test builds only): milliseconds since the core started.
        #[cfg(feature = "next-minor")]
        pub uptime_ms: unsafe extern "C" fn() -> u64,
    }
}

// SAFETY: a `GnoblinStr` is a pointer and a length. The type itself never dereferences the pointer; whoever builds one
// for a static (the SDK, from a string literal) guarantees the bytes live for the whole program. This is what lets a
// `GnoblinShellEntry`, which holds one, be a `static`.
#[allow(unsafe_code)]
unsafe impl Sync for GnoblinStr {}

impl GnoblinStr {
    /// The empty string: a null pointer and zero length.
    pub const EMPTY: GnoblinStr = GnoblinStr {
        ptr: core::ptr::null(),
        len: 0,
    };

    /// A string borrowing `text`. The caller keeps `text` alive for as long as the `GnoblinStr` is read.
    pub const fn borrowed(text: &str) -> GnoblinStr {
        GnoblinStr {
            ptr: text.as_ptr(),
            len: text.len(),
        }
    }
}

abi_exports! {
    /// `void gnoblin_abi_version(major, minor, patch)`: the ABI version of the installed core. Never fails.
    AbiVersionFn => gnoblin_abi_version = unsafe extern "C" fn(major: *mut u32, minor: *mut u32, patch: *mut u32);
    /// `int32_t gnoblin_get_api(want_major, built_minor, out)`: the function table. A different major returns `E_ABI`
    /// with the two versions in `gnoblin_last_error`; a newer `built_minor` than the core's still succeeds and the
    /// table's `size` says which functions exist.
    GetApiFn => gnoblin_get_api = unsafe extern "C" fn(want_major: u32, built_minor: u32, out: *mut *const GnoblinApi) -> i32;
    /// `size_t gnoblin_last_error(buf, cap)`: copies the text of the last failure on this thread, NUL terminated and
    /// truncated to `cap`, and returns its full length. Usable when `gnoblin_get_api` itself failed.
    LastErrorFn => gnoblin_last_error = unsafe extern "C" fn(buf: *mut u8, cap: usize) -> usize;
    /// `uint64_t gnoblin_catalog_hash(void)`: a 64-bit hash of the schema catalog the core was built with, a cheap
    /// compatibility probe for generated SDK constants.
    CatalogHashFn => gnoblin_catalog_hash = unsafe extern "C" fn() -> u64;
}
