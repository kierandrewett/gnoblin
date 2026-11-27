//! The ABI's types, constants, callbacks and exports.
//!
//! Everything a shell and the core exchange is declared here and nowhere else. The prefix `Gnoblin` on a type and
//! `GNOBLIN_` on a constant is the C header's namespace; the Rust SDK renames them to idiomatic types.
//!
//! The rules every declaration follows:
//!
//! - No Rust type crosses the boundary. Strings are `(pointer, length)` UTF-8, never NUL terminated.
//! - Inputs are borrowed for the call; the core copies what it keeps.
//! - Each side frees what it allocated: a `GnoblinBuf` and an owned `GnoblinValue` go back through the core's
//!   `buf_free` and `value_free`; a `user` pointer goes back through the `drop_user` the shell registered with it.
//! - A struct a caller passes in starts with `size: u32`, set by the caller; the core reads only that many bytes and
//!   defaults the rest. A struct the core fills in starts with `size` too. This is what makes appending a field
//!   compatible in both directions.

use core::ffi::{c_char, c_int, c_void};

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
    /// An object of the object protocol (`GnoblinObjectApi`): a class instance one library holds for a shell. Made by `create`,
    /// `implement`, or handed back by a call; the low 32 bits name a slot and the high 32 bits the slot's generation, so a
    /// handle of an object that is gone is `E_STALE`, never another object. Zero is nil. A handle means something only to the
    /// library and the generation that issued it.
    pub type GnoblinObj = u64;
    /// A class of an object-protocol catalog (a handle class, a record, an interface, a service or an enum), by the id its library
    /// issues for the name. Stable for the life of the process and meaningful only against the library that issued it. Zero is nil.
    pub type GnoblinClassId = u32;
    /// A member of a class (a property, method, signal or enum value), by id within its class: the position in the class's member
    /// list, base class members first. Zero is nil. Ids of a base class's members are the same in every class derived from it.
    pub type GnoblinMemberId = u32;
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
    /// A change in the tree. `event` is valid until the callback returns. Returns `E_OK`, or `E_PANIC` when the SDK
    /// trampoline caught a panic.
    pub type GnoblinNodeFn = unsafe extern "C" fn(user: *mut c_void, event: *const GnoblinNodeEvent) -> i32;
    /// The answer to a call or a flush. `answer` is valid until the callback returns.
    pub type GnoblinAnswerFn = unsafe extern "C" fn(user: *mut c_void, answer: *const GnoblinAnswer) -> i32;
    /// One event of an input claim. `event` is valid until the callback returns.
    pub type GnoblinClaimFn = unsafe extern "C" fn(user: *mut c_void, event: *const GnoblinClaimEvent) -> i32;
    /// A timer fired.
    pub type GnoblinTimerFn = unsafe extern "C" fn(user: *mut c_void) -> i32;
    /// A program the shell started has ended. `exit` is valid until the callback returns.
    pub type GnoblinExitFn = unsafe extern "C" fn(user: *mut c_void, exit: *const GnoblinProcessExit) -> i32;
    /// A closure, an interface method or a signal listener of the object protocol. `member` is zero for a closure value, the
    /// member id of the method (or property read) for an object the shell implements, and the signal or property id for a listener.
    /// `args` are borrowed for the call. The callee writes its answer to `out`, which the caller has set to `VALUE_NONE`; the answer's
    /// memory belongs to the callee and the caller copies it before it makes another call into the callee. Returns `E_OK`, or an
    /// error code (`E_PANIC` when a trampoline caught a panic); on an error `out` is not an answer, but a callee may write its own words
    /// for the failure to it as `VALUE_TEXT`, which a caller reads when it is text and otherwise ignores (a callee that writes none leaves
    /// the `VALUE_NONE` the caller set). It runs on the shell thread, from the turn of the dispatch, never inside a call of the library
    /// that serves the object.
    pub type GnoblinInvokeFn = unsafe extern "C" fn(user: *mut c_void, member: GnoblinMemberId, args: *const GnoblinValue, n_args: usize, out: *mut GnoblinValue) -> i32;
    /// One entry of an object-protocol catalog, from `catalog_each`: a class, then its members, class by class in id order.
    /// `entry` is valid until the callback returns. A nonzero return stops the walk and is returned by `catalog_each`.
    pub type GnoblinCatalogFn = unsafe extern "C" fn(user: *mut c_void, entry: *const GnoblinCatalogEntry) -> i32;
    /// A read of a property, told to the tracker a library holds (`GnoblinObjectApi::track`): the object, its class and the member that
    /// was read. It is the library's own code calling the tracker's, on the shell thread, inside the `get` that made the read.
    pub type GnoblinTrackFn = unsafe extern "C" fn(user: *mut c_void, obj: GnoblinObj, class: GnoblinClassId, member: GnoblinMemberId);
    /// A write of a property of an object of the core's object model (`gnoblin.model`), offered to the write stage hook
    /// (`GnoblinApi::set_write_stage`) before the core sends it: the node the object stands for, the node's property key and the value, borrowed
    /// for the call. A nonzero return says the hook kept the write, and the core does not send it; zero lets the write go on as if no
    /// hook were installed. It is the core's own code calling the hook's, on the shell thread, inside the object call that made the write.
    pub type GnoblinWriteStageFn = unsafe extern "C" fn(user: *mut c_void, node: GnoblinNode, key: GnoblinStr, value: *const GnoblinValue) -> i32;
    /// A read of a property of an object of the core's object model, offered to the write stage hook before the core answers from
    /// its tree: the node and the property key. A nonzero return says the hook holds a value for it, which the callee has written to
    /// `out` and keeps valid until its next call; the core copies it and answers with it. Zero leaves `out` alone and the core
    /// answers from its tree.
    pub type GnoblinWriteStagePeekFn = unsafe extern "C" fn(user: *mut c_void, node: GnoblinNode, key: GnoblinStr, out: *mut GnoblinValue) -> i32;
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
    /// `GnoblinValue.kind`: a list of effects: `ptr` points at `len` `GnoblinEffect`s (a node's `effects`). Only a write carries it.
    pub const VALUE_EFFECTS: u32 = 9;
    /// `GnoblinEffect.kind`: a backdrop blur.
    pub const EFFECT_BLUR: u32 = 1;
    /// `GnoblinEffect.kind`: a drop shadow.
    pub const EFFECT_SHADOW: u32 = 2;
    /// `GnoblinEffect.kind`: a border stroke.
    pub const EFFECT_BORDER: u32 = 3;
    /// `GnoblinEffect.kind`: a user shader pass, written in WGSL (`source` and `params`).
    pub const EFFECT_SHADER: u32 = 4;
    /// `GnoblinEffect.kind`: a background colour under the node's content (`colour`).
    pub const EFFECT_BACKGROUND: u32 = 5;
    /// `GnoblinEffect.kind`: a blur and a grade of what the node paints (`blur_radius`, `saturation`, `brightness`, `contrast`).
    pub const EFFECT_FILTER: u32 = 6;
    /// `GnoblinValue.kind`: opaque bytes in `ptr` and `len`.
    pub const VALUE_BYTES: u32 = 10;
    /// `GnoblinValue.kind`: secret text in `ptr` and `len`. Never logged; the receiver copies it out.
    pub const VALUE_SECRET: u32 = 11;
    /// `GnoblinValue.kind`: a record value (a struct-like object reference); `ptr` points at `len` `GnoblinValue`s,
    /// the record's properties in schema order. In the object protocol `bits` is the record class's id (`GnoblinClassId`),
    /// and a field a record leaves out is `VALUE_NONE`.
    pub const VALUE_RECORD: u32 = 12;
    /// `GnoblinValue.kind`: an object of the object protocol: `bits` is the `GnoblinObj` and `len` the id of its class (zero when
    /// the sender does not say). Passing one hands over no ownership: a handle a call returns is a reference the receiver
    /// `release`s when it is done with it.
    pub const VALUE_OBJECT: u32 = 13;
    /// `GnoblinValue.kind`: a closure: `ptr` points at a `GnoblinClosure`. A library that keeps it adopts `user` and calls its
    /// `drop_user` exactly once, when it lets go; a call that fails has adopted none.
    pub const VALUE_CLOSURE: u32 = 14;
    /// `GnoblinValue.kind`: a member of an enum: `bits` is the member's id within the enum class (the first value is 1) and `len`
    /// the enum's `GnoblinClassId` when the sender says.
    pub const VALUE_ENUM: u32 = 15;
    /// `GnoblinValue.kind`: nothing: an optional value that is absent, or the result of a call that answers nothing.
    pub const VALUE_NONE: u32 = 16;
    /// `GnoblinValue.kind`: an object another library serves: `bits` is its `GnoblinObj` in that library and `ptr`/`len` that library's
    /// namespace (`gnoblin.model`) as UTF-8. The receiver reads it through the object table of that namespace and asks it for the
    /// class (`class_of`). Like `VALUE_OBJECT`, a value a call hands out carries one reference, which the receiver `release`s in the
    /// table of that namespace, not the table that delivered it.
    pub const VALUE_FOREIGN_OBJECT: u32 = 17;
    /// The kind of a declared type, never of a value: the member takes or gives any value a script holds (none, boolean, number, text,
    /// or a list of those). The server refuses a value that holds an object, a record, a closure or bytes; a value on the wire carries
    /// its own kind.
    pub const VALUE_ANY: u32 = 18;

    /// `GnoblinValue.flags`: `ptr` was allocated by the core and is freed with `value_free`. The object protocol never sets it:
    /// a value its library returns is lent until the next call into that library on the thread.
    pub const VALUE_OWNED: u32 = 1;

    /// A catalog member kind: a property.
    pub const MEMBER_PROPERTY: u32 = 1;
    /// A catalog member kind: a method.
    pub const MEMBER_METHOD: u32 = 2;
    /// A catalog member kind: a signal.
    pub const MEMBER_SIGNAL: u32 = 3;
    /// A catalog member kind: a value of an enum (the member id is the value, the first is 1).
    pub const MEMBER_VALUE: u32 = 4;

    /// `GnoblinMember.flags`: a `write_set` may carry `seconds` to animate the property.
    pub const MEMBER_ANIMATED: u32 = 1;
    /// `GnoblinMember.flags`: the property cannot be set.
    pub const MEMBER_READONLY: u32 = 2;
    /// `GnoblinMember.flags`: the member's values are secret.
    pub const MEMBER_SECRET: u32 = 4;
    /// `GnoblinMember.flags` and `GnoblinCatalogEntry.flags`: the method is a function of the class (no receiver).
    pub const MEMBER_STATIC: u32 = 8;
    /// `GnoblinCatalogEntry.flags`: the method of an interface must be implemented.
    pub const MEMBER_REQUIRED: u32 = 16;
    /// `GnoblinCatalogEntry.flags`: the method of an interface has a default the library runs when the object does not implement it.
    pub const MEMBER_NATIVE_DEFAULT: u32 = 32;
    /// `GnoblinCatalogEntry.flags`: the member is accepted and not applied (the catalog says why).
    pub const MEMBER_INERT: u32 = 64;
    /// `GnoblinCatalogEntry.flags`: the property holds a closure.
    pub const MEMBER_CALLBACK: u32 = 128;
    /// `GnoblinCatalogEntry.flags`: the method's result may be none (a lookup that finds nothing answers none; any other method that
    /// answers an object and returns none is a library fault).
    pub const MEMBER_OPTIONAL: u32 = 256;

    /// `GnoblinCatalogEntry.kind` of a class: an object with an identity, made by `create`.
    pub const CLASS_HANDLE: u32 = 1;
    /// `GnoblinCatalogEntry.kind` of a class: a value, crossing as `VALUE_RECORD`.
    pub const CLASS_RECORD: u32 = 2;
    /// `GnoblinCatalogEntry.kind` of a class: members the shell implements (`implement`) and the library calls.
    pub const CLASS_INTERFACE: u32 = 3;
    /// `GnoblinCatalogEntry.kind` of a class: one object per generation, made by the first `create`, and functions.
    pub const CLASS_SERVICE: u32 = 4;
    /// `GnoblinCatalogEntry.kind` of a class: a closed set of named values, crossing as `VALUE_ENUM`.
    pub const CLASS_ENUM: u32 = 5;
    /// `GnoblinCatalogEntry.flags` of a class: an object of it stays until `destroy` (or its parent's) even when the shell
    /// `release`s every reference, as a widget on screen does.
    pub const CLASS_FLAG_RETAINED: u32 = 1;
    /// `GnoblinCatalogEntry.entry`: the entry describes a class.
    pub const CATALOG_CLASS: u32 = 1;
    /// `GnoblinCatalogEntry.entry`: the entry describes a member of the class that the last class entry named.
    pub const CATALOG_MEMBER: u32 = 2;

    /// `GnoblinCtxInfo.language`: a native shell module.
    pub const LANG_NATIVE: u32 = 1;
    /// `GnoblinCtxInfo.language`: a Python shell.
    pub const LANG_PYTHON: u32 = 2;
    /// `GnoblinCtxInfo.language`: a JavaScript shell.
    pub const LANG_JS: u32 = 3;
    /// `GnoblinCtxInfo.language`: a TypeScript shell.
    pub const LANG_TS: u32 = 4;
    /// `GnoblinCtxInfo.language`: a script of a language no other constant names, run by a runtime library that handles its
    /// extension (`libgnoblin-runtime-<extension>.so`).
    pub const LANG_OTHER: u32 = 5;

    /// `GnoblinShellEntry.flags`: the shell runs its own event loop and drives `dispatch` itself.
    pub const SHELL_FLAG_FOREIGN_LOOP: u32 = 1;

    /// `tree_attach` mode: take the tree as the shell, the only one that may change it.
    pub const ATTACH_HOLDER: u32 = 1;
    /// `tree_attach` mode: read the tree and its changes.
    pub const ATTACH_OBSERVER: u32 = 2;

    /// `GnoblinNodeEvent.kind`: a node came into the tree.
    pub const NODE_CREATED: u32 = 1;
    /// `GnoblinNodeEvent.kind`: a node and its subtree left the tree.
    pub const NODE_DESTROYED: u32 = 2;
    /// `GnoblinNodeEvent.kind`: a property of a node changed.
    pub const NODE_CHANGED: u32 = 3;

    /// `GnoblinClaim.kind`: a key chord, or a pointer chord when it ends in `Button1` to `Button9`.
    pub const CLAIM_CHORD: u32 = 1;
    /// `GnoblinClaim.kind`: a touchpad gesture of `subject` kind with `fingers` contacts.
    pub const CLAIM_GESTURE: u32 = 2;
    /// `GnoblinClaim.kind`: every key of the devices named by `device`, which then fire only the chords claimed for them.
    pub const CLAIM_DEVICE: u32 = 3;
    /// `GnoblinClaim.kind`: every pointer button of the pointers named by `device`.
    pub const CLAIM_POINTER: u32 = 4;
    /// `GnoblinClaim.kind`: the seat's pointer, keyboard or both (`subject`) for the shell node `node`.
    pub const CLAIM_GRAB: u32 = 5;

    /// `GnoblinClaim.flags`: a chord claim withholds the key from clients and the seat's key state ("grab" in the schema).
    pub const CLAIM_FLAG_GRAB: u32 = 1;

    /// `GnoblinClaimEvent.kind`: a claimed chord changed state.
    pub const CLAIM_EVENT_CHORD: u32 = 1;
    /// `GnoblinClaimEvent.kind`: a claimed gesture advanced.
    pub const CLAIM_EVENT_GESTURE: u32 = 2;
    /// `GnoblinClaimEvent.kind`: a claimed pointer chord's drag advanced.
    pub const CLAIM_EVENT_POINTER_CHORD: u32 = 3;
    /// `GnoblinClaimEvent.kind`: a button of a claimed pointer went down or up.
    pub const CLAIM_EVENT_POINTER_BUTTON: u32 = 4;

    /// `GnoblinSpawn` stream mode: write to the compositor's own output, which the session log records.
    pub const STREAM_INHERIT: u32 = 0;
    /// `GnoblinSpawn` stream mode: drop the output.
    pub const STREAM_DISCARD: u32 = 1;
    /// `GnoblinSpawn` stream mode: keep the output, up to `output_limit` bytes, for the exit callback.
    pub const STREAM_CAPTURE: u32 = 2;

    /// `provider_register` kind: the window manager that places and focuses windows while no shell holds the tree
    /// (`GnoblinWmVtable`). At most one is live.
    pub const PROVIDER_WM: u32 = 1;
    /// `provider_register` kind: the UI that shows portal dialogs (`GnoblinUiVtable`). At most one is live.
    pub const PROVIDER_UI: u32 = 2;
    /// `provider_register` kind: an observer of the accepted configuration document (`GnoblinConfigVtable`). At most one is live.
    pub const PROVIDER_CONFIG: u32 = 3;

    /// `GnoblinWmWindow.kind` and `GnoblinWmOpen.kind`: an application toplevel.
    pub const WM_KIND_NORMAL: u32 = 0;
    /// `GnoblinWmWindow.kind` and `GnoblinWmOpen.kind`: a dialog with a parent.
    pub const WM_KIND_DIALOG: u32 = 1;
    /// `GnoblinWmWindow.kind` and `GnoblinWmOpen.kind`: a dialog that blocks its parent.
    pub const WM_KIND_MODAL: u32 = 2;

    /// `GnoblinWmWindow.flags`: the window fills its output's work area.
    pub const WM_WINDOW_MAXIMISED: u32 = 1;
    /// `GnoblinWmWindow.flags`: the window covers its whole output.
    pub const WM_WINDOW_FULLSCREEN: u32 = 2;
    /// `GnoblinWmWindow.flags`: the window is hidden and out of the focus rotation.
    pub const WM_WINDOW_MINIMISED: u32 = 4;
    /// `GnoblinWmWindow.flags`: `output` names the output the window is on.
    pub const WM_WINDOW_HAS_OUTPUT: u32 = 8;
    /// `GnoblinWmWindow.flags` and `GnoblinWmOpen.flags`: `transient_for` names the window's parent.
    pub const WM_WINDOW_HAS_PARENT: u32 = 16;
    /// `GnoblinWmOpen.flags`: `requested` is the rectangle the client asked for.
    pub const WM_OPEN_HAS_REQUESTED: u32 = 32;

    /// `GnoblinWmStanding.flags`: `focus` names the focused window.
    pub const WM_STANDING_HAS_FOCUS: u32 = 1;

    /// `GnoblinWmEffect.kind`: place the window at `geometry` and tell its client `flags` (the `WM_WINDOW_*` state bits).
    pub const WM_EFFECT_CONFIGURE: u32 = 1;
    /// `GnoblinWmEffect.kind`: show the window (`flags` nonzero) or hide it.
    pub const WM_EFFECT_VISIBLE: u32 = 2;
    /// `GnoblinWmEffect.kind`: the paint order, topmost first, in `ids`.
    pub const WM_EFFECT_STACK: u32 = 3;
    /// `GnoblinWmEffect.kind`: move keyboard focus to `window` (`flags` nonzero) or to none.
    pub const WM_EFFECT_FOCUS: u32 = 4;

    /// A window manager call's result: the provider refused, because the window is already open.
    pub const WM_REFUSED_DUPLICATE: i32 = 1;
    /// A window manager call's result: the record is full.
    pub const WM_REFUSED_TOO_MANY: i32 = 2;
    /// A window manager call's result: no output is connected to place the window on.
    pub const WM_REFUSED_NO_OUTPUT: i32 = 3;
    /// A window manager call's result: the window is not open.
    pub const WM_REFUSED_GONE: i32 = 4;
    /// A window manager call's result: the window is maximised or fullscreen, so it has no floating rectangle.
    pub const WM_REFUSED_NOT_FLOATING: i32 = 5;
    /// A window manager call's result: the output is not connected.
    pub const WM_REFUSED_UNKNOWN_OUTPUT: i32 = 6;
    /// A window manager call's result: a transient moves with its parent, not on its own.
    pub const WM_REFUSED_FOLLOWS_PARENT: i32 = 7;

    /// `may_take_focus` policy: a new window may take focus.
    pub const WM_FOCUS_ALLOW: u32 = 0;
    /// `may_take_focus` policy: a new window takes focus only when nothing is focused or it is a transient of the focused one.
    pub const WM_FOCUS_PREVENT: u32 = 1;

    /// `GnoblinUiVtable.set_scheme`: egui's stock dark visuals.
    pub const UI_SCHEME_DARK: u32 = 0;
    /// `GnoblinUiVtable.set_scheme`: egui's stock light visuals.
    pub const UI_SCHEME_LIGHT: u32 = 1;
    /// `GnoblinUiVtable.submit` result: the dialog is open now.
    pub const UI_STARTED: i32 = 0;
    /// `GnoblinUiVtable.submit` result: admitted, waiting behind the open dialog.
    pub const UI_QUEUED: i32 = 1;
    /// `GnoblinUiVtable.submit` result: refused, with the portal error's number (`gnoblin_portal_core::wire::error_code`)
    /// added to this value.
    pub const UI_REFUSED: i32 = 16;

    /// The most bytes `state_put` keeps across all keys: 16 MiB.
    pub const STATE_MAX_BYTES: u64 = 16 * 1024 * 1024;

    /// `node_adopt` result: the node was made by an earlier generation and is the shell's again. A success, like `E_OK`.
    pub const ADOPTED: i32 = 1;
    /// `node_adopt` flag: the node dies with its generation unless the next one adopts it during its `init`.
    pub const NODE_FLAG_EPHEMERAL: u32 = 1;
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

    /// One effect of a node's `effects` list, the `effect` record of the schema flattened: a parameter the kind does not use is
    /// ignored. Text fields are borrowed for the call. A write of `effects` passes a `GnoblinValue` of kind `VALUE_EFFECTS` whose
    /// `ptr` is an array of these; the list replaces the node's whole list. The array's stride is the `size` of its first record,
    /// and every record carries the same `size`: this struct grows by appended fields, so a shell built before a field existed
    /// passes shorter records than the core's struct, and the core reads each field the record reaches and takes the rest as
    /// zero. A shell sets `size` to the size of the struct it was built with and indexes its own array with `sizeof`; it never
    /// pads to a size it was not built with.
    pub struct GnoblinEffect {
        /// Size of this struct, set by the caller.
        pub size: u32,
        /// `EFFECT_BLUR`, `EFFECT_SHADOW`, `EFFECT_BORDER`, `EFFECT_SHADER`, `EFFECT_BACKGROUND` or `EFFECT_FILTER`.
        pub kind: u32,
        /// Blur: the backdrop is sampled at `1 / downscale` of full resolution.
        pub downscale: u32,
        /// Blur: the number of passes.
        pub passes: u32,
        /// Border: the stroke lies inside the silhouette when nonzero. Shadow: the shadow lies inside the node's shape.
        pub inset: u32,
        /// Shadow: the shadow follows the whole frame (titlebar included) when nonzero.
        pub frame: u32,
        /// Blur: grain from 0 to 1.
        pub noise: f64,
        /// Blur and filter: brightness multiplier, 1 leaves it.
        pub brightness: f64,
        /// Blur and filter: contrast multiplier, 1 leaves it.
        pub contrast: f64,
        /// Blur and filter: saturation multiplier, 1 leaves it.
        pub saturation: f64,
        /// Shadow: horizontal offset in logical pixels.
        pub offset_x: f64,
        /// Shadow: vertical offset in logical pixels.
        pub offset_y: f64,
        /// Shadow: blur radius in logical pixels. Filter: the blur of what the node paints, 0 for none.
        pub blur_radius: f64,
        /// Shadow: how far the shadow grows before blurring.
        pub spread: f64,
        /// Border: stroke width in logical pixels.
        pub width: f64,
        /// Border: distance of the stroke from the silhouette's edge.
        pub offset: f64,
        /// Blur: the kernel name (`kawase`); empty reads as the compositor's default.
        pub algorithm: GnoblinStr,
        /// Blur: the tint as `#rrggbb` or `#rrggbbaa`; empty is none.
        pub tint: GnoblinStr,
        /// Shadow, border and background: the colour as `#rrggbb` or `#rrggbbaa`.
        pub colour: GnoblinStr,
        /// Shader: the WGSL source, at most 16 KiB; empty for any other kind.
        pub source: GnoblinStr,
        /// Shader: the numbers the source reads as `p[0]` to `p[7]`, `n_params` of them; null when `n_params` is zero.
        pub params: *const f64,
        /// Shader: how many numbers `params` points at, at most 8.
        pub n_params: usize,
    }

    /// How a timed write plays: the argument of `write_set_timed`. A struct that starts with `size` may grow.
    pub struct GnoblinTiming {
        /// Size of this struct, set by the caller.
        pub size: u32,
        /// Zero.
        pub reserved: u32,
        /// The duration in seconds, above zero.
        pub seconds: f64,
        /// Seconds the compositor holds the start value before the animation begins; zero is none, at most 3600.
        pub delay: f64,
        /// The `tween` node whose `progress`, `paused` and `reverse` drive the track, or zero for none.
        pub tween: GnoblinNode,
        /// The curve: `n_samples` (at least two) progress values from 0 to 1, each in `[0, 1]`, spread evenly over the duration.
        pub samples: *const f64,
        /// Number of samples.
        pub n_samples: usize,
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
        /// The names of the `arg_count` arguments, in order, separated by single spaces: a method's parameter names and a
        /// signal's field names, as the schema spells them. Empty for a property. Static storage owned by the core, valid for
        /// the life of the process. A caller built before this field existed passes a shorter `size` and does not see it.
        pub arg_names: GnoblinStr,
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

    /// An array of node ids the core lends for the call: `ptr` points at `len` ids, valid until the next ABI call on the
    /// calling thread.
    pub struct GnoblinNodes {
        /// First id, or null when `len` is zero.
        pub ptr: *const GnoblinNode,
        /// Number of ids.
        pub len: usize,
    }

    /// A change in the tree, delivered to `on_node_created`, `on_node_destroyed` and `on_change` callbacks. Valid until the
    /// callback returns.
    pub struct GnoblinNodeEvent {
        /// Size of this struct, set by the core.
        pub size: u32,
        /// `NODE_CREATED`, `NODE_DESTROYED` or `NODE_CHANGED`.
        pub kind: u32,
        /// The catalog class id of the node, zero when the class is not in the catalog.
        pub class: u32,
        /// For `NODE_CHANGED`: the changed property's id in the node's class, zero when the catalog does not know it.
        pub property: u32,
        /// The node. For `NODE_DESTROYED` it is already gone from reads.
        pub node: GnoblinNode,
        /// The node's parent when the event is `NODE_CREATED` and the batch parented it, else zero.
        pub parent: GnoblinNode,
        /// For `NODE_CHANGED`: the new value, null otherwise.
        pub value: *const GnoblinValue,
    }

    /// The compositor's answer to a call or a flush. Valid until the callback returns.
    pub struct GnoblinAnswer {
        /// Size of this struct, set by the core.
        pub size: u32,
        /// `E_OK`, or `E_REFUSED` / `E_ERR` with the compositor's words in `error`.
        pub code: i32,
        /// The answer's bytes (text for most calls). For a flush: the serial the transaction was applied at, in decimal.
        pub result: GnoblinStr,
        /// The failure, empty on success.
        pub error: GnoblinStr,
    }

    /// What an input claim asks for: the argument of `claim`. A struct that starts with `size` may grow.
    pub struct GnoblinClaim {
        /// Size of this struct, set by the caller.
        pub size: u32,
        /// `CLAIM_CHORD`, `CLAIM_GESTURE`, `CLAIM_DEVICE`, `CLAIM_POINTER` or `CLAIM_GRAB`.
        pub kind: u32,
        /// `CLAIM_FLAG_*` flags.
        pub flags: u32,
        /// Zero.
        pub reserved: u32,
        /// A gesture's finger count (2 to 5).
        pub fingers: i64,
        /// The chord (`Super+Return`, or one ending in `Button1` for a pointer chord), the gesture kind (`swipe` or
        /// `pinch`), or a grab's scope (`pointer`, `keyboard` or `both`). Empty for a device or pointer claim.
        pub subject: GnoblinStr,
        /// The device a claim is scoped to (a name from the seat's `device`), empty for every device. Required for
        /// `CLAIM_DEVICE` and `CLAIM_POINTER`.
        pub device: GnoblinStr,
        /// A grab's node: the shell node that hears the input.
        pub node: GnoblinNode,
    }

    /// One event of an input claim. Valid until the callback returns; fields a kind does not use are zero or empty.
    pub struct GnoblinClaimEvent {
        /// Size of this struct, set by the core.
        pub size: u32,
        /// `CLAIM_EVENT_CHORD`, `CLAIM_EVENT_GESTURE`, `CLAIM_EVENT_POINTER_CHORD` or `CLAIM_EVENT_POINTER_BUTTON`.
        pub kind: u32,
        /// The claim's id, as the shell allocated it.
        pub claim: i64,
        /// A chord's `press`, `release` or `repeat`; a gesture's or drag's phase (`start`, `update`, `end`, `cancel`, or
        /// a gesture's own); a button's `press` or `release`.
        pub state: GnoblinStr,
        /// A chord's text, a gesture's kind (`swipe` or `pinch`), a pointer chord's modifiers.
        pub text: GnoblinStr,
        /// A gesture's finger count.
        pub fingers: i64,
        /// A pointer chord's button number or a pointer button's evdev code.
        pub button: i64,
        /// The pointer's global x for a pointer chord or button.
        pub x: f64,
        /// The pointer's global y.
        pub y: f64,
        /// Movement since the start (a pointer drag) or of the gesture.
        pub dx: f64,
        /// Movement since the start (a pointer drag) or of the gesture.
        pub dy: f64,
        /// A pinch's scale.
        pub scale: f64,
        /// A pinch's rotation.
        pub rotation: f64,
        /// The input event's time in milliseconds.
        pub time: i64,
    }

    /// How a program is started: the argument of `spawn`. A struct that starts with `size` may grow.
    pub struct GnoblinSpawn {
        /// Size of this struct, set by the caller.
        pub size: u32,
        /// What to do with standard output: `STREAM_INHERIT`, `STREAM_DISCARD` or `STREAM_CAPTURE`.
        pub stdout: u32,
        /// What to do with standard error.
        pub stderr: u32,
        /// Zero.
        pub reserved: u32,
        /// The working directory, empty for the shell's own.
        pub cwd: GnoblinStr,
        /// `n_env` strings of the form `NAME=value`, added to the session's environment.
        pub env: *const GnoblinStr,
        /// Number of `env` strings.
        pub n_env: usize,
        /// Text written to standard input, then closed; a null `ptr` means no input.
        pub stdin: GnoblinStr,
        /// The most bytes of captured output kept, zero for the default.
        pub output_limit: u64,
        /// Milliseconds before the process group is terminated, zero for never.
        pub timeout_ms: f64,
    }

    /// How a program ended: the argument of the `on_exit` callback. Valid until the callback returns.
    pub struct GnoblinProcessExit {
        /// Size of this struct, set by the core.
        pub size: u32,
        /// Nonzero when `code` is the exit status of a normal exit.
        pub has_code: u32,
        /// The exit status after a normal exit.
        pub code: i32,
        /// Nonzero when the `timeout_ms` option ended it.
        pub timed_out: u32,
        /// The process.
        pub process: GnoblinProc,
        /// The name of the signal that ended it, empty after a normal exit.
        pub signal: GnoblinStr,
        /// What a captured standard output held.
        pub stdout: GnoblinStr,
        /// What a captured standard error held.
        pub stderr: GnoblinStr,
        /// Bit 0: standard output passed `output_limit`. Bit 1: standard error did.
        pub truncated: u32,
        /// Zero.
        pub reserved: u32,
    }

    /// A rectangle in global logical pixels, as `gnoblin-types` has it.
    pub struct GnoblinWmRect {
        /// Left edge.
        pub x: i32,
        /// Top edge.
        pub y: i32,
        /// Horizontal extent.
        pub width: u32,
        /// Vertical extent.
        pub height: u32,
    }

    /// The insets layer surfaces reserve along the edges of an output.
    pub struct GnoblinWmInsets {
        /// Reserved above the work area.
        pub top: i32,
        /// Reserved to the right of the work area.
        pub right: i32,
        /// Reserved below the work area.
        pub bottom: i32,
        /// Reserved to the left of the work area.
        pub left: i32,
    }

    /// One window a window manager provider holds. Filled by the provider for `windows`, passed by the core for nothing else.
    pub struct GnoblinWmWindow {
        /// Size of this struct, set by whoever fills it.
        pub size: u32,
        /// `WM_KIND_NORMAL`, `WM_KIND_DIALOG` or `WM_KIND_MODAL`.
        pub kind: u32,
        /// `WM_WINDOW_*` flags: the state bits, and whether `output` and `transient_for` are set.
        pub flags: u32,
        /// Zero.
        pub reserved: u32,
        /// The compositor's identity for the window.
        pub id: u64,
        /// The window it is a transient of, when `WM_WINDOW_HAS_PARENT` is set.
        pub transient_for: u64,
        /// The output it is on, when `WM_WINDOW_HAS_OUTPUT` is set.
        pub output: u64,
        /// Its frame rectangle in global logical coordinates.
        pub geometry: GnoblinWmRect,
        /// The floating rectangle it returns to when it leaves maximise or fullscreen.
        pub restore: GnoblinWmRect,
    }

    /// One output a window manager provider holds. Filled by the provider for `outputs`.
    pub struct GnoblinWmOutput {
        /// Size of this struct, set by whoever fills it.
        pub size: u32,
        /// Zero.
        pub reserved: u32,
        /// The output's identity, as the backend numbers it.
        pub id: u64,
        /// The whole monitor rectangle.
        pub rect: GnoblinWmRect,
        /// What layer surfaces reserved along its edges.
        pub reserved_insets: GnoblinWmInsets,
    }

    /// The paint order and the focus a decision starts from, which the scene and the keyboard own. Borrowed for the call.
    pub struct GnoblinWmStanding {
        /// Size of this struct, set by the core.
        pub size: u32,
        /// `WM_STANDING_HAS_FOCUS` when `focus` names a window.
        pub flags: u32,
        /// The focused window.
        pub focus: u64,
        /// Every window drawn, topmost first.
        pub stack: *const u64,
        /// Number of ids in `stack`.
        pub n_stack: usize,
    }

    /// A window the compositor opens. Borrowed for the call.
    pub struct GnoblinWmOpen {
        /// Size of this struct, set by the core.
        pub size: u32,
        /// `WM_KIND_*`.
        pub kind: u32,
        /// `WM_WINDOW_HAS_PARENT` and `WM_OPEN_HAS_REQUESTED`.
        pub flags: u32,
        /// Zero.
        pub reserved: u32,
        /// The window's identity.
        pub id: u64,
        /// The window it is a transient of, when `WM_WINDOW_HAS_PARENT` is set.
        pub transient_for: u64,
        /// The rectangle the client asked for, when `WM_OPEN_HAS_REQUESTED` is set; only its size is used.
        pub requested: GnoblinWmRect,
    }

    /// One thing a provider's decision asks the compositor to do. Borrowed for the call of the sink it is passed to.
    pub struct GnoblinWmEffect {
        /// Size of this struct, set by the provider.
        pub size: u32,
        /// `WM_EFFECT_*`.
        pub kind: u32,
        /// A configure's `WM_WINDOW_*` state bits; a visibility's or a focus's flag.
        pub flags: u32,
        /// Zero.
        pub reserved: u32,
        /// The window a configure, a visibility or a focus is about.
        pub window: u64,
        /// A configure's rectangle.
        pub geometry: GnoblinWmRect,
        /// A stack's window ids, topmost first.
        pub ids: *const u64,
        /// Number of ids.
        pub n_ids: usize,
    }

    /// Where a provider puts the effects of a decision: it calls `push` once per effect, during the call it was passed to.
    pub struct GnoblinWmSink {
        /// Size of this struct, set by the core.
        pub size: u32,
        /// Zero.
        pub reserved: u32,
        /// The core's own pointer, handed back to `push`.
        pub sink: *mut c_void,
        /// Takes one effect; returns `E_OK`. The effect is copied before `push` returns.
        pub push: unsafe extern "C" fn(sink: *mut c_void, effect: *const GnoblinWmEffect) -> i32,
    }

    /// The window manager a provider registers with `provider_register(PROVIDER_WM)`: the decisions the compositor asks while
    /// no shell holds the tree. Every function runs on the compositor's thread, once per window event and never per frame,
    /// and is wrapped in `catch_unwind` by the provider, which returns `E_PANIC` after a panic. A mutating function returns
    /// `E_OK`, a `WM_REFUSED_*` refusal (nothing changed) or a negative error; an effect that does not decode or a negative
    /// result takes the provider out of the session: the core logs it once and runs `NullWm` from then on. `user` is the
    /// pointer the provider registered; the core calls `drop_user` on it exactly once when the provider ends, from the
    /// compositor's thread while the provider's code is still mapped.
    pub struct GnoblinWmVtable {
        /// Size of this struct, set by the provider.
        pub size: u32,
        /// Zero.
        pub reserved: u32,
        /// Writes up to `cap` of the windows the provider holds, by identity, to `out`, and returns how many it holds.
        pub windows: unsafe extern "C" fn(user: *mut c_void, out: *mut GnoblinWmWindow, cap: usize) -> usize,
        /// Writes up to `cap` of the outputs the provider holds to `out`, and returns how many it holds.
        pub outputs: unsafe extern "C" fn(user: *mut c_void, out: *mut GnoblinWmOutput, cap: usize) -> usize,
        /// Applies a window's role: its kind and parent.
        pub set_role: unsafe extern "C" fn(user: *mut c_void, id: u64, kind: u32, flags: u32, transient_for: u64, standing: *const GnoblinWmStanding, sink: *const GnoblinWmSink) -> i32,
        /// Whether a window that has just mapped may take focus under `policy` (`WM_FOCUS_*`): 1 yes, 0 no, or a negative error.
        pub may_take_focus: unsafe extern "C" fn(user: *mut c_void, id: u64, policy: u32, standing: *const GnoblinWmStanding) -> i32,
        /// Opens a window and places it.
        pub open: unsafe extern "C" fn(user: *mut c_void, open: *const GnoblinWmOpen, standing: *const GnoblinWmStanding, sink: *const GnoblinWmSink) -> i32,
        /// Forgets a closed window.
        pub close: unsafe extern "C" fn(user: *mut c_void, id: u64, standing: *const GnoblinWmStanding, sink: *const GnoblinWmSink) -> i32,
        /// Focuses a window, and raises it when `raise` is nonzero.
        pub activate: unsafe extern "C" fn(user: *mut c_void, id: u64, raise: u32, standing: *const GnoblinWmStanding, sink: *const GnoblinWmSink) -> i32,
        /// Puts a window on top of its layer.
        pub raise: unsafe extern "C" fn(user: *mut c_void, id: u64, standing: *const GnoblinWmStanding, sink: *const GnoblinWmSink) -> i32,
        /// Puts a window under every other window of its layer.
        pub lower: unsafe extern "C" fn(user: *mut c_void, id: u64, standing: *const GnoblinWmStanding, sink: *const GnoblinWmSink) -> i32,
        /// Maximises a window (`on` nonzero), or returns it to its floating rectangle.
        pub set_maximised: unsafe extern "C" fn(user: *mut c_void, id: u64, on: u32, sink: *const GnoblinWmSink) -> i32,
        /// Makes a window cover its output (`on` nonzero), or returns it to the rectangle it came from.
        pub set_fullscreen: unsafe extern "C" fn(user: *mut c_void, id: u64, on: u32, standing: *const GnoblinWmStanding, sink: *const GnoblinWmSink) -> i32,
        /// Hides a window (`on` nonzero), or brings it back.
        pub set_minimised: unsafe extern "C" fn(user: *mut c_void, id: u64, on: u32, standing: *const GnoblinWmStanding, sink: *const GnoblinWmSink) -> i32,
        /// Moves and resizes a floating window, as a control request asks.
        pub move_resize: unsafe extern "C" fn(user: *mut c_void, id: u64, wanted: *const GnoblinWmRect, sink: *const GnoblinWmSink) -> i32,
        /// Puts a floating window's top left at an exact position and keeps its size.
        pub place: unsafe extern "C" fn(user: *mut c_void, id: u64, x: i32, y: i32, sink: *const GnoblinWmSink) -> i32,
        /// Sets a floating window's rectangle exactly: what an interactive move or resize ends each pointer step on.
        pub set_rect: unsafe extern "C" fn(user: *mut c_void, id: u64, wanted: *const GnoblinWmRect, sink: *const GnoblinWmSink) -> i32,
        /// Takes the size the client drew a floating window at as the window's size: 1 when the record changed, 0 when not.
        pub adopt_size: unsafe extern "C" fn(user: *mut c_void, id: u64, width: u32, height: u32) -> i32,
        /// Moves a window, and its transients, to another output.
        pub move_to_output: unsafe extern "C" fn(user: *mut c_void, id: u64, target: u64, sink: *const GnoblinWmSink) -> i32,
        /// An output arrived.
        pub output_added: unsafe extern "C" fn(user: *mut c_void, id: u64, rect: *const GnoblinWmRect, standing: *const GnoblinWmStanding, sink: *const GnoblinWmSink) -> i32,
        /// An output went away.
        pub output_removed: unsafe extern "C" fn(user: *mut c_void, id: u64, standing: *const GnoblinWmStanding, sink: *const GnoblinWmSink) -> i32,
        /// An output's rectangle changed.
        pub output_moved: unsafe extern "C" fn(user: *mut c_void, id: u64, rect: *const GnoblinWmRect, sink: *const GnoblinWmSink) -> i32,
        /// The insets layer surfaces reserve on an output changed.
        pub reserve: unsafe extern "C" fn(user: *mut c_void, id: u64, reserved: *const GnoblinWmInsets, sink: *const GnoblinWmSink) -> i32,
    }

    /// Where the UI provider puts the pixels of the dialog it draws: `frame` is called once, during the `take_frame` call it
    /// was passed to, and `rgba` is copied before it returns.
    pub struct GnoblinUiFrameSink {
        /// Size of this struct, set by the core.
        pub size: u32,
        /// Zero.
        pub reserved: u32,
        /// The core's own pointer, handed back to `frame`.
        pub sink: *mut c_void,
        /// Takes one frame of `width` by `height` pixels, four bytes per pixel, row-major.
        pub frame: unsafe extern "C" fn(sink: *mut c_void, width: u32, height: u32, rgba: *const u8, len: usize) -> i32,
    }

    /// Where the UI provider puts the verdicts of the requests that settled: `verdict` is called once per request during the
    /// `take_outcomes` call it was passed to, with the verdict in the byte form of `gnoblin_portal_core::wire`.
    pub struct GnoblinUiOutcomeSink {
        /// Size of this struct, set by the core.
        pub size: u32,
        /// Zero.
        pub reserved: u32,
        /// The core's own pointer, handed back to `verdict`.
        pub sink: *mut c_void,
        /// Takes one settled request and its verdict bytes, which are copied before it returns.
        pub verdict: unsafe extern "C" fn(sink: *mut c_void, request: u64, bytes: *const u8, len: usize) -> i32,
    }

    /// The portal dialog UI a provider registers with `provider_register(PROVIDER_UI)`: one dialog on screen at a time, a
    /// queue behind it, and the verdict of each request once it settles. The functions are the methods of
    /// `gnoblin_portal_ui::provider::UiProvider`, driven from the compositor's IPC thread with the time passed in. A dialog,
    /// a verdict and a key cross as the bytes `gnoblin_portal_core::wire` defines, so neither side names the other's types. A
    /// function returns `E_OK` or a negative error (`E_PANIC` after a caught panic); a negative `submit` settles the request
    /// `error(provider-failed)`, and the core closes the provider's open requests with `error(no-ui)` when it ends.
    pub struct GnoblinUiVtable {
        /// Size of this struct, set by the provider.
        pub size: u32,
        /// Zero.
        pub reserved: u32,
        /// Writes the frame size in pixels: the open dialog's size, or the bounds when nothing is open.
        pub frame_size: unsafe extern "C" fn(user: *mut c_void, width: *mut u32, height: *mut u32) -> i32,
        /// Sets the visuals later dialogs are drawn with (`UI_SCHEME_*`).
        pub set_scheme: unsafe extern "C" fn(user: *mut c_void, scheme: u32) -> i32,
        /// Sets the look later dialogs are drawn with, already clamped, as `gnoblin_portal_ui::style::DialogStyle::to_bytes`.
        pub set_style: unsafe extern "C" fn(user: *mut c_void, bytes: *const u8, len: usize) -> i32,
        /// Sets the largest frame a dialog may take, in pixels.
        pub set_bounds: unsafe extern "C" fn(user: *mut c_void, width: u32, height: u32) -> i32,
        /// Writes the request whose dialog is open to `request` and returns 1, or returns 0 when none is open.
        pub active_request: unsafe extern "C" fn(user: *mut c_void, request: *mut u64) -> i32,
        /// The number of admitted requests waiting behind the open dialog.
        pub waiting: unsafe extern "C" fn(user: *mut c_void) -> usize,
        /// Admits a request for the interface `interface` (its place in `Interface::ALL`) on behalf of `app_id`, with the
        /// dialog already built (its bytes). Returns `UI_STARTED`, `UI_QUEUED`, `UI_REFUSED` plus the error's number, or a
        /// negative error.
        pub submit: unsafe extern "C" fn(user: *mut c_void, request: u64, app_id: GnoblinStr, interface: u32, dialog: *const u8, dialog_len: usize, timeout_ms: u64, now: u64) -> i32,
        /// Feeds one key press or repeat (`repeat` nonzero) to the open dialog: its kind and character as `wire::key_code`.
        pub key: unsafe extern "C" fn(user: *mut c_void, kind: u32, character: u32, repeat: u32) -> i32,
        /// Feeds a pointer press at frame pixel `(x, y)` to the open dialog.
        pub pointer_press: unsafe extern "C" fn(user: *mut c_void, x: f32, y: f32) -> i32,
        /// Feeds a pointer release at frame pixel `(x, y)` to the open dialog.
        pub pointer_release: unsafe extern "C" fn(user: *mut c_void, x: f32, y: f32) -> i32,
        /// Moves the clock: animations advance and the open request's deadline is checked.
        pub tick: unsafe extern "C" fn(user: *mut c_void, now: u64) -> i32,
        /// Gives `sink` the open dialog rasterised, but only when its pixels changed since the last frame; nothing when not.
        pub take_frame: unsafe extern "C" fn(user: *mut c_void, sink: *const GnoblinUiFrameSink) -> i32,
        /// Hands the open dialog out again by the next `take_frame`, even when its pixels did not change.
        pub redraw: unsafe extern "C" fn(user: *mut c_void) -> i32,
        /// Gives `sink` every request that settled since the last call, in settle order.
        pub take_outcomes: unsafe extern "C" fn(user: *mut c_void, sink: *const GnoblinUiOutcomeSink) -> i32,
        /// The requester has gone: a dialog open for it closes and a queued one is dropped. Returns 1 when the request was
        /// admitted, 0 when not.
        pub cancel: unsafe extern "C" fn(user: *mut c_void, request: u64) -> i32,
        /// The session lock changes: locking settles every admitted request `error(locked)` and refuses new ones.
        pub set_locked: unsafe extern "C" fn(user: *mut c_void, locked: u32) -> i32,
    }

    /// The configuration observer a provider registers with `provider_register(PROVIDER_CONFIG)`. The core calls `apply` with
    /// the accepted document as JSON when the provider registers, and again on every accepted commit, from the thread that
    /// accepted it; the provider turns the sections it owns into `setting_set` calls. A section the document leaves out puts
    /// its settings back to the default, so the provider sets them all every time.
    pub struct GnoblinConfigVtable {
        /// Size of this struct, set by the provider.
        pub size: u32,
        /// Zero.
        pub reserved: u32,
        /// The accepted document, as UTF-8 JSON. Returns `E_OK` or a negative error, which the core logs.
        pub apply: unsafe extern "C" fn(user: *mut c_void, document: GnoblinStr) -> i32,
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

    /// What a language runtime exports through its one symbol, `gnoruntime_entry_v1`. A runtime is a library that runs scripts
    /// of one language: the loader opens it once, by the script's extension, and calls `start` for each generation of each
    /// script. The runtime keeps the interpreter; a reload is a new `start` with a new context while the old one still serves
    /// (design section 8.2).
    pub struct GnoblinRuntimeEntry {
        /// Size of this struct, set by the runtime.
        pub size: u32,
        /// The ABI major the runtime was built against.
        pub abi_major: u32,
        /// The ABI minor the runtime was built against.
        pub abi_minor_built: u32,
        /// Zero.
        pub flags: u32,
        /// The runtime's name (`js`), valid for the life of the module.
        pub name: GnoblinStr,
        /// The extensions it runs, without dots, separated by spaces (`js mjs ts mts`), valid for the life of the module.
        pub extensions: GnoblinStr,
        /// The runtime's version string, with the versions of the engines baked into it. Caches the runtime makes key on it.
        pub version: GnoblinStr,
        /// Stages a generation of the script at `script`, on the shell thread, with the context made for it: the first
        /// generation runs it and serves; a later one evaluates it beside the generation that serves and waits for `activate`.
        /// Returns `E_OK` or an error code and says why through `set_last_error`; a failed `start` leaves the serving
        /// generation as it was.
        pub start: unsafe extern "C" fn(api: *const GnoblinApi, ctx: GnoblinCtx, script: GnoblinStr) -> i32,
        /// Ends a generation: its listeners and timers are gone when this returns. Runs when the generation retires, after
        /// its shutdown hooks and before its registrations are removed.
        pub shutdown: unsafe extern "C" fn(ctx: GnoblinCtx),
        /// Makes a staged generation the serving one. Absent for a runtime that serves from `start`.
        pub activate: Option<unsafe extern "C" fn(ctx: GnoblinCtx) -> i32>,
        /// Throws a staged generation away unserved. Absent for a runtime that has nothing to undo.
        pub abandon: Option<unsafe extern "C" fn(ctx: GnoblinCtx)>,
        /// One turn of the runtime's own loop: waits up to `timeout_ms` for work the runtime owns (its connections, its
        /// timers), does it, and returns the number of callbacks run, or a negative error code when the runtime cannot go on.
        /// Absent for a runtime that is driven by the core's `dispatch` alone.
        pub turn: Option<unsafe extern "C" fn(ctx: GnoblinCtx, timeout_ms: u32) -> i32>,
    }

    /// A closure the shell gives the library: the payload of a `VALUE_CLOSURE`. Borrowed for the call; the library copies the three
    /// fields when it keeps the closure. A struct that starts with `size` may grow.
    pub struct GnoblinClosure {
        /// Size of this struct, set by the shell.
        pub size: u32,
        /// Zero.
        pub reserved: u32,
        /// Runs the closure: `member` is zero, `args` are what the library passes, `out` takes the answer.
        pub invoke: Option<GnoblinInvokeFn>,
        /// The shell's pointer, handed back to `invoke`. Adopted by the library when the call that carried the closure succeeds.
        pub user: *mut c_void,
        /// Frees `user`: called exactly once, on the shell thread, when the library lets go of the closure, while the generation's
        /// code is still mapped.
        pub drop_user: Option<GnoblinDropFn>,
    }

    /// One property of a new object, for `create` and `update`: the member (a property id of the object's class) and its value.
    pub struct GnoblinInit {
        /// The property's `GnoblinMemberId`.
        pub member: GnoblinMemberId,
        /// Zero.
        pub reserved: u32,
        /// The value, borrowed for the call.
        pub value: GnoblinValue,
    }

    /// What a shell implements of an interface class: the argument of `implement`. A struct that starts with `size` may grow.
    pub struct GnoblinImpl {
        /// Size of this struct, set by the shell.
        pub size: u32,
        /// Zero.
        pub reserved: u32,
        /// A bit mask of the members the shell defined, `n_words` words long, least significant bit first: member id `m` is bit
        /// `m % 64` of word `m / 64` (bit 0 is unused). A member whose bit is clear is never called across the ABI: an optional method
        /// runs the library's default when it has one. A required method whose bit is clear refuses `implement` with `E_TYPE`.
        pub implemented: *const u64,
        /// Number of words at `implemented`.
        pub n_words: usize,
        /// Runs a member: `member` is the method (or the property to read) the library wants.
        pub invoke: Option<GnoblinInvokeFn>,
        /// The shell's pointer, handed back to `invoke`. Adopted when `implement` succeeds.
        pub user: *mut c_void,
        /// Frees `user`, once, when the library lets go of the object.
        pub drop_user: Option<GnoblinDropFn>,
    }

    /// One entry of an object-protocol catalog, from `catalog_each`: a class (`CATALOG_CLASS`) or one of its members
    /// (`CATALOG_MEMBER`). A struct the library fills in starts with `size`, so entries may grow.
    pub struct GnoblinCatalogEntry {
        /// Size of this struct, set by the library.
        pub size: u32,
        /// `CATALOG_CLASS` or `CATALOG_MEMBER`.
        pub entry: u32,
        /// The class: the class an entry describes, or the class that owns the member.
        pub class: GnoblinClassId,
        /// The member's id within its class; zero for a class entry.
        pub member: GnoblinMemberId,
        /// A class's `CLASS_*` kind, or a member's `MEMBER_PROPERTY`, `MEMBER_METHOD`, `MEMBER_SIGNAL` or `MEMBER_VALUE`.
        pub kind: u32,
        /// A class's `CLASS_FLAG_*` flags, or a member's `MEMBER_*` flags.
        pub flags: u32,
        /// A class entry's base class (the same namespace), zero for none.
        pub base: GnoblinClassId,
        /// A property's value kind (`VALUE_*`), a method's result kind (`VALUE_NONE` for none), zero for a signal.
        pub value_kind: u32,
        /// The class of a `VALUE_OBJECT`, `VALUE_RECORD` or `VALUE_ENUM` that `value_kind` names, else zero.
        pub value_class: GnoblinClassId,
        /// Number of arguments: a method's parameters, a signal's fields.
        pub arg_count: u32,
        /// `arg_count` value kinds. Static storage owned by the library.
        pub arg_kinds: *const u32,
        /// The argument names, separated by single spaces. Static storage owned by the library.
        pub arg_names: GnoblinStr,
        /// The class's or member's name, as the catalog spells it. Static storage owned by the library.
        pub name: GnoblinStr,
    }

    /// The object protocol of one namespace: classes, properties, methods and signals served from a catalog through a few
    /// functions, so a class added to the catalog needs no function in `GnoblinApi` or `GnoblinStdApi`. Got from `object_api`.
    ///
    /// Every function runs on the shell thread and returns `E_OK` or a negative error code with the reason in the serving
    /// library's last-error text. Inputs are borrowed for the call and the library copies what it keeps. A `GnoblinValue` the
    /// library writes is lent: it stays valid until the next call into this table on the calling thread, and is never
    /// `VALUE_OWNED`. A handle the library hands out (a result, a payload, a callback argument) is one reference its receiver
    /// `release`s. The library never runs a closure or an implemented method inside `create`, `update`, `set`, `call` or `destroy`:
    /// a change announces itself from the next turn of the shell's dispatch, so a handler cannot re-enter the call that caused it.
    /// A closure inside a value that a call carries is adopted only if the call returns `E_OK`; after any other result the shell
    /// still owns its `user`.
    pub struct GnoblinObjectApi {
        /// Byte size of this table.
        pub size: u32,
        /// Zero.
        pub reserved: u32,
        /// The catalog's hash (`gnoblin_sdkgen`'s `Catalog::namespace_fingerprint`): a generated SDK compares it with its own to learn
        /// whether it matches the library it runs on.
        pub catalog_hash: u64,
        /// Writes the id of the class `name` to `out`. `E_NO_SUCH_MEMBER` for a name the catalog lacks.
        pub class_id: unsafe extern "C" fn(ctx: GnoblinCtx, name: GnoblinStr, out: *mut GnoblinClassId) -> i32,
        /// Writes the id of the member `name` of `class`, of `kind` (`MEMBER_*`), to `out`. A member of a base class has the id it has
        /// in the base. `E_NO_SUCH_MEMBER` for a name the class lacks.
        pub member_id: unsafe extern "C" fn(ctx: GnoblinCtx, class: GnoblinClassId, kind: u32, name: GnoblinStr, out: *mut GnoblinMemberId) -> i32,
        /// Calls `cb` for every class of the namespace and each of its members, in id order. A nonzero return of `cb` ends the walk
        /// and is returned.
        pub catalog_each: unsafe extern "C" fn(ctx: GnoblinCtx, cb: GnoblinCatalogFn, user: *mut c_void) -> i32,
        /// Makes an object of `class` with the `n` properties of `inits` set, and writes its handle to `out`. A service class gives its
        /// one object. `E_TYPE` for a record, an enum, an interface (those are made by `implement`), a value the property does not take,
        /// or a property that cannot be set.
        pub create: unsafe extern "C" fn(ctx: GnoblinCtx, class: GnoblinClassId, inits: *const GnoblinInit, n: usize, out: *mut GnoblinObj) -> i32,
        /// Ends an object and every object that is its child: its listeners go, `drop_user` of each closure it held runs, and every
        /// handle to it is `E_STALE`.
        pub destroy: unsafe extern "C" fn(ctx: GnoblinCtx, obj: GnoblinObj) -> i32,
        /// Drops one reference the shell holds to an object. An object that nothing else holds ends when the last reference goes,
        /// unless its class is retained (`CLASS_FLAG_RETAINED`) or it has a parent: those stay until `destroy`.
        pub release: unsafe extern "C" fn(ctx: GnoblinCtx, obj: GnoblinObj) -> i32,
        /// Reads a property into `out`. A property never set reads as its catalog default, or the empty value of its type.
        pub get: unsafe extern "C" fn(ctx: GnoblinCtx, obj: GnoblinObj, member: GnoblinMemberId, out: *mut GnoblinValue) -> i32,
        /// Sets a property. `E_TYPE` for a value of the wrong kind or a read-only property.
        pub set: unsafe extern "C" fn(ctx: GnoblinCtx, obj: GnoblinObj, member: GnoblinMemberId, value: *const GnoblinValue) -> i32,
        /// Sets the `n` properties of `inits`, in order, stopping at the first that fails.
        pub update: unsafe extern "C" fn(ctx: GnoblinCtx, obj: GnoblinObj, inits: *const GnoblinInit, n: usize) -> i32,
        /// Sets an `MEMBER_ANIMATED` property as a timed write: it plays over `timing.seconds` along the curve of `timing.samples`.
        pub set_timed: unsafe extern "C" fn(ctx: GnoblinCtx, obj: GnoblinObj, member: GnoblinMemberId, value: *const GnoblinValue, timing: *const GnoblinTiming) -> i32,
        /// Calls a method with `n` arguments and writes its answer to `out` (`VALUE_NONE` when it answers nothing). The arguments are
        /// checked against the catalog: a missing optional one reads as `VALUE_NONE`, a surplus or mistyped one is `E_TYPE`.
        pub call: unsafe extern "C" fn(ctx: GnoblinCtx, obj: GnoblinObj, member: GnoblinMemberId, args: *const GnoblinValue, n: usize, out: *mut GnoblinValue) -> i32,
        /// Calls a function of a class (`MEMBER_STATIC`).
        pub call_static: unsafe extern "C" fn(ctx: GnoblinCtx, class: GnoblinClassId, member: GnoblinMemberId, args: *const GnoblinValue, n: usize, out: *mut GnoblinValue) -> i32,
        /// Calls `cb` from the next turn of the dispatch each time `member` of `obj` happens: a signal (the payload is its fields) or
        /// a property (the payload is the new value; changes of one turn are told once). Zero on failure, with `user` not adopted.
        pub listen: unsafe extern "C" fn(ctx: GnoblinCtx, obj: GnoblinObj, member: GnoblinMemberId, cb: GnoblinInvokeFn, user: *mut c_void, drop_user: Option<GnoblinDropFn>) -> GnoblinReg,
        /// Ends a listener: `drop_user` runs now, or after the delivery that is running returns.
        pub unlisten: unsafe extern "C" fn(ctx: GnoblinCtx, reg: GnoblinReg) -> i32,
        /// Makes an object of the interface class `class` that the shell implements, and writes its handle to `out`. The library
        /// adopts `imp.user` when this succeeds and calls its `drop_user` once, when the last holder (the shell's reference and the
        /// native objects that took it) lets go.
        pub implement: unsafe extern "C" fn(ctx: GnoblinCtx, class: GnoblinClassId, imp: *const GnoblinImpl, out: *mut GnoblinObj) -> i32,
        /// Writes the class of an object to `out`: the most derived one.
        pub class_of: unsafe extern "C" fn(ctx: GnoblinCtx, obj: GnoblinObj, out: *mut GnoblinClassId) -> i32,
        /// Installs a tracker for this thread, or clears it when `cb` is null: from now on every `get` of this library on this thread calls
        /// `cb(user, obj, class, member)` once the read succeeded, until the tracker is cleared. It is how a binding that computes a value
        /// from properties of objects of several libraries learns what it depends on (the core runs the compute, installs a tracker on
        /// each library it may read, and listens to what was read). `user` is borrowed: the caller keeps it alive until it clears the
        /// tracker. A second tracker replaces the first. The tracker must not call into the library that called it.
        pub track: unsafe extern "C" fn(ctx: GnoblinCtx, cb: Option<GnoblinTrackFn>, user: *mut c_void) -> i32,
        /// Copies the code the script API gives the last failed call of this table on this thread, as `gnoblin_last_error` copies its text:
        /// into `buf`, NUL terminated and truncated to `cap`, returning the code's full length. The code is one of `invalid_argument`,
        /// `not_found`, `already_exists`, `invalid_state`, `read_only`, `unsupported`, `denied`, `busy`, `cancelled`, `timed_out`,
        /// `unavailable`, `internal` and `config_rejected`, in ASCII and at most 16 bytes. The integer a failed call returns says the kind
        /// of failure and stays as it is; this says why, because `E_REFUSED` and `E_ERR` stand for several reasons. The length is zero and
        /// `buf` is not written when the failure has no code (a bad argument, `E_TYPE`) or no call has failed. The library copies: nothing
        /// is lent. Read it right after the call that failed, on the same thread, before another call into the table. A table too short to
        /// hold this field (`table_has`) belongs to a library that reports no codes, and the integer is all it says.
        pub last_error_code: unsafe extern "C" fn(buf: *mut u8, cap: usize) -> usize,
        /// Adds one reference to `obj`, which the caller gives back with `release`. It is how a library that hands out an object of
        /// another library (`VALUE_FOREIGN_OBJECT`) gives each receiver the reference it releases. Fails with `E_STALE` when the
        /// object is gone. A table too short to hold this field (`table_has`) belongs to a library built before it, whose objects
        /// another library cannot pass on.
        pub retain: unsafe extern "C" fn(ctx: GnoblinCtx, obj: GnoblinObj) -> i32,
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

        // ---- the tree: attach, read, write ----------------------------------------------------------------------------

        /// Attaches the shell to the compositor's tree: `ATTACH_HOLDER` takes it as the shell (one holder at a time,
        /// `E_BUSY` for a second), `ATTACH_OBSERVER` reads it. Blocks up to the boot wait for the snapshot. A reload's
        /// generation attaches again and shares the tree the first one holds. Shell thread.
        pub tree_attach: unsafe extern "C" fn(ctx: GnoblinCtx, mode: u32) -> i32,
        /// Writes the id of the tree's root node to `out`: the parent of the windows and of the shell's top-level nodes. Shell thread.
        pub tree_root: unsafe extern "C" fn(ctx: GnoblinCtx, out: *mut GnoblinNode) -> i32,
        /// 1 when `node` is in the tree as this tick sees it (a write made in this tick counts), 0 when it is not, or a
        /// negative error. Shell thread.
        pub node_exists: unsafe extern "C" fn(ctx: GnoblinCtx, node: GnoblinNode) -> i32,
        /// The catalog class id of `node` (`catalog_class`), or zero when the node is not in the tree or its class is not
        /// in the catalog (`gnoblin_last_error` says which). Shell thread.
        pub node_class: unsafe extern "C" fn(ctx: GnoblinCtx, node: GnoblinNode) -> u32,
        /// Writes the parent of `node` to `out`: zero for the root and for a node not inserted yet. Shell thread.
        pub node_parent: unsafe extern "C" fn(ctx: GnoblinCtx, node: GnoblinNode, out: *mut GnoblinNode) -> i32,
        /// Writes the children of `node`, in order, to `out`. The array is valid until the next ABI call on this thread.
        /// Shell thread.
        pub node_children: unsafe extern "C" fn(ctx: GnoblinCtx, node: GnoblinNode, out: *mut GnoblinNodes) -> i32,
        /// Reads a property of `node`, through this tick's writes: `prop` is the property's id in the node's class
        /// (`catalog_member`). A property that was never set reads as its schema default, or the empty value of its kind (zero,
        /// false, empty text). Text and bytes are owned by the caller (`VALUE_OWNED`, freed with `value_free`); a list is lent
        /// until the next call that lends transient data on this thread (`node_children`, `class_name`, another list read).
        /// Shell thread.
        pub node_get: unsafe extern "C" fn(ctx: GnoblinCtx, node: GnoblinNode, prop: u32, out: *mut GnoblinValue) -> i32,
        /// Writes the schema name of a catalog class (`window`, `gnoblin.session`) to `out`. The text is valid until the next
        /// ABI call on this thread. Shell thread.
        pub class_name: unsafe extern "C" fn(ctx: GnoblinCtx, class: u32, out: *mut GnoblinStr) -> i32,

        /// Makes a node of `class` in this tick and writes its id to `out`. The id is the shell's own; the node exists in
        /// this shell's reads at once and in the compositor when the tick's transaction is applied. Shell thread.
        pub write_create: unsafe extern "C" fn(ctx: GnoblinCtx, class: u32, out: *mut GnoblinNode) -> i32,
        /// Destroys `node` and everything beneath it. Shell thread.
        pub write_destroy: unsafe extern "C" fn(ctx: GnoblinCtx, node: GnoblinNode) -> i32,
        /// Inserts `node` under `parent` at `index` (negative appends), moving it from its parent. Shell thread.
        pub write_insert: unsafe extern "C" fn(ctx: GnoblinCtx, parent: GnoblinNode, node: GnoblinNode, index: i64) -> i32,
        /// Sets a property of `node` to `value`. With `seconds > 0` on an `MEMBER_ANIMATED` property the compositor plays
        /// the change over that many seconds. Writes are queued in this tick and sent as one transaction when the dispatch
        /// cycle ends (or at `flush`). Shell thread.
        pub write_set: unsafe extern "C" fn(ctx: GnoblinCtx, node: GnoblinNode, prop: u32, value: *const GnoblinValue, seconds: f64) -> i32,
        /// Calls method `method` (its id in the node's class) of `node` with `n_args` arguments, queued like a write. When
        /// `cb` is given, it receives the compositor's answer once, on the shell thread, and the pending call's handle is
        /// written to `out` (cancel it with `cancel`); an answer that never comes is the shell's to cancel. Without `cb` nobody
        /// waits: `user` and `drop_user` are ignored and `out` receives zero. A call that fails has not adopted `user`. The
        /// arguments are checked against the method's parameters (`catalog_member` gives their kinds): integers, booleans and
        /// node ids travel as the call's integers and text as its text. Shell thread.
        pub write_call: unsafe extern "C" fn(ctx: GnoblinCtx, node: GnoblinNode, method: u32, args: *const GnoblinValue, n_args: usize, cb: Option<GnoblinAnswerFn>, user: *mut c_void, drop_user: Option<GnoblinDropFn>, out: *mut GnoblinPending) -> i32,
        /// Sends this tick's writes now, as one transaction when the gate allows (one is in flight at a time). `cb` is
        /// told once when everything sent so far has been answered: `E_OK` with the serial, or the refusal. Shell thread.
        pub flush: unsafe extern "C" fn(ctx: GnoblinCtx, cb: Option<GnoblinAnswerFn>, user: *mut c_void, drop_user: Option<GnoblinDropFn>, out: *mut GnoblinPending) -> i32,
        /// Cancels a pending call or flush: its callback never runs and `drop_user` runs now. Shell thread.
        pub cancel: unsafe extern "C" fn(pending: GnoblinPending) -> i32,

        // ---- events -----------------------------------------------------------------------------------------------------

        /// Calls `cb` when a property changes: of one node (`node` nonzero) or of every node of `class`, for property `prop`
        /// of that class or for every property (`prop` zero). A node made in the same batch raises none: it is born with its
        /// final values. Zero on failure, with `user` not adopted. Shell thread.
        pub on_change: unsafe extern "C" fn(ctx: GnoblinCtx, node: GnoblinNode, class: u32, prop: u32, cb: GnoblinNodeFn, user: *mut c_void, drop_user: Option<GnoblinDropFn>) -> GnoblinReg,
        /// Calls `cb` when a node of `class` (zero for every class) comes into the tree: a window appearing. The batch that
        /// made it has been applied, so the callback reads the node as it will stay. Shell thread.
        pub on_node_created: unsafe extern "C" fn(ctx: GnoblinCtx, class: u32, cb: GnoblinNodeFn, user: *mut c_void, drop_user: Option<GnoblinDropFn>) -> GnoblinReg,
        /// Calls `cb` when a node of `class` (zero for every class) leaves the tree. The node is already gone from reads;
        /// the event carries what is left of it. Shell thread.
        pub on_node_destroyed: unsafe extern "C" fn(ctx: GnoblinCtx, class: u32, cb: GnoblinNodeFn, user: *mut c_void, drop_user: Option<GnoblinDropFn>) -> GnoblinReg,

        // ---- input claims, timers, processes, state -------------------------------------------------------------------

        /// Claims input on `seat` as `spec` describes (a chord, a touchpad gesture, a pointer's buttons, a device, or an input
        /// grab) and calls `cb` for each event of the claim. The claim is made as a call on the seat in the next
        /// transaction and lasts until `remove(reg)`, which gives it up, or the generation retires. Zero on failure, with
        /// `user` not adopted. Shell thread.
        pub claim: unsafe extern "C" fn(ctx: GnoblinCtx, seat: GnoblinNode, spec: *const GnoblinClaim, cb: Option<GnoblinClaimFn>, user: *mut c_void, drop_user: Option<GnoblinDropFn>) -> GnoblinReg,
        /// Calls `cb` after `ms` milliseconds, and every `ms` milliseconds when `repeat` is nonzero, until `remove(reg)` or
        /// the generation retires. Zero on failure, with `user` not adopted. Shell thread.
        pub timer_add: unsafe extern "C" fn(ctx: GnoblinCtx, ms: u32, repeat: u32, cb: GnoblinTimerFn, user: *mut c_void, drop_user: Option<GnoblinDropFn>) -> GnoblinReg,
        /// Starts `argv` (`n_argv` strings) in a process group of its own and writes its handle to `out`. `options` may be
        /// null. A program the shell's `init` starts waits until the generation takes over, and one that a reload's
        /// generation starts again with the same arguments is adopted, not doubled. `on_exit` is called once when it ends.
        /// Shell thread.
        pub spawn: unsafe extern "C" fn(ctx: GnoblinCtx, argv: *const GnoblinStr, n_argv: usize, options: *const GnoblinSpawn, on_exit: Option<GnoblinExitFn>, user: *mut c_void, drop_user: Option<GnoblinDropFn>, out: *mut GnoblinProc) -> i32,
        /// Sends the named signal (`term`, `kill`, `int`, `hup`, `usr1`, `usr2`, `stop`, `cont`) to a process group. Shell thread.
        pub proc_kill: unsafe extern "C" fn(ctx: GnoblinCtx, process: GnoblinProc, signal: *const u8, len: usize) -> i32,
        /// Keeps `bytes` under `key` for the next generation of this shell (a reload), with the caller's `version` number.
        /// The store is dropped when the compositor restarts and holds at most `STATE_MAX_BYTES` in all. Shell thread.
        pub state_put: unsafe extern "C" fn(ctx: GnoblinCtx, key: *const u8, key_len: usize, bytes: *const u8, len: usize, version: u32) -> i32,
        /// Reads what an earlier generation kept under `key`: the bytes (owned by the caller, freed with `buf_free`) and the
        /// version they were kept with. `E_NO_SUCH_MEMBER` when nothing is kept under `key`. Shell thread.
        pub state_get: unsafe extern "C" fn(ctx: GnoblinCtx, key: *const u8, key_len: usize, out: *mut GnoblinBuf, version: *mut u32) -> i32,
        /// Makes the node of `class` this shell calls `name`, or finds it again after a reload. If an earlier generation of this
        /// shell made a node of that class under that name and it is still in the tree, its id is written to `out` and the
        /// result is `ADOPTED`: the node, its children and every value on it are exactly as they were, so a reload does not
        /// make a second one. Otherwise a node is made as `write_create` does, its `name` property is set to `name`, and the
        /// result is `E_OK`. Asking twice in one generation gives the same node. With `NODE_FLAG_EPHEMERAL` the node dies
        /// with its generation: when the next generation takes over, an ephemeral node it did not adopt during its `init` is
        /// destroyed. Nodes made by `write_create`, and nodes the compositor owns (windows, screens, seats), are never
        /// destroyed by a reload. Shell thread.
        pub node_adopt: unsafe extern "C" fn(ctx: GnoblinCtx, class: u32, name: *const u8, name_len: usize, flags: u32, out: *mut GnoblinNode) -> i32,
        /// Sets a property of `node` to `value` as a timed write: the compositor plays the change over `timing.seconds` along
        /// the curve of `timing.samples`, after `timing.delay`, as part of `timing.tween` when one is named. The property must be
        /// `MEMBER_ANIMATED`. Writes are queued like `write_set`'s. Shell thread.
        pub write_set_timed: unsafe extern "C" fn(ctx: GnoblinCtx, node: GnoblinNode, prop: u32, value: *const GnoblinValue, timing: *const GnoblinTiming) -> i32,

        // ---- the relayed link: a runtime that keeps its own mirror drives the compositor's link as bytes ---------------

        /// Opens the compositor's link for a runtime that keeps its own mirror and replaces `tree_attach` for this shell:
        /// `mode` is `ATTACH_HOLDER` or `ATTACH_OBSERVER`. The core does not attach; the runtime does, through
        /// `tree_link_attach`. A shell that attached with `tree_attach` gets `E_BUSY`. Shell thread.
        pub tree_link_open: unsafe extern "C" fn(ctx: GnoblinCtx, mode: u32) -> i32,
        /// Sends the attach request over the opened link, with the protocol `version`. Shell thread.
        pub tree_link_attach: unsafe extern "C" fn(ctx: GnoblinCtx, version: u64) -> i32,
        /// Sends one transaction over the opened link: `bytes` is a `gnoblin_tree_client::relay` encoding. Shell thread.
        pub tree_link_send: unsafe extern "C" fn(ctx: GnoblinCtx, bytes: *const u8, len: usize) -> i32,
        /// Takes everything that arrived on the opened link since the last call, as one `gnoblin_tree_client::relay`
        /// encoding in `out` (free it with `buf_free`); an empty batch is a valid encoding. It never waits. A failed link is
        /// `E_ERR` with the reason. Shell thread.
        pub tree_link_recv: unsafe extern "C" fn(ctx: GnoblinCtx, out: *mut GnoblinBuf) -> i32,
        /// A descriptor that turns readable when the link has something to take, duplicated for the caller, who closes it;
        /// negative when the link has none (and the caller polls). Shell thread.
        pub tree_link_fd: unsafe extern "C" fn(ctx: GnoblinCtx) -> i32,

        // ---- listing the compositor's nodes by class ------------------------------------------------------------------
        /// Writes every node of `class` that the compositor has published, as this tick sees it, to `out`, ordered by id (the
        /// order the nodes came into the tree). Windows and layer surfaces arrive unparented: they are not children of the
        /// root until a shell inserts them, so this is how a shell lists them. The array is valid until the next ABI call on
        /// this thread. Shell thread.
        pub class_nodes: unsafe extern "C" fn(ctx: GnoblinCtx, class: u32, out: *mut GnoblinNodes) -> i32,

        // ---- providers and compositor settings --------------------------------------------------------------------------

        /// Registers a provider of `kind` (`PROVIDER_WM`, `PROVIDER_UI` or `PROVIDER_CONFIG`): `vtable` points at that kind's
        /// vtable, read by its own `size`, and `user` is handed back to each of its functions. The provider serves while the
        /// generation lives: when the generation retires, or `remove(reg)` is called, the core stops calling it, runs
        /// `drop_user` on `user` from the thread the provider was called on and falls back (`NullWm`, `NoUi`, no observer).
        /// The window manager, the UI and the observer are one each: a second registration of a kind while one is live is
        /// `E_BUSY` with `user` not adopted. Zero on failure, with `user` not adopted. Shell thread of the generation, or the
        /// thread that made a context of the host's own (std's).
        pub provider_register: unsafe extern "C" fn(ctx: GnoblinCtx, kind: u32, vtable: *const c_void, user: *mut c_void, drop_user: Option<GnoblinDropFn>) -> GnoblinReg,
        /// Sets a compositor setting by key: `a11y.zoom` (a double, at most 1 is off), `a11y.filter` (bytes, 80 for the twenty
        /// little-endian `f32` of a 4 by 5 colour matrix, none for off), `bell.flash` (text `off`, `frame` or `fullscreen`), and the window
        /// management keys `wm.edge_tiling`, `wm.drag_constraint`, `wm.new_window_focus`, `wm.focus_mode`,
        /// `wm.focus_change_on_pointer_rest`, `wm.raise_on_click`, `wm.auto_raise`, `wm.auto_raise_delay`,
        /// `wm.check_alive_timeout`, `wm.titlebar.double_click`, `wm.titlebar.middle_click` and `wm.titlebar.right_click`.
        /// `E_NO_SUCH_MEMBER` for another key, `E_TYPE` for a value of the wrong kind, `E_REFUSED` with the reason for a
        /// value out of range. Any thread.
        pub setting_set: unsafe extern "C" fn(ctx: GnoblinCtx, key: *const u8, key_len: usize, value: *const GnoblinValue) -> i32,
        // ---- the guardian's reports -----------------------------------------------------------------------------------
        /// Tells the host the shell committed `document` (UTF-8 JSON, `len` bytes) as configuration `revision`, and the compositor
        /// accepted it. The host tells the guardian: one `autostart` line for every enabled entry of the document, then the
        /// revision accepted. A shell with no configuration document of its own never calls it (the host reports revision zero
        /// for it). Shell thread.
        pub report_config: unsafe extern "C" fn(ctx: GnoblinCtx, revision: u64, document: *const u8, len: usize) -> i32,
        /// Tells the host the shell refused a configuration revision (`revision` is zero when the document never got that far),
        /// with the reason as UTF-8 text. The guardian is told; the last accepted configuration stays in force. Shell thread.
        pub report_config_rejected: unsafe extern "C" fn(ctx: GnoblinCtx, revision: u64, reason: *const u8, len: usize) -> i32,

        // ---- the shell's own event loop, and the compositor's verbs ----------------------------------------------------------

        /// A descriptor that turns readable when `dispatch` has something to do: a post or an event is queued, a timer fell due,
        /// or the compositor's link has bytes to read. It is level triggered and it never carries data: a shell that owns its
        /// own event loop (tokio, calloop, GLib, an interpreter's) polls it beside its own sources and calls `dispatch(ctx, 0)`
        /// when it is readable, which drains it. It is one descriptor for the whole shell instance, so a reload's generation
        /// gets the same one. The caller owns the duplicate it is given and closes it. Any thread.
        pub wake_fd: unsafe extern "C" fn(ctx: GnoblinCtx) -> i32,
        /// Calls method `method` (its id in `class`'s catalog list) of an object of the compositor's `gnoblin` namespace (the
        /// session, idle, outputs) over the shell's owner connection: `object` is the compositor's object identity, or zero for
        /// the class's one object. The answer arrives through `cb` as `write_call`'s does: `E_OK` and the operation's value as
        /// JSON text (empty for a bare `done`), or `E_REFUSED` and the compositor's error code as the text. Arguments are
        /// checked against the method's parameters and sent by their schema names. A call that fails has not adopted `user`.
        /// Shell thread.
        pub call: unsafe extern "C" fn(ctx: GnoblinCtx, object: u64, class: u32, method: u32, args: *const GnoblinValue, n_args: usize, cb: Option<GnoblinAnswerFn>, user: *mut c_void, drop_user: Option<GnoblinDropFn>, out: *mut GnoblinPending) -> i32,
        /// Calls `cb` for every `signal` that an object of the compositor's `gnoblin` namespace raises (`gnoblin.session`'s
        /// `bell`, `lock_state_changed`, `reload_requested`), over the shell's owner connection, with the payload in the
        /// signal's schema order. Zero on failure (no owner connection), with `user` not adopted. Shell thread.
        pub subscribe: unsafe extern "C" fn(ctx: GnoblinCtx, class: u32, signal: u32, cb: GnoblinEventFn, user: *mut c_void, drop_user: Option<GnoblinDropFn>) -> GnoblinReg,

        // ---- the object protocol ---------------------------------------------------------------------------------------

        /// The object protocol (`GnoblinObjectApi`) of `namespace`: the classes this library serves, `gnoblin.model` for the core. The
        /// table is static, the same for every context, and valid for the life of the process. `E_NO_SUCH_MEMBER` for a namespace
        /// this library does not serve. Shell thread.
        pub object_api: unsafe extern "C" fn(ctx: GnoblinCtx, namespace: GnoblinStr, out: *mut *const GnoblinObjectApi) -> i32,

        /// Installs the write stage hook of this thread, or clears it. With `stage` set, every write of a property that an object of the
        /// core's object model (`gnoblin.model`: a client's `opacity`, `x`, `rotation` and the like) makes to a node is first offered to
        /// `stage(user, node, key, value)`; when it returns nonzero the core does not send the write. Reads of those properties ask
        /// `peek` first (when it is not null) and answer with the value it holds, so a caller reads back what it staged. A timed write
        /// (`set_timed`) is never offered, and neither are the writes the node API makes (`write_set`, `write_set_timed`): the hook
        /// stages the writes of an object's setter, and what it staged it sends itself through the node API. `user` is borrowed: the caller
        /// keeps it alive until it clears the hook. A second hook replaces the first. With `stage` null the call clears the hook when
        /// `user` is the one that installed it, and does nothing otherwise; it never fails, so a caller may clear on any path, even after
        /// its context retired. The hook must not call into the object library that called it. A table too short to hold this field
        /// (`table_has`) belongs to a core built before it, whose object writes cannot be staged. Shell thread.
        pub set_write_stage: unsafe extern "C" fn(ctx: GnoblinCtx, stage: Option<GnoblinWriteStageFn>, peek: Option<GnoblinWriteStagePeekFn>, user: *mut c_void) -> i32,

        /// The serial of the last write of `node`'s `effects` that this shell made, written to `out`: zero when it made none. The node
        /// API's writes of `effects` (`write_set`, `write_set_timed`) each take the next serial of the session, so a caller that read
        /// the serial after its own write knows later that the list is still its own when the serial is unchanged, and that
        /// some other write replaced it when it is not. A write that a staged generation made and that was rolled back with its stage
        /// gives its serial back to the one before it. The effects themselves are not carried out by `node_get`; this is the only
        /// read of them. `E_NO_SUCH_NODE` for a node the tree does not hold. A table too short to hold this field (`table_has`)
        /// belongs to a core built before it, which cannot say. Shell thread.
        pub node_effects_serial: unsafe extern "C" fn(ctx: GnoblinCtx, node: GnoblinNode, out: *mut u64) -> i32,

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
    /// `int gnoblin_main(int argc, char **argv)`: the launcher's entry. `argv` is the launcher's command line as it was
    /// parsed (the program name, `--role compositor`, then an optional shell path). The core initialises the log, serves
    /// the role in the calling thread until the session ends and returns the process's exit code. It returns 2 for a
    /// command line it does not serve, 1 for a start that failed and the logout code for a logout.
    MainFn => gnoblin_main = unsafe extern "C" fn(argc: c_int, argv: *const *const c_char) -> c_int;
}
