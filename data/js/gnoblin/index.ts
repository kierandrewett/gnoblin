// The user-facing `gnoblin` module: `import { defineConfig, MasterStackLayout } from "gnoblin"`.
// It only re-exports. The native host objects (`gnoblin.clients`, Client, the listeners) are not here: a
// config reaches them through the `gnoblin` global or through `setup(gnoblin)`.

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
    StyleProperty,
} from "gnoblin/style";
export { Camera3D, Node, View } from "gnoblin/scene";
