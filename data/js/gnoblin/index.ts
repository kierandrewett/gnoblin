// Public `gnoblin` package entry: this module owns only the export surface, not runtime objects.
// It re-exports configuration, bindings, layouts, helpers and stable data types.
// Host objects are reached through the `gnoblin` global or `setup(gnoblin)`.
// Read `config.ts` first for setup and the distinction between public and host APIs.

export * from "gnoblin/bindings";
export * from "gnoblin/focus";
export * from "gnoblin/input";
export * from "gnoblin/layout";
export * from "gnoblin/layouts/floating";
export * from "gnoblin/layouts/grid";
export * from "gnoblin/layouts/master-stack";
export * from "gnoblin/layouts/monocle";
export * from "gnoblin/layouts/dwindle";
export * from "gnoblin/layouts/kiosk";
export * from "gnoblin/layouts/strip";
export * from "gnoblin/sets";
export * from "gnoblin/gestures";
export * from "gnoblin/gesture_actions";
export * from "gnoblin/inspect";
export * from "gnoblin/config";
export * from "gnoblin/rules";
export * from "gnoblin/snap";
export * from "gnoblin/dialogs";
export * from "gnoblin/authentication";
export * from "gnoblin/idle";
export * from "gnoblin/launches";
export * from "gnoblin/titlebar";
export * from "gnoblin/ui";
export * from "gnoblin/widgets";
export * from "gnoblin/menus";
export * from "gnoblin/alerts";
export * from "gnoblin/launcher";
export * from "gnoblin/overview";
export * from "gnoblin/panel";
export * from "gnoblin/window_state";
export * from "gnoblin/reposition";
// `gnoblin.requestTransition` and what it returns. The engine and the staging stay with the host.
export { Transition } from "gnoblin/transition";
export type { TransitionEasing, TransitionOptions, TransitionResult, TransitionSubscription } from "gnoblin/transition";
// The host API of `gnoblin.session`, `gnoblin.process`, `gnoblin.timers` and `gnoblin.on`: the classes a script holds
// and the shapes it reads. The module that builds them (`HostApi`) stays with the host.
export { Operation, Timer } from "gnoblin/operation";
export type { OperationError, OperationErrorCode, OperationStatus, Subscription } from "gnoblin/operation";
export { Process, ProcessListener } from "gnoblin/process";
export type { ProcessExit, ProcessSignal, SpawnOptions, StreamMode } from "gnoblin/process";
export type {
    Accepted,
    FocusHistoryFilter,
    LockRequest,
    LockState,
    ReloadResult,
    SessionStatus,
    SignalEvent,
    Version,
} from "gnoblin/session";
// The style classes and their list elements. The cascade's plumbing (`bindStyle`, `resolveStyle`, ...) stays in the
// module for the host.
export {
    Border,
    BlurFilter,
    BrightnessFilter,
    ClientStyle,
    ContrastFilter,
    MAX_STYLE_LIST,
    SaturateFilter,
    Shadow,
    Style,
    StyleElement,
} from "gnoblin/style";
export type {
    BorderInit,
    BorderLike,
    BorderValue,
    ClientStyleInit,
    ComputedStyle,
    CornerRadii,
    Filter,
    FilterInit,
    FilterValue,
    ShadowInit,
    ShadowLike,
    ShadowValue,
    StyleList,
    StyleProperty,
} from "gnoblin/style";
export { Camera3D, Node, View } from "gnoblin/scene";
// Logical displays (owner-spec sections 8 and 9). The manager (`gnoblin/display_manager`) stays with the host.
export { Camera2D, Display, OutputProfile, Viewport } from "gnoblin/display";
export { LockScreen, PasswordEntry, defaultLockStyle, formatClock, lockScreen } from "gnoblin/lock";
export type { LockAnswer, LockBackend, LockStyle } from "gnoblin/lock";
// The device classes of a seat and the base class of the shell's objects. The settings ledger stays with the host.
export { InputDevice, Keyboard, Mouse, Tablet, Touchpad, Touchscreen } from "gnoblin/devices";
export type { DeviceBindOptions } from "gnoblin/devices";
export { NativeObject } from "gnoblin/native_object";
export { VirtualOutput } from "gnoblin/virtual_output";
export type { VirtualOutputAttachment, VirtualOutputOptions } from "gnoblin/virtual_output";
