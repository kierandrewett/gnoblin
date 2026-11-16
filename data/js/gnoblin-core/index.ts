// The `gnoblin-core` package entry: the machinery a shell stands on, with no desktop policy in it. It exports the
// object model (clients, outputs, collections, listeners), the input contexts and binding classes, the host calls
// (`process`, `session`, timers), the dialogs, idle and authentication services, and the types between them. A shell
// is complete on these alone; workspaces, layouts, animation, styling and the rest are `gnoblin-std`.
// The runtime objects (`gnoblin`, `Client`, `Collection`, ...) come from `gnoblin-core/host`, which the shell appends
// to this module when it embeds it. A module that only needs the shapes imports them from here without a host.

export type { Query, Rect } from "gnoblin-core/types";
export type { Rect as BindingRect } from "gnoblin-core/bindings/types";
export {
    Binding,
    BindingCollection,
    BindingConflictError,
    checkTrigger,
    DragBinding,
    GestureBinding,
    KeyBinding,
    PointerBinding,
    PointerDragBinding,
    resolveTriggers,
} from "gnoblin-core/bindings/binding";
export { claimOf } from "gnoblin-core/bindings/claim";
export { parseTrigger, TriggerError } from "gnoblin-core/bindings/trigger";
export type { BindingWatcher, KeyBindingOptions, PointerDragBindingOptions } from "gnoblin-core/bindings/binding";
export type { TriggerClaim } from "gnoblin-core/bindings/claim";
export type {
    ControllerButton,
    ControllerTriggerParts,
    GestureTrigger,
    GestureTriggerParts,
    KeyTrigger,
    KeyTriggerParts,
    ParsedTrigger,
    PointerTrigger,
    PointerTriggerParts,
    Trigger,
    TriggerButton,
    TriggerDirection,
    TriggerModifier,
    TriggerModifierName,
    WheelTrigger,
    WheelTriggerParts,
} from "gnoblin-core/bindings/trigger_types";
export type { BindingClient, BindingEvent, BindingOptions, DragEvent, Point } from "gnoblin-core/bindings/types";
export * from "gnoblin-core/input";
export * from "gnoblin-core/inspect";
export * from "gnoblin-core/dialogs";
export * from "gnoblin-core/authentication";
export * from "gnoblin-core/idle";
export * from "gnoblin-core/launches";
export { growRect, isNoInset, NO_INSET, sameInset, shrinkRect } from "gnoblin-core/inset";
export type { FrameInset } from "gnoblin-core/inset";
// The host API of `gnoblin.session`, `gnoblin.runtime`, `gnoblin.process` and `gnoblin.timers`: the classes a script
// holds and the shapes it reads. The module that builds them (`HostApi`) stays with the host.
export { Operation, Timer } from "gnoblin-core/operation";
export type { OperationError, OperationStatus, Subscription } from "gnoblin-core/operation";
export type { ErrorCode } from "gnoblin-core/errors";
export { Process, ProcessListener } from "gnoblin-core/process";
export type { ProcessExit, ProcessSignal, SpawnOptions, StreamMode } from "gnoblin-core/process";
export type { FocusHistoryFilter } from "gnoblin-core/session/focus_history";
export { RuntimeListener, SessionListener } from "gnoblin-core/session/events";
export type { ConfigReloaded, ConfigReloadFailed } from "gnoblin-core/session/events";
export type { ListenerHost, Prefixed, Subscribe, Subscriptions, Unsubscribe } from "gnoblin-core/subscriptions";
export type {
    Accepted,
    LockRequest,
    LockState,
    ReloadResult,
    SessionStatus,
    Version,
} from "gnoblin-core/session/types";
export type { PropertyBinding, PropertyBindings } from "gnoblin-core/properties";
export type { RotationInit, ScaleInit, TransformInit, Vec3Like } from "gnoblin-core/math/spatial";
export type { LockAnswer, LockBackend } from "gnoblin-core/lock";
// The device classes of a seat and the base class of the shell's objects. The settings ledger stays with the host.
export { InputDevice, Keyboard, Mouse, Tablet, Touchpad, Touchscreen } from "gnoblin-core/devices";
export type { DeviceBindOptions } from "gnoblin-core/devices";
export { NativeObject } from "gnoblin-core/native_object";
export { VirtualOutput } from "gnoblin-core/virtual_output";
export type { VirtualOutputAttachment, VirtualOutputOptions } from "gnoblin-core/virtual_output";
