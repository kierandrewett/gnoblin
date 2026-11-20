export declare const ABI_MAJOR = 1;
export declare const ABI_MINOR = 0;
export declare const SONAME = "libgnoblin-core.so.1";
/** `E_*` constants of the ABI. */
export declare const ErrorKind: {
    /** Success. */
    readonly OK: 0;
    /** Failure; see `gnoblin_last_error`. */
    readonly ERR: -1;
    /** A panic was caught at this boundary (either side). */
    readonly PANIC: -2;
    /** The function or catalog member is not in this core (an older minor, or removed from the schema). */
    readonly UNSUPPORTED: -3;
    /** Major mismatch or a malformed `size`. */
    readonly ABI: -4;
    /** A module failed to load or initialise. */
    readonly LOAD: -5;
    /** A handle that was removed, or never existed. */
    readonly STALE: -6;
    /** The calling generation has been retired; the call did nothing. */
    readonly RETIRED: -7;
    /** Node id not in the mirror. */
    readonly NO_SUCH_NODE: -8;
    /** Class, property, method or signal not in the catalog, or not a member of that class. */
    readonly NO_SUCH_MEMBER: -9;
    /** A value of the wrong kind for the member. */
    readonly TYPE: -10;
    /** The compositor refused the request. */
    readonly REFUSED: -11;
    /** A shell-thread-only function was called from another thread. */
    readonly THREAD: -12;
    /** A single-holder resource is taken. */
    readonly BUSY: -13;
    /** A deadline passed. */
    readonly TIMEOUT: -14;
};
export type ErrorKind = (typeof ErrorKind)[keyof typeof ErrorKind];
/** `LOG_*` constants of the ABI. */
export declare const LogKind: {
    /** `log` level: error. */
    readonly ERROR: 1;
    /** `log` level: warning. */
    readonly WARN: 2;
    /** `log` level: information. */
    readonly INFO: 3;
    /** `log` level: debug. */
    readonly DEBUG: 4;
    /** `log` level: trace. */
    readonly TRACE: 5;
};
export type LogKind = (typeof LogKind)[keyof typeof LogKind];
/** `VALUE_*` constants of the ABI. */
export declare const ValueKind: {
    /** `GnoblinValue.kind`: a boolean, in `bits` (0 or 1). */
    readonly BOOL: 1;
    /** `GnoblinValue.kind`: a signed 64-bit integer, in `bits` (two's complement). */
    readonly INT: 2;
    /** `GnoblinValue.kind`: an unsigned 64-bit integer, in `bits`. */
    readonly UINT: 3;
    /** `GnoblinValue.kind`: a double, in `bits` (IEEE 754 bit pattern). */
    readonly DOUBLE: 4;
    /** `GnoblinValue.kind`: UTF-8 text in `ptr` and `len`. */
    readonly TEXT: 5;
    /** `GnoblinValue.kind`: a node id, in `bits`. */
    readonly NODE: 6;
    /** `GnoblinValue.kind`: a list of values: `ptr` points at `len` `GnoblinValue`s. */
    readonly LIST: 7;
    /** `GnoblinValue.kind`: a list of strings: `ptr` points at `len` `GnoblinStr`s. */
    readonly TEXTS: 8;
    /** `GnoblinValue.kind`: a list of effects: `ptr` points at `len` `GnoblinEffect`s (a node's `effects`). Only a write carries it. */
    readonly EFFECTS: 9;
    /** `GnoblinValue.kind`: opaque bytes in `ptr` and `len`. */
    readonly BYTES: 10;
    /** `GnoblinValue.kind`: secret text in `ptr` and `len`. Never logged; the receiver copies it out. */
    readonly SECRET: 11;
    /** `GnoblinValue.kind`: a record value (a struct-like object reference); `ptr` points at `len` `GnoblinValue`s, */
    readonly RECORD: 12;
    /** `GnoblinValue.flags`: `ptr` was allocated by the core and is freed with `value_free`. */
    readonly OWNED: 1;
};
export type ValueKind = (typeof ValueKind)[keyof typeof ValueKind];
/** `EFFECT_*` constants of the ABI. */
export declare const EffectKind: {
    /** `GnoblinEffect.kind`: a backdrop blur. */
    readonly BLUR: 1;
    /** `GnoblinEffect.kind`: a drop shadow. */
    readonly SHADOW: 2;
    /** `GnoblinEffect.kind`: a border stroke. */
    readonly BORDER: 3;
    /** `GnoblinEffect.kind`: a user shader pass, written in WGSL (`source` and `params`). */
    readonly SHADER: 4;
};
export type EffectKind = (typeof EffectKind)[keyof typeof EffectKind];
/** `MEMBER_*` constants of the ABI. */
export declare const MemberKind: {
    /** A catalog member kind: a property. */
    readonly PROPERTY: 1;
    /** A catalog member kind: a method. */
    readonly METHOD: 2;
    /** A catalog member kind: a signal. */
    readonly SIGNAL: 3;
    /** `GnoblinMember.flags`: a `write_set` may carry `seconds` to animate the property. */
    readonly ANIMATED: 1;
    /** `GnoblinMember.flags`: the property cannot be set. */
    readonly READONLY: 2;
    /** `GnoblinMember.flags`: the member's values are secret. */
    readonly SECRET: 4;
};
export type MemberKind = (typeof MemberKind)[keyof typeof MemberKind];
/** `LANG_*` constants of the ABI. */
export declare const LangKind: {
    /** `GnoblinCtxInfo.language`: a native shell module. */
    readonly NATIVE: 1;
    /** `GnoblinCtxInfo.language`: a Python shell. */
    readonly PYTHON: 2;
    /** `GnoblinCtxInfo.language`: a JavaScript shell. */
    readonly JS: 3;
    /** `GnoblinCtxInfo.language`: a TypeScript shell. */
    readonly TS: 4;
    /** `GnoblinCtxInfo.language`: a script of a language no other constant names, run by a runtime library that handles its */
    readonly OTHER: 5;
};
export type LangKind = (typeof LangKind)[keyof typeof LangKind];
/** `ATTACH_*` constants of the ABI. */
export declare const AttachFlags: {
    /** `tree_attach` mode: take the tree as the shell, the only one that may change it. */
    readonly HOLDER: 1;
    /** `tree_attach` mode: read the tree and its changes. */
    readonly OBSERVER: 2;
};
export type AttachFlags = (typeof AttachFlags)[keyof typeof AttachFlags];
/** `NODE_*` constants of the ABI. */
export declare const NodeKind: {
    /** `GnoblinNodeEvent.kind`: a node came into the tree. */
    readonly CREATED: 1;
    /** `GnoblinNodeEvent.kind`: a node and its subtree left the tree. */
    readonly DESTROYED: 2;
    /** `GnoblinNodeEvent.kind`: a property of a node changed. */
    readonly CHANGED: 3;
    /** `node_adopt` flag: the node dies with its generation unless the next one adopts it during its `init`. */
    readonly FLAG_EPHEMERAL: 1;
};
export type NodeKind = (typeof NodeKind)[keyof typeof NodeKind];
/** `CLAIM_*` constants of the ABI. */
export declare const ClaimKind: {
    /** `GnoblinClaim.kind`: a key chord, or a pointer chord when it ends in `Button1` to `Button9`. */
    readonly CHORD: 1;
    /** `GnoblinClaim.kind`: a touchpad gesture of `subject` kind with `fingers` contacts. */
    readonly GESTURE: 2;
    /** `GnoblinClaim.kind`: every key of the devices named by `device`, which then fire only the chords claimed for them. */
    readonly DEVICE: 3;
    /** `GnoblinClaim.kind`: every pointer button of the pointers named by `device`. */
    readonly POINTER: 4;
    /** `GnoblinClaim.kind`: the seat's pointer, keyboard or both (`subject`) for the shell node `node`. */
    readonly GRAB: 5;
    /** `GnoblinClaim.flags`: a chord claim withholds the key from clients and the seat's key state ("grab" in the schema). */
    readonly FLAG_GRAB: 1;
    /** `GnoblinClaimEvent.kind`: a claimed chord changed state. */
    readonly EVENT_CHORD: 1;
    /** `GnoblinClaimEvent.kind`: a claimed gesture advanced. */
    readonly EVENT_GESTURE: 2;
    /** `GnoblinClaimEvent.kind`: a claimed pointer chord's drag advanced. */
    readonly EVENT_POINTER_CHORD: 3;
    /** `GnoblinClaimEvent.kind`: a button of a claimed pointer went down or up. */
    readonly EVENT_POINTER_BUTTON: 4;
};
export type ClaimKind = (typeof ClaimKind)[keyof typeof ClaimKind];
/** `STREAM_*` constants of the ABI. */
export declare const StreamKind: {
    /** `GnoblinSpawn` stream mode: write to the compositor's own output, which the session log records. */
    readonly INHERIT: 0;
    /** `GnoblinSpawn` stream mode: drop the output. */
    readonly DISCARD: 1;
    /** `GnoblinSpawn` stream mode: keep the output, up to `output_limit` bytes, for the exit callback. */
    readonly CAPTURE: 2;
};
export type StreamKind = (typeof StreamKind)[keyof typeof StreamKind];
/** `PROVIDER_*` constants of the ABI. */
export declare const ProviderKind: {
    /** `provider_register` kind: the window manager that places and focuses windows while no shell holds the tree */
    readonly WM: 1;
    /** `provider_register` kind: the UI that shows portal dialogs (`GnoblinUiVtable`). At most one is live. */
    readonly UI: 2;
    /** `provider_register` kind: an observer of the accepted configuration document (`GnoblinConfigVtable`). At most one is live. */
    readonly CONFIG: 3;
};
export type ProviderKind = (typeof ProviderKind)[keyof typeof ProviderKind];
/** `WM_*` constants of the ABI. */
export declare const WmKind: {
    /** `GnoblinWmWindow.kind` and `GnoblinWmOpen.kind`: an application toplevel. */
    readonly KIND_NORMAL: 0;
    /** `GnoblinWmWindow.kind` and `GnoblinWmOpen.kind`: a dialog with a parent. */
    readonly KIND_DIALOG: 1;
    /** `GnoblinWmWindow.kind` and `GnoblinWmOpen.kind`: a dialog that blocks its parent. */
    readonly KIND_MODAL: 2;
    /** `GnoblinWmWindow.flags`: the window fills its output's work area. */
    readonly WINDOW_MAXIMISED: 1;
    /** `GnoblinWmWindow.flags`: the window covers its whole output. */
    readonly WINDOW_FULLSCREEN: 2;
    /** `GnoblinWmWindow.flags`: the window is hidden and out of the focus rotation. */
    readonly WINDOW_MINIMISED: 4;
    /** `GnoblinWmWindow.flags`: `output` names the output the window is on. */
    readonly WINDOW_HAS_OUTPUT: 8;
    /** `GnoblinWmWindow.flags` and `GnoblinWmOpen.flags`: `transient_for` names the window's parent. */
    readonly WINDOW_HAS_PARENT: 16;
    /** `GnoblinWmOpen.flags`: `requested` is the rectangle the client asked for. */
    readonly OPEN_HAS_REQUESTED: 32;
    /** `GnoblinWmStanding.flags`: `focus` names the focused window. */
    readonly STANDING_HAS_FOCUS: 1;
    /** `GnoblinWmEffect.kind`: place the window at `geometry` and tell its client `flags` (the `WM_WINDOW_*` state bits). */
    readonly EFFECT_CONFIGURE: 1;
    /** `GnoblinWmEffect.kind`: show the window (`flags` nonzero) or hide it. */
    readonly EFFECT_VISIBLE: 2;
    /** `GnoblinWmEffect.kind`: the paint order, topmost first, in `ids`. */
    readonly EFFECT_STACK: 3;
    /** `GnoblinWmEffect.kind`: move keyboard focus to `window` (`flags` nonzero) or to none. */
    readonly EFFECT_FOCUS: 4;
    /** A window manager call's result: the provider refused, because the window is already open. */
    readonly REFUSED_DUPLICATE: 1;
    /** A window manager call's result: the record is full. */
    readonly REFUSED_TOO_MANY: 2;
    /** A window manager call's result: no output is connected to place the window on. */
    readonly REFUSED_NO_OUTPUT: 3;
    /** A window manager call's result: the window is not open. */
    readonly REFUSED_GONE: 4;
    /** A window manager call's result: the window is maximised or fullscreen, so it has no floating rectangle. */
    readonly REFUSED_NOT_FLOATING: 5;
    /** A window manager call's result: the output is not connected. */
    readonly REFUSED_UNKNOWN_OUTPUT: 6;
    /** A window manager call's result: a transient moves with its parent, not on its own. */
    readonly REFUSED_FOLLOWS_PARENT: 7;
    /** `may_take_focus` policy: a new window may take focus. */
    readonly FOCUS_ALLOW: 0;
    /** `may_take_focus` policy: a new window takes focus only when nothing is focused or it is a transient of the focused one. */
    readonly FOCUS_PREVENT: 1;
};
export type WmKind = (typeof WmKind)[keyof typeof WmKind];
/** `UI_*` constants of the ABI. */
export declare const UiKind: {
    /** `GnoblinUiVtable.set_scheme`: egui's stock dark visuals. */
    readonly SCHEME_DARK: 0;
    /** `GnoblinUiVtable.set_scheme`: egui's stock light visuals. */
    readonly SCHEME_LIGHT: 1;
    /** `GnoblinUiVtable.submit` result: the dialog is open now. */
    readonly STARTED: 0;
    /** `GnoblinUiVtable.submit` result: admitted, waiting behind the open dialog. */
    readonly QUEUED: 1;
    /** `GnoblinUiVtable.submit` result: refused, with the portal error's number (`gnoblin_portal_core::wire::error_code`) */
    readonly REFUSED: 16;
};
export type UiKind = (typeof UiKind)[keyof typeof UiKind];
export declare const SHELL_FLAG_FOREIGN_LOOP = 1;
export declare const STATE_MAX_BYTES = 16777216;
export declare const ADOPTED = 1;
