// Public `gnoblin` package entry: this module owns only the export surface, not runtime objects.
// It re-exports bindings, layouts, helpers and stable data types.
// Host objects are reached through the `gnoblin` global.

export type { Rect } from "gnoblin/layout";
export type { Rect as BindingRect } from "gnoblin/bindings/types";
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
} from "gnoblin/bindings/binding";
export { claimOf } from "gnoblin/bindings/claim";
export { clampAxis, constrainRect, DRAG_STRIP, dragConstraintMode } from "gnoblin/bindings/drag";
export { ClientMoveBinding } from "gnoblin/bindings/move";
export { ClientResizeBinding } from "gnoblin/bindings/resize";
export { ClientSwapBinding, SplitResizeBinding } from "gnoblin/bindings/tiling";
export { parseTrigger, TriggerError } from "gnoblin/bindings/trigger";
export type { BindingWatcher, KeyBindingOptions, PointerDragBindingOptions } from "gnoblin/bindings/binding";
export type { TriggerClaim } from "gnoblin/bindings/claim";
export type { DragConstraint } from "gnoblin/bindings/drag";
export type { ClientMoveOptions, MoveState } from "gnoblin/bindings/move";
export type { ClientResizeOptions, ResizeCorner, ResizeEdge, ResizeState } from "gnoblin/bindings/resize";
export type { LayoutSplit, OrderedWorkspace, SplitState, SwapState, TiledClient } from "gnoblin/bindings/tiling";
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
} from "gnoblin/bindings/trigger_types";
export type { BindingClient, BindingEvent, BindingOptions, DragEvent, Point } from "gnoblin/bindings/types";
export * from "gnoblin/focus";
export { InputContext } from "gnoblin/input/context";
export { InputManager } from "gnoblin/input/manager";
export type { InputContextOptions } from "gnoblin/input/context";
export type { InputHit, InputHost, InputLayout, InputWorld, SeatSettings } from "gnoblin/input/host";
export type { InputChordMessage, InputGestureMessage, InputMessage, InputPointerMessage } from "gnoblin/input/messages";
export type { InputBindingInfo } from "gnoblin/input/registration";
export * from "gnoblin/layout";
export * from "gnoblin/layouts/floating";
export * from "gnoblin/layouts/grid";
export * from "gnoblin/layouts/master-stack";
export * from "gnoblin/layouts/monocle";
export * from "gnoblin/layouts/dwindle";
export * from "gnoblin/layouts/kiosk";
export * from "gnoblin/layouts/arc";
export * from "gnoblin/layouts/strip";
export * from "gnoblin/gestures";
export * from "gnoblin/gesture_actions";
export * from "gnoblin/inspect";
export * from "gnoblin/snap";
export * from "gnoblin/dialogs";
export * from "gnoblin/authentication";
export * from "gnoblin/idle";
export * from "gnoblin/launches";
export * from "gnoblin/titlebar";
export * from "gnoblin/ui";
export * from "gnoblin/widgets";
export { MenuItem, MenuPanel, WindowMenu } from "gnoblin/menus/card";
export { MENU_COLOURS, menuHeight, placeCard } from "gnoblin/menus/metrics";
export { menuKeyAction, MenuNavigator } from "gnoblin/menus/navigator";
export { defaultMenuItems, windowMenu, WindowMenus } from "gnoblin/menus/windows";
export type { MenuKeyAction, NavRow } from "gnoblin/menus/navigator";
export type { MenuClient, MenuEntry, MenuWorkspace, MenuWorld } from "gnoblin/menus/types";
export * from "gnoblin/alerts";
export * from "gnoblin/launcher";
export * from "gnoblin/overview";
export * from "gnoblin/panel";
export * from "gnoblin/window_state";
export * from "gnoblin/reposition";
// `gnoblin.requestTransition` and what it returns. The engine and the staging stay with the host.
export { Transition } from "gnoblin/transition/transition";
export type { Easing } from "gnoblin/transition/easing";
export type { TransitionOptions, TransitionResult, TransitionSubscription } from "gnoblin/transition/types";
// The host API of `gnoblin.session`, `gnoblin.process`, `gnoblin.timers` and `gnoblin.on`: the classes a script holds
// and the shapes it reads. The module that builds them (`HostApi`) stays with the host.
export { Operation, Timer } from "gnoblin/operation";
export type { OperationError, OperationStatus, Subscription } from "gnoblin/operation";
export type { ErrorCode } from "gnoblin/errors";
export { Process, ProcessListener } from "gnoblin/process";
export type { ProcessExit, ProcessSignal, SpawnOptions, StreamMode } from "gnoblin/process";
export type { FocusHistoryFilter } from "gnoblin/session/focus_history";
export type {
    Accepted,
    LockRequest,
    LockState,
    ReloadResult,
    SessionStatus,
    SignalEvent,
    Version,
} from "gnoblin/session/types";
// The style classes and their list elements. The cascade's plumbing (`bindStyle`, `resolveStyle`, ...) stays in the
// module for the host.
export { Border, Shadow } from "gnoblin/style/border_shadow";
export { StyleElement } from "gnoblin/style/element";
export { BlurFilter, BrightnessFilter, ContrastFilter, SaturateFilter } from "gnoblin/style/filters";
export { ClientStyle, Style } from "gnoblin/style/style";
export { MAX_STYLE_LIST } from "gnoblin/style/values";
export type { BorderInit, BorderLike, ShadowInit, ShadowLike } from "gnoblin/style/border_shadow";
export type { Filter, FilterInit } from "gnoblin/style/filters";
export type { StyleList, StyleProperty } from "gnoblin/style/lists";
export type { BorderValue, ComputedStyle, FilterValue, ShadowValue } from "gnoblin/style/resolve";
export type { ClientStyleInit } from "gnoblin/style/style";
export type { CornerRadii } from "gnoblin/style/values";
// Logical displays (owner-spec sections 8 and 9). The manager (`gnoblin/display_manager`) stays with the host.
export { Camera2D, Display, OutputProfile, Viewport } from "gnoblin/display";
export { Camera3D } from "gnoblin/scene/camera";
export { Node } from "gnoblin/scene/node";
export { Scene } from "gnoblin/scene/scene";
export { View } from "gnoblin/scene/view";
export type { InputMode, NodeOptions, NodePointerEvent } from "gnoblin/scene/node";
export type { Size2D, ViewOptions } from "gnoblin/scene/view";
export type { ContentFit } from "gnoblin/scene_mirror";
export type { PropertyBinding, PropertyBindings } from "gnoblin/properties";
export type { RotationInit, ScaleInit, TransformInit, Vec3Like } from "gnoblin/spatial";
export { LockScreen, PasswordEntry, defaultLockStyle, formatClock, lockScreen } from "gnoblin/lock";
export type { LockAnswer, LockBackend, LockStyle } from "gnoblin/lock";
// The device classes of a seat and the base class of the shell's objects. The settings ledger stays with the host.
export { InputDevice, Keyboard, Mouse, Tablet, Touchpad, Touchscreen } from "gnoblin/devices";
export type { DeviceBindOptions } from "gnoblin/devices";
export { NativeObject } from "gnoblin/native_object";
export { VirtualOutput } from "gnoblin/virtual_output";
export type { VirtualOutputAttachment, VirtualOutputOptions } from "gnoblin/virtual_output";
