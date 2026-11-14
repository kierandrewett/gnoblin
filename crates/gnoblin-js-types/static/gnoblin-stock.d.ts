// ---------------------------------------------------------------------------
// The stock library: `import { ... } from "gnoblin"`.
//
// PROPOSED API. This half of the file is written by hand, not generated. It is
// the object model of design/js-research/owner-spec.md (sections 4, 5, 11, 12)
// plus the simple tier of design/js-research/spec-review.md (section 4), and
// no runtime implements it yet. A declaration here is a proposal until the
// stock library ships it; the generated modules above are the part that
// follows the schema.
// ---------------------------------------------------------------------------

declare module "gnoblin" {
    // -- Geometry and transforms ---------------------------------------------

    export interface Vec2 {
        x: number;
        y: number;
    }
    export interface Vec3 {
        x: number;
        y: number;
        z: number;
    }
    /** Rotation about each axis, in radians. */
    export type Euler3 = Vec3;
    export interface Quaternion {
        x: number;
        y: number;
        z: number;
        w: number;
    }
    export interface Size2D {
        width: number;
        height: number;
    }
    export interface Rect extends Vec2, Size2D {}
    export interface Transform3D {
        position: Vec3;
        rotation: Quaternion | Euler3;
        scale: number | Vec3;
    }

    // -- Native objects (owner-spec section 4) --------------------------------

    /**
     * Every native object has a stable id and an explicit lifetime. Wrappers for the
     * same native id compare equal within a runtime generation. A destroyed object
     * reads `alive === false`, and a write or method call on it rejects.
     */
    export abstract class NativeObject {
        readonly id: string;
        readonly alive: boolean;
        /** Write several properties at once, optionally as one timed transition. */
        set(properties: object, options?: TransitionOptions): Transition | void;
        addListener(listener: object): void;
        removeListener(listener: object): void;
    }

    export class Node extends NativeObject {
        readonly parent: Node | null;
        readonly children: Collection<Node>;
        transform: Transform3D;
        /** Clamped to `[0, 1]`. */
        opacity: number;
        visible: boolean;
        style: Style;
        /** Set on layout-bearing containers such as a `Workspace`. */
        layout?: Layout | null;
        /** Convenience accessor for `transform.position`. */
        position: Vec3;
        /** Convenience accessor for `transform.rotation`. */
        rotation: Quaternion | Euler3;
        /** Convenience accessor for `transform.scale`. */
        scale: number | Vec3;
        add(node: Node): void;
        remove(node: Node): void;
        destroy(): void;
    }

    /** A managed Wayland toplevel (not a `wl_client` connection). */
    export class Client extends Node {
        readonly appId: string;
        readonly title: string;
        readonly committedSize: Size2D;
        /** The logical size the client is asked to take (its configure request). */
        size: Size2D;
        /** The geometry the layout gave the client. */
        bounds: Rect;
        readonly workspace: Workspace | null;
        /** Where the user left the client in a floating layout. */
        floatingBounds?: Rect;
        /** Whether the window covers its output. Writing asks the compositor to make it fullscreen or restore it. */
        fullscreen: boolean;
        /** From 0 to 1; the compositor clamps a value outside that. */
        opacity: number;
        /** Whether the window is kept out of screen captures. */
        secure: boolean;
        /** A floating client is left out of its workspace's tiling and keeps its place. Only the script knows it. */
        floating: boolean;
        style: ClientStyle;
        readonly computedStyle: Readonly<ClientStyle>;
        /** Whether the window sits in a tiled layout, so its client drops the border and rounding it draws itself. */
        tiled: boolean;
        /** The window's place in depth; negative is in front of the camera. */
        z: number;
        /**
         * Moves properties to `values` over `options.duration` seconds along the easing curve; reading one back at once
         * gives the end value. A duration of 0 writes at once.
         */
        animate(values: ClientAnimation, options: AnimateOptions): void;
        close(): void;
        /** Puts the window on top of every other window. */
        raise(): void;
        /** Puts the window under every other window. */
        lower(): void;
        /** Kills the process that owns the window (SIGKILL): the not-responding alert's Force quit. The compositor may refuse. */
        kill(): void;
        /** Whether the client has left a liveness ping unanswered too long. */
        readonly unresponsive: boolean;
        /** Whether the window is maximised. Writing asks the compositor to maximise or restore it. */
        maximised: boolean;
        minimised: boolean;
        /** The room a titlebar takes above the content: `bounds` is the content, the frame is the content with this on top. */
        readonly frameInset: number;
    }

    /** Rotation about each axis in radians, or the scale along each axis: three numbers. */
    export type Vector3 = readonly [number, number, number];

    /** The curves `Client.animate` knows, named as the Lua shell's `ease` table named them. */
    export type ClientEasing = "linear" | "out" | "out-cubic" | "in-cubic" | "in-out";

    export interface ClientAnimation {
        x?: number;
        y?: number;
        width?: number;
        height?: number;
        opacity?: number;
        z?: number;
        rotation?: Vector3;
        scale?: Vector3;
    }

    export interface AnimateOptions {
        /** Seconds. Zero is an instant write. */
        duration: number;
        /** Default `"out"`. */
        easing?: ClientEasing;
    }

    /** A scene container that owns a layout and holds clients. */
    export class Workspace extends Node {
        readonly clients: Collection<Client>;
        /** Inner and outer gap that every stock layout reads from `LayoutContext`. */
        gaps: Gaps;
        constructor(options: WorkspaceOptions);
    }

    export interface WorkspaceOptions {
        id: string;
        layout?: Layout | LayoutName;
        gaps?: Partial<Gaps>;
    }

    export interface Gaps {
        inner: number;
        outer: number;
    }

    // -- Styles ---------------------------------------------------------------

    export interface Shadow {
        x?: number;
        y?: number;
        blur?: number;
        spread?: number;
        /** `#rgb`, `#rrggbb` or `#rrggbbaa`. */
        color?: string;
    }
    export interface Style {
        opacity?: number;
        cornerRadius?: number;
        shadows?: Shadow[];
    }
    export interface ClientStyle extends Style {
        borderWidth?: number;
        borderColor?: string;
    }

    // -- Collections (owner-spec section 5) -----------------------------------

    /**
     * A live, read-mostly set of native objects. It is not a JS `Array`: membership
     * of `gnoblin.clients` belongs to the compositor, and `add`/`remove` exist only
     * on explicitly mutable collections such as `gnoblin.workspaces`.
     */
    export interface Collection<T> extends Iterable<T> {
        readonly length: number;
        at(index: number): T | undefined;
        get(id: string): T | undefined;
        find(predicate: (item: T) => boolean): T | undefined;
        /** A snapshot array. */
        filter(predicate: (item: T) => boolean): T[];
        /** A snapshot array. */
        map<U>(mapper: (item: T) => U): U[];
        /** A live selection. The query is plain data so the compositor can evaluate it. */
        where(query: Query<T>): Collection<T>;
        addListener(listener: CollectionListener<T> | MemberListener<T>): void;
        removeListener(listener: object): void;
    }

    /** A collection that accepts membership changes from scripts. */
    export interface MutableCollection<T> extends Collection<T> {
        add(item: T): void;
        remove(idOrItem: string | T): void;
        clear(): void;
    }

    /** Data-only match on a member's properties; each value is compared with `===`. */
    export type Query<T> = { [K in keyof T]?: T[K] extends Function ? never : T[K] };

    // -- Listeners (owner-spec section 4) -------------------------------------

    /**
     * Observes membership changes of a collection. Every method gets the object it
     * is about as its first argument.
     */
    export class CollectionListener<T = Client> {
        onAdded?(item: T): void;
        onRemoved?(item: T): void;
    }

    /** Observes member events; attach to one object, or to a collection for all of its members. */
    export class MemberListener<T = Client> {
        onDestroyed?(item: T): void;
    }

    /** A pointer edge on a window, in the window's own coordinates; `button` is 0 for every edge but a press and a release. */
    export interface ClientPointerEvent {
        readonly x: number;
        readonly y: number;
        readonly button: number;
    }

    export class ClientListener extends MemberListener<Client> {
        onTitleChanged?(client: Client, title: string): void;
        onAppIdChanged?(client: Client, appId: string): void;
        /** The pointer crossed into the window. */
        onPointerEnter?(client: Client, event: ClientPointerEvent): void;
        onPointerMotion?(client: Client, event: ClientPointerEvent): void;
        onPointerLeave?(client: Client, event: ClientPointerEvent): void;
        onPointerPress?(client: Client, event: ClientPointerEvent): void;
        onPointerRelease?(client: Client, event: ClientPointerEvent): void;
        onFocusChanged?(client: Client, focused: boolean): void;
        onGeometryChanged?(client: Client, geometry: Rect): void;
        onClosed?(client: Client): void;
    }

    export class OutputListener extends MemberListener<Output> {
        onModeChanged?(output: Output, mode: string): void;
        onScaleChanged?(output: Output, scale: number): void;
        onConnectionChanged?(output: Output, connected: boolean): void;
    }

    /** A connected physical or virtual output: read-only capabilities and applied state. */
    export class Output extends NativeObject {
        readonly connector: string;
        readonly mode: string;
        readonly scale: number;
        readonly position: Vec2;
        readonly connected: boolean;
    }

    // -- Triggers and bindings (owner-spec sections 10 and 11) ----------------

    /** The modifiers a canonical trigger writes, in this order. */
    export type Modifier = "Shift" | "Ctrl" | "Alt" | "Meta" | "Super" | "Hyper";
    /** Every spelling of a modifier a trigger accepts (`Control`, `Primary`, `Mod1`, `Logo` are aliases). */
    export type ModifierName = Modifier | "Control" | "Primary" | "Mod1" | "Logo";
    type ModifierPrefix =
        | ""
        | `${ModifierName}+`
        | `${ModifierName}+${ModifierName}+`
        | `${ModifierName}+${ModifierName}+${ModifierName}+`;
    export type MouseButton = "MouseLeft" | "MouseMiddle" | "MouseRight";
    /** `Super+MouseLeft+Drag` drags; `Super+MouseLeft` and `Super+MouseLeft+Click` click. */
    export type PointerTrigger =
        `${ModifierPrefix}${MouseButton}` | `${ModifierPrefix}${MouseButton}+${"Drag" | "Click"}`;
    /** Parsed, but the compositor has no wheel claim yet, so binding one throws. */
    export type WheelTrigger = `${ModifierPrefix}Wheel${"" | "Up" | "Down" | "Left" | "Right"}`;
    /** A touchpad gesture with 2 to 5 fingers, optionally a direction: `Swipe3Left`, `Pinch4In`. */
    export type GestureTrigger =
        | `Swipe${2 | 3 | 4 | 5}`
        | `Swipe${2 | 3 | 4 | 5}${"Left" | "Right" | "Up" | "Down"}`
        | `Pinch${2 | 3 | 4 | 5}`
        | `Pinch${2 | 3 | 4 | 5}${"In" | "Out"}`;
    /**
     * Modifiers then one key, by keysym name and never by the character it types: `Super+1` and `Super+Shift+1`
     * are two triggers, and `Super+!` is refused. The key names are the compositor's, so the type cannot list them.
     */
    export type KeyTrigger = `${ModifierPrefix}${string}`;
    export type Trigger = PointerTrigger | WheelTrigger | GestureTrigger | KeyTrigger;

    interface TriggerBase {
        /** The canonical spelling. Two spellings of one trigger have one `text`. */
        readonly text: string;
        readonly modifiers: readonly Modifier[];
    }
    export type ParsedTrigger =
        | (TriggerBase & { readonly kind: "key"; readonly key: string })
        | (TriggerBase & { readonly kind: "pointer"; readonly button: MouseButton; readonly action: "click" | "drag" })
        | (TriggerBase & { readonly kind: "wheel"; readonly direction: "up" | "down" | "left" | "right" | null })
        | (TriggerBase & {
              readonly kind: "gesture";
              readonly gesture: "swipe" | "pinch";
              readonly fingers: number;
              readonly direction: "left" | "right" | "up" | "down" | "in" | "out" | null;
          });
    /** Throws a `TriggerError` naming the token that is wrong. */
    export function parseTrigger(trigger: string, what?: string): ParsedTrigger;
    export class TriggerError extends TypeError {
        readonly token: string;
        readonly trigger: string;
    }
    /** Two bindings of one collection match one trigger at one priority. */
    export class BindingConflictError extends Error {
        readonly trigger: string;
        readonly ids: readonly [string, string];
    }

    export interface BindingOptions {
        id: string;
        trigger: Trigger;
        enabled?: boolean;
        /** Higher wins inside one tier (device, then layout, then context). */
        priority?: number;
    }

    export class Binding {
        readonly id: string;
        /** Assigning a new trigger moves the binding and cancels a drag it runs. */
        trigger: Trigger;
        enabled: boolean;
        priority: number;
        readonly parsed: ParsedTrigger;
        constructor(options: BindingOptions);
        /** Takes the binding out of the collection that holds it. */
        unbind(): void;
        onStart(event: DragStartEvent): object | false;
        onUpdate(event: DragUpdateEvent, state: object): void;
        onEnd(event: DragEndEvent, state: object): void;
        onCancel(event: DragCancelEvent, state: object): void;
        /** A key press or a click. Return `false` to let the next tier have it. */
        onPress?(event: BindingEvent): void | false;
    }
    export interface KeyBindingOptions extends BindingOptions {
        action?: (event: BindingEvent) => void | false;
    }
    export class KeyBinding extends Binding {
        constructor(options: KeyBindingOptions);
    }
    export class PointerBinding extends KeyBinding {}
    export class GestureBinding extends Binding {}

    export interface BindingEvent {
        readonly timestamp: number;
        readonly [field: string]: unknown;
    }
    export interface DragEvent extends BindingEvent {
        readonly context: InputContext;
        /** The device the chord came from when the claim was for one device; null for every device. */
        readonly device: InputDevice | null;
        /** The client under the pointer at the press (a key press: the focused client). */
        readonly client?: Client;
        /** The layout whose binding this is, for a layout binding. */
        readonly layout?: Layout;
        /** The pointer in compositor logical coordinates. A drag's start event holds the press position. */
        readonly position?: Vec2;
        /** Coordinates relative to the hit client's origin. */
        readonly localPosition?: Vec2;
        /** Seconds since the previous event of this gesture, at most 0.1. */
        readonly deltaTime: number;
        readonly modifiers: ReadonlySet<string>;
        /** The canonical trigger of the binding. */
        readonly trigger: string;
        /** Pointer drags only: the pointer, the same as `position`. */
        readonly x?: number;
        readonly y?: number;
        /** Pointer drags only: the button, X11 numbering (1 left, 2 middle, 3 right). */
        readonly button?: number;
        /** Pointer drags only: the rectangle a dragged window stays in, and the seat's `drag_constraint`. */
        readonly workArea?: Rect;
        readonly dragConstraint?: string;
        /** Pointer drags only: the seat's `edge_tiling`, or the config's `edgeTiling`: whether a drop on an edge tiles the window. */
        readonly edgeTiling?: boolean;
        /** Gestures only: `begin`, `update`, `end` or `cancel`. */
        readonly phase?: string;
        /** Gestures only: movement since the begin in touchpad units, the spread (1 is unchanged) and the rotation in radians. A pointer drag's `dx` and `dy` are pixels from the press. */
        readonly dx?: number;
        readonly dy?: number;
        readonly scale?: number;
        readonly rotation?: number;
        /** Gestures only: 0 to 1, how far the gesture has gone its own way. */
        readonly progress?: number;
    }
    export type DragStartEvent = DragEvent;
    export type DragUpdateEvent = DragEvent;
    export type DragEndEvent = DragEvent;
    export type DragCancelEvent = DragEvent;

    /**
     * `onStart` returns a fresh per-gesture state object, or `false` to decline. Each later callback receives that
     * state, so one binding instance can serve two input contexts at once. A drag starts once the pointer has gone
     * `threshold` pixels from the press. Pointer events reach the shell once per frame, and when scripts are behind
     * the next `onUpdate` reads the latest position; `onStart`, `onEnd` and `onCancel` are never dropped.
     */
    export abstract class DragBinding<S extends object = object> extends Binding {
        onStart(event: DragStartEvent): S | false;
        onUpdate(event: DragUpdateEvent, state: S): void;
        onEnd(event: DragEndEvent, state: S): void;
        onCancel(event: DragCancelEvent, state: S): void;
    }
    export interface PointerDragBindingOptions extends BindingOptions {
        /** Pixels the pointer must move from the press before `onStart`. Default 4. */
        threshold?: number;
    }
    export class PointerDragBinding<S extends object = object> extends DragBinding<S> {
        threshold: number;
        constructor(options: PointerDragBindingOptions);
    }
    /** How a dragged window is kept in the work area: inside it, or with a strip of it inside. */
    export type DragConstraint = "work_area" | "minimum_strip";
    /** The visible strip of a window kept inside the work area under `minimum_strip`: 75 pixels. */
    export const DRAG_STRIP: number;
    /** The constraint a seat's `drag_constraint` names; undefined and null are `work_area`, any other value throws. */
    export function dragConstraintMode(value: unknown): DragConstraint;
    /** One axis: a window of extent `size` at `pos` against an area that starts at `start` and is `span` long. */
    export function clampAxis(pos: number, size: number, start: number, span: number, mode: DragConstraint): number;
    /** `rect` moved to the place `mode` allows inside `area`, keeping its size. */
    export function constrainRect(rect: Rect, area: Rect, mode: DragConstraint): Rect;
    /** Super+MouseLeft drag: the client follows the pointer. */
    export class ClientMoveBinding extends PointerDragBinding {
        /** How near a screen edge, in pixels, a drop tiles the window (default 4); `false` never tiles. Needs edge tiling on. */
        snap: number | false;
        constructor(options?: Partial<PointerDragBindingOptions> & { snap?: number | false });
    }
    export type ResizeCorner = "top-left" | "top-right" | "bottom-left" | "bottom-right";
    /** The one side that moves with the pointer when `edges` is `nearest`. */
    export type ResizeEdge = "left" | "right" | "top" | "bottom";
    /**
     * Super+MouseRight drag: the corner chosen at the press follows the pointer; it never changes mid-drag. With
     * `edges: "nearest"` the side nearest the press moves alone.
     */
    export class ClientResizeBinding extends PointerDragBinding {
        edges: "quadrant" | "nearest" | ResizeCorner;
        constructor(options?: Partial<PointerDragBindingOptions> & { edges?: "quadrant" | "nearest" | ResizeCorner });
    }
    export class ClientSwapBinding extends PointerDragBinding {
        constructor(options?: Partial<BindingOptions>);
    }
    export class SplitResizeBinding extends PointerDragBinding {
        constructor(options?: Partial<BindingOptions>);
    }
    export class WorkspaceSwitchBinding extends GestureBinding {}

    /** The bindings of one layout, device or context, by id. */
    export class BindingCollection implements Iterable<Binding> {
        readonly size: number;
        /** Refuses a taken id, a binding that is in another collection and an equal-priority match. */
        add(binding: Binding): Binding;
        get(id: string): Binding | undefined;
        has(id: string): boolean;
        remove(id: string): boolean;
        clear(): void;
        [Symbol.iterator](): Iterator<Binding>;
    }

    export interface InputDevice {
        readonly id: string;
        readonly name: string;
        readonly capabilities: readonly string[];
        readonly context: InputContext;
        /** Bindings that match only this device's events. They run before a layout's and the context's. */
        readonly bindings: BindingCollection;
        bind(trigger: Trigger, action: ((event: BindingEvent) => void | false) | Binding): Binding;
    }
    /** The window management settings a seat publishes (the schema's `seat` keys in camel case, the compositor's words). */
    export interface SeatSettings {
        readonly edgeTiling: boolean;
        /** `click`, `hover` or `hover-strict`. */
        readonly focusMode: string;
        /** `allow` or `prevent`. */
        readonly focusNewWindows: string;
        readonly raiseOnClick: boolean;
        readonly autoRaise: boolean;
        readonly autoRaiseDelay: number;
        readonly focusChangeOnPointerRest: boolean;
    }
    /** One seat: its devices, its focus, its pointer and the bindings that run on its chords and drags. */
    export interface InputContext {
        readonly name: string;
        readonly devices: readonly InputDevice[];
        readonly bindings: BindingCollection;
        /** Read through to the seat on every call. */
        readonly focusedClient: Client | null;
        /** Not wired: displays have no JavaScript object yet. */
        readonly activeDisplay: null;
        readonly pointer: Vec2;
        /** What the seat publishes, with the config's `edgeTiling` and `dragConstraint` over it. */
        readonly settings: SeatSettings;
        /** Takes the keyboard focus from every window. */
        clearFocus(): void;
        /**
         * Claims the chord on the seat and runs `action` on it. Returns the binding; `unbind()` releases the
         * claim once no binding needs it. A drag trigger needs a `PointerDragBinding`, not a function.
         */
        bind(trigger: Trigger, action: ((event: BindingEvent) => void | false) | Binding): Binding;
        /** The topmost client under `position`, in tree order. */
        hitTest(position: Vec2): { client: Client; localPosition: Vec2 } | null;
        /**
         * Makes a layout's bindings live for the clients `owns` accepts (default: those whose workspace's layout it
         * is). The layouts of the workspaces are attached by the runtime; this is for any other.
         */
        attachLayout(layout: Layout, owns?: (client: Client) => boolean): void;
        detachLayout(layout: Layout): void;
    }
    export interface InputManager {
        readonly contexts: readonly InputContext[];
        /** The first seat. Throws when the compositor reports none. */
        readonly primary: InputContext;
        readonly devices: readonly InputDevice[];
    }

    // -- Layouts (owner-spec section 12) --------------------------------------

    export type LayoutName =
        "floating" | "master-stack" | "grid" | "monocle" | "dwindle" | "kiosk" | "strip" | "carousel-3d";

    export interface LayoutContext {
        workspace: Workspace;
        clients: readonly Client[];
        children: readonly Node[];
        area: Rect;
        focused: Client | null;
        /** Proposed: supplied by the workspace so one setting applies to every layout. */
        gap: Gaps;
    }

    export interface Placement {
        node: Node;
        bounds?: Rect;
        transform?: Transform3D;
        visible?: boolean;
        tiled?: boolean;
        /** Depth: negative is in front of the camera. Default 0. */
        z?: number;
        /** Rotation about each axis in radians. Default none. */
        rotation?: Vector3;
        /** Scale along each axis. Default 1. */
        scale?: Vector3;
    }

    /**
     * Subclass it and override the hooks you need. `compute` is called on invalidation,
     * never every frame; a throw leaves the last valid placement in place.
     */
    export class Layout extends NativeObject {
        readonly workspace: Workspace;
        readonly bindings: BindingCollection;
        onClientAdded?(client: Client, context: LayoutContext): void;
        onClientRemoved?(client: Client, context: LayoutContext): void;
        onAreaChanged?(oldArea: Rect, newArea: Rect): void;
        compute?(context: LayoutContext): Placement[];
    }

    /** Each trigger: omitted keeps the default, a string replaces it, `null` removes it. */
    export interface LayoutTriggers {
        move?: Trigger | null;
        resize?: Trigger | null;
        swap?: Trigger | null;
    }

    export class FloatingLayout extends Layout {
        constructor(options?: {
            placement?: "smart" | "center" | "cascade";
            cascadeStep?: number;
            triggers?: LayoutTriggers;
        });
    }
    export class MasterStackLayout extends Layout {
        constructor(options?: {
            masterCount?: number;
            ratio?: number;
            gap?: number;
            orientation?: "left" | "right" | "top" | "bottom";
            triggers?: LayoutTriggers;
        });
    }
    export class GridLayout extends Layout {
        constructor(options?: { columns?: number | "auto"; gap?: number });
    }
    export class MonocleLayout extends Layout {}
    export class DwindleLayout extends Layout {
        split: "aspect" | "horizontal" | "vertical";
        ratio: number;
        gap: number | undefined;
        constructor(options?: { split?: "aspect" | "horizontal" | "vertical"; ratio?: number; gap?: number });
    }
    /** Places exactly as `MonocleLayout` does. */
    export class KioskLayout extends MonocleLayout {}

    /** What the strip and carousel helpers need of a node and of a workspace. */
    export interface FocusableNode {
        readonly focused: boolean;
        focus(): void;
    }
    export interface NodeSource {
        readonly clients: Iterable<object>;
    }
    /** A column's width as a fraction of the strip's, held to 0.1 to 1; anything but a number is an error. */
    export function clampColumn(value: unknown): number;
    export class StripLayout extends Layout {
        column: number;
        /** The view offset `scrollTo` pinned, or null while the view follows the focused column. */
        pin: number | null;
        constructor(options?: { column?: number });
        scrollTo(offset: number): void;
        focusLeft(workspace: NodeSource): void;
        focusRight(workspace: NodeSource): void;
    }
    export interface RingOptions {
        fov?: number;
        cardWidth?: number;
        cardHeight?: number;
    }
    export interface RingSlot extends Rect {
        opacity: number;
        /** Degrees: 0 at the front, 180 at the back. */
        rotation: number;
        z: number;
    }
    /** The pure ring layout of `count` cards with the 1-based `focus` at the front. */
    export function ring(count: number, focus: number, area: Rect, options?: RingOptions): RingSlot[];
    export interface Carousel3dOptions extends RingOptions {}
    export class Carousel3dLayout extends Layout {
        fov: number;
        cardWidth: number;
        cardHeight: number;
        constructor(options?: Carousel3dOptions);
        rotate(workspace: NodeSource, steps: number): void;
    }
    export class ArcLayout extends Layout {
        constructor(options?: { radius?: number; spacing?: number });
    }

    // -- Transitions (owner-spec section 14) ----------------------------------

    export type Easing =
        "linear" | "ease-in" | "ease-out" | "ease-in-out" | { cubicBezier: [number, number, number, number] };

    /** Every time is in milliseconds. */
    export interface TransitionOptions {
        duration?: number;
        delay?: number;
        easing?: Easing;
        /** True by default. */
        autoplay?: boolean;
    }

    export class Transition extends NativeObject {
        /** 0..1, writable when the transition is controlled by hand. */
        progress: number;
        readonly running: boolean;
        readonly duration: number;
        readonly finished: Promise<void>;
        play(): void;
        pause(): void;
        reverse(): void;
        finish(): void;
        cancel(): void;
    }

    // -- The simple tier (spec-review section 4) ------------------------------

    /** What a key can do: a closed set, so bindings stay inspectable and diffable on reload. */
    export type KeyAction =
        | "close"
        | SpawnAction
        | ShowAction
        | ShowPositionAction
        | FocusColumnAction
        | BindingAction
        | ((context: { gnoblin: Gnoblin }) => void);

    export interface SpawnAction {
        readonly kind: "spawn";
        readonly argv: readonly string[];
    }
    export interface ShowAction {
        readonly kind: "show";
        readonly workspace: string;
    }
    /** Show the workspace at a 1-based position, whatever its id; nothing if there is none. */
    export interface ShowPositionAction {
        readonly kind: "show-position";
        readonly position: number;
    }
    /** Move focus one column left or right when the shown workspace is a `StripLayout`. */
    export interface FocusColumnAction {
        readonly kind: "focus-column";
        readonly direction: "left" | "right";
    }
    /** Bind what `create` makes (a gesture or drag binding) at the key's trigger. */
    export interface BindingAction {
        readonly kind: "binding";
        readonly create: () => Binding;
    }

    /** Start a process: `spawn("ghostty")`, `spawn("grim", "-g", "...")`. */
    export function spawn(command: string, ...args: string[]): SpawnAction;

    /** Show a workspace on the display under the pointer. */
    export function show(workspace: Workspace | string): ShowAction;
    export namespace show {
        /** `show.numbered("Super+{n}", ["desktop", "dev"])` binds Super+1 and Super+2. */
        function numbered(pattern: `${string}{n}${string}`, workspaces: readonly string[]): Record<string, ShowAction>;
        /** `show.position(3)` shows the third workspace. */
        function position(index: number): ShowPositionAction;
        /** `show.positions("Super+{n}")` binds Super+1 to Super+9 to the workspaces at those positions. */
        function positions(pattern: `${string}{n}${string}`): Record<string, ShowPositionAction>;
    }
    export function focusColumn(direction: "left" | "right"): FocusColumnAction;
    export function binding(create: () => Binding): BindingAction;

    /** A glob (`*` any run, `?` one character) that matches the whole text, or a RegExp tested anywhere in it. */
    export type Pattern = string | RegExp;
    export interface RuleStyle {
        cornerRadius?: number;
        /** From 0 to 1. Applied to the window. */
        opacity?: number;
        shadows?: readonly unknown[];
    }
    /**
     * A window rule: data, so it can be checked and diffed. The query keys must all hold; the first matching rule
     * applies, and one with `all: true` lets the next matching rule apply too. Docs: docs/config/window_rule.md.
     */
    export interface RuleConfig {
        appId?: Pattern;
        title?: Pattern;
        /** Refused: the tree publishes no window role. */
        role?: Pattern;
        focused?: boolean;
        /** The id of the workspace the window is on. */
        onWorkspace?: string;
        /** The workspace id the window is sent to. */
        workspace?: string;
        floating?: boolean;
        size?: { width: number; height: number };
        position?: { x: number; y: number };
        style?: RuleStyle;
        fullscreen?: boolean;
        maximised?: boolean;
        secure?: boolean;
        /** The window does not take focus when it appears. */
        skipFocus?: boolean;
        all?: boolean;
        /** Apply again when appId or title changes, and undo what the rule set when it stops matching. */
        reapply?: boolean;
    }
    export interface RuleEffects {
        workspace?: string;
        floating?: boolean;
        size?: { width: number; height: number };
        position?: { x: number; y: number };
        opacity?: number;
        cornerRadius?: number;
        shadows?: readonly unknown[];
        fullscreen?: boolean;
        maximised?: boolean;
        secure?: boolean;
        skipFocus?: boolean;
    }
    /** A rule after `compileRules` has checked it. */
    export interface CompiledRule {
        readonly index: number;
        readonly effects: Readonly<RuleEffects>;
        readonly all: boolean;
        readonly reapply: boolean;
    }
    /** The window as a rule reads it. */
    export interface RuleView {
        readonly appId: string;
        readonly title: string;
        readonly focused: boolean;
        readonly workspace: string | null;
    }
    /** Checks and compiles rules; throws a TypeError naming the rule's index and the field. */
    export function compileRules(rules: readonly RuleConfig[], workspaces?: ReadonlySet<string> | null): CompiledRule[];
    export function compilePattern(value: unknown, where: string): object;
    export function matchesPattern(pattern: object, text: string): boolean;
    export function ruleMatches(rule: CompiledRule, view: RuleView): boolean;
    /** The rules that apply to a window, in list order: the first match, then on while a rule says `all`. */
    export function matchingRules(rules: readonly CompiledRule[], view: RuleView): CompiledRule[];
    /** What a list of rules sets together: the later rule wins for each field. */
    export function mergeEffects(rules: readonly CompiledRule[]): RuleEffects;
    export function effectsFor(rules: readonly CompiledRule[], view: RuleView): RuleEffects;
    /** What an undone rule leaves a window with. */
    export const RULE_DEFAULTS: {
        readonly secure: false;
        readonly fullscreen: false;
        readonly maximised: false;
        readonly opacity: 1;
    };
    /** What the rule engine asks of the shell. */
    export interface RuleHost<C extends object> {
        view(client: C): RuleView;
        alive(client: C): boolean;
        isArrival(client: C): boolean;
        moveToWorkspace(client: C, id: string): boolean;
        setFloating(client: C, on: boolean): void;
        setSize(client: C, size: { width: number; height: number }): void;
        setBounds(client: C, rect: Rect): void;
        bounds(client: C): Rect;
        setFullscreen(client: C, on: boolean): void;
        setMaximised(client: C, on: boolean): void;
        setSecure(client: C, on: boolean): void;
        setOpacity(client: C, value: number): void;
    }
    /** Applies rules to the windows of a collection it is attached to; `defineConfig` attaches one. */
    export class RuleEngine<C extends object> {
        constructor(rules: readonly CompiledRule[], host: RuleHost<C>);
        skipsFocus(client: C): boolean;
        styleOf(client: C): { cornerRadius?: number; shadows?: readonly unknown[] };
        onAdded(client: C): void;
        onTitleChanged(client: C): void;
        onAppIdChanged(client: C): void;
    }

    export type FocusMode = "click" | "sloppy" | "follow-mouse";
    export type NewWindowPolicy = "allow" | "prevent";
    /** The `focus` key of a config. A key left out is the seat's. */
    export interface FocusConfig {
        mode?: FocusMode;
        newWindows?: NewWindowPolicy;
        raiseOnClick?: boolean;
        autoRaise?: boolean;
        /** Milliseconds. */
        autoRaiseDelay?: number;
        changeOnPointerRest?: boolean;
    }
    export interface FocusSettings {
        readonly mode: FocusMode;
        readonly newWindows: NewWindowPolicy;
        readonly raiseOnClick: boolean;
        readonly autoRaise: boolean;
        readonly autoRaiseDelay: number;
        readonly changeOnPointerRest: boolean;
    }
    export const FOCUS_DEFAULTS: FocusSettings;
    /** The defaults with `user` on top; an unknown key or a bad value throws, naming `where`. */
    export function focusSettings(user?: FocusConfig, where?: string, base?: FocusSettings): FocusSettings;
    /** The seat's published settings in the library's words (hover is sloppy, hover-strict is follow-mouse). */
    export function seatFocusSettings(seat: SeatSettings): FocusSettings;
    export function opensFocused(settings: FocusSettings, fromLauncher: boolean): boolean;
    export function hoverFocus(settings: FocusSettings, hovering: boolean, entered: boolean): boolean;
    export function autoRaiseSeconds(settings: FocusSettings): number;
    export const REST_SECONDS: number;
    export function restFocus(settings: FocusSettings): boolean;
    export function leaveClears(settings: FocusSettings): boolean;
    export type FocusEvent =
        | { readonly kind: "added"; readonly fromLauncher?: boolean; readonly skipFocus?: boolean }
        | { readonly kind: "enter" | "motion"; readonly held: boolean }
        | { readonly kind: "leave"; readonly focused: boolean; readonly heldHere: boolean }
        | { readonly kind: "press" };
    export interface FocusDecision {
        readonly focus: boolean;
        readonly clear: boolean;
        readonly raise: boolean;
        /** Seconds, or null. */
        readonly focusAfter: number | null;
        readonly raiseAfter: number | null;
    }
    /** What to do about one event under `settings`. Pure. */
    export function focusDecision(settings: FocusSettings, event: FocusEvent): FocusDecision;
    /** What the focus policy asks of the shell. */
    export interface FocusHost<C extends object> {
        settings(): FocusSettings;
        focused(client: C): boolean;
        focus(client: C): void;
        clearFocus(): void;
        raise(client: C): void;
        now(): number;
        isArrival(client: C): boolean;
        skipsFocus(client: C): boolean;
        alive(client: C): boolean;
    }
    /** Attached to a collection of clients: focus on a new window, hover and click focus, raise and auto raise. */
    export class FocusPolicy<C extends object> {
        constructor(host: FocusHost<C>);
        onAdded(client: C): void;
        onPointerEnter(client: C): void;
        onPointerMotion(client: C): void;
        onPointerLeave(client: C): void;
        onPointerPress(client: C): void;
        onPointerRelease(): void;
        onClosed(client: C): void;
        /** Runs the timers that are due. */
        frame(): void;
    }

    /** The part of the work area a window tiles to. */
    export type SnapZone = "left" | "right" | "top";
    /** How near an edge the pointer must be to name it: 4 pixels. */
    export const SNAP_MARGIN: number;
    export function edgeTilingEnabled(value: unknown): boolean;
    export function edgeZone(x: number, y: number, area: Rect, margin?: number): SnapZone | null;
    export function tileRect(zone: string, area: Rect): Rect | null;
    export interface SnapClient {
        bounds: Rect;
        maximised?: boolean;
        floatingBounds?: Rect | null;
    }
    export interface SnapOptions {
        edgeTiling: boolean;
        area: Rect;
    }
    /** Tiles a client to a zone; false (and nothing moves) when edge tiling is off or the zone does not tile. */
    export function snapClient(client: SnapClient, zone: string, options: SnapOptions): boolean;
    /** Puts a snapped client back where it floated. False when it is not snapped. */
    export function unsnapClient(client: SnapClient): boolean;
    export function snappedZone(client: object): SnapZone | null;
    /** Ends the snapped state without moving the client. */
    export function forgetSnap(client: object): void;

    export interface OutputSetting {
        /** `"3440x1440@144"` or `"preferred"`. */
        mode?: string;
        scale?: number;
        transform?: "normal" | "90" | "180" | "270" | "flipped" | "flipped-90" | "flipped-180" | "flipped-270";
        vrr?: boolean;
    }

    /** A value of the config document. */
    export type DocumentValue =
        string | number | boolean | readonly DocumentValue[] | { readonly [key: string]: DocumentValue };

    /** One section of the config document. Keys are camelCase or the document's kebab-case. */
    export interface DocumentSection {
        readonly [setting: string]: DocumentValue | undefined;
    }

    /** The look of the compositor's own dialogs (`dialogs.style` in the document); the compositor clamps it. */
    export interface DialogStyleConfig {
        /** The corner radius in points; a dialog is never rounder than 24. */
        radius?: number;
        /** A named accent, `#rgb` or `#rrggbb`. */
        accent?: string;
        font?: { family?: string; size?: number };
        buttonLayout?: "start" | "end" | "stretch" | "stacked";
        /** A number from 0.1 to 100; a dialog is drawn at 1 to 3. */
        scale?: number;
    }

    export interface DialogsConfig {
        style?: DialogStyleConfig;
    }

    /** What a request is: a consent dialog for a portal request, or the compositor's passphrase prompt. */
    export type DialogKind = "consent" | "secret";

    /** A request the compositor is about to open a dialog for. It lives for the observation window and no longer. */
    export interface DialogRequest {
        readonly kind: DialogKind;
        /** The app id the compositor verified for whoever asks. Empty when the request names none. */
        readonly appId: string;
        /**
         * What is asked about: the portal interface of a consent request (`Screenshot`), the app's own prompt text of a
         * passphrase request (not to be trusted as identity).
         */
        readonly subject: string;
        /** When the compositor takes the request back, in milliseconds on its own clock. */
        readonly deadline: number;
        /** Refuses the request by policy; the dialog never opens. There is no method that allows one. */
        decline(reason?: string): void;
    }

    export interface ConsentRequest extends DialogRequest {
        readonly kind: "consent";
    }

    export interface SecretRequest extends DialogRequest {
        readonly kind: "secret";
    }

    /** Observes the requests of the compositor's dialogs. Every method is optional. */
    export class DialogListener {
        onConsentRequest?(request: ConsentRequest): void;
        onSecretRequest?(request: SecretRequest): void;
    }

    export interface DialogManager {
        readonly listeners: readonly DialogListener[];
        addListener(listener: DialogListener): void;
        removeListener(listener: DialogListener): void;
    }

    export interface ConfigEvents {
        clientAdded?(client: Client): void;
        clientRemoved?(client: Client): void;
        titleChanged?(client: Client, title: string): void;
        focusChanged?(client: Client, focused: boolean): void;
    }

    /** Plain data first: `export default defineConfig({ ... })`. Each key desugars to the object API. */
    export interface Config {
        gaps?: Partial<Gaps>;
        style?: ClientStyle;
        workspaces?: readonly WorkspaceOptions[];
        /** Keyed by connector name, for example `"DP-1"`. */
        outputs?: Record<string, OutputSetting>;
        keys?: Record<string, KeyAction>;
        /**
         * The command `Super+space` runs, as a command line or an argv. Absent, `Super+space` opens the stock launcher entry.
         * A key of `keys` for `Super+space` takes precedence.
         */
        launcher?: string | readonly string[];
        /** A titlebar above every window's content. The stock config turns it on; a config that does not ask has none. */
        decorations?: boolean;
        /** Whether the stock window menu answers the compositor's menu requests. On by default. */
        menus?: boolean;
        /** Whether the stock not-responding alert opens for a window that stops answering. On by default. */
        alerts?: boolean;
        rules?: readonly RuleConfig[];
        /** Focus settings; a key left out is the seat's. */
        focus?: FocusConfig;
        /** Whether dropping a dragged window on a screen edge tiles it; left out, the seat's `edge_tiling`. */
        edgeTiling?: boolean;
        /** Where a dragged window may go; left out, the seat's `drag_constraint`. */
        dragConstraint?: DragConstraint;
        on?: ConfigEvents;
        /** The escape hatch: runs after the declarative part and may use any class. */
        setup?(gnoblin: Gnoblin): void | Promise<void>;
        /** The compositor's own dialogs: committed to the config document (`dialogs.style`). */
        dialogs?: DialogsConfig;
        /** Settings of the config document, committed to it. Keys are camelCase: `windowManagement`, `buttonLayout`. */
        compositor?: DocumentSection;
        windowManagement?: DocumentSection;
        cursor?: DocumentSection;
        appearance?: DocumentSection;
        input?: DocumentSection;
        inputSources?: DocumentSection;
        accessibility?: DocumentSection;
        layerShell?: DocumentSection;
        keyring?: DocumentSection;
        auth?: DocumentSection;
        location?: DocumentSection;
        compat?: DocumentSection;
        capture?: DocumentSection;
        monitors?: DocumentSection;
        xwayland?: DocumentSection;
        logging?: DocumentSection;
    }

    /** An identity function with a typed parameter, plus the registration the runtime reads. */
    export function defineConfig(config: Config): Config;

    // -- The global object ----------------------------------------------------

    export interface Gnoblin {
        /** Compositor-managed: read-only membership. */
        readonly clients: Collection<Client>;
        readonly workspaces: WorkspaceCollection;
        readonly outputs: Collection<Output>;
        /** The compositor's seats as input contexts, with the bindings that run on their chords and drags. */
        readonly input: InputManager;
        /**
         * Policy over the compositor's own consent and passphrase dialogs. Attached listeners may refuse a request
         * before its dialog opens; nothing here allows one or reads a secret.
         */
        readonly dialogs: DialogManager;
        /**
         * Run `staging` once, now, and send every property write it makes as one
         * transaction that the compositor animates. It is not called per frame, and
         * `spawn`, I/O and `await` are not allowed inside it.
         */
        requestTransition(staging: () => void, options?: TransitionOptions): Transition;
        spawn(command: string, ...args: string[]): void;
        show(workspace: Workspace | string, options?: ShowOptions): void;
        /** Runs `run` once, `seconds` from now: at the first frame after that time. */
        after(seconds: number, run: () => void): void;
    }

    export interface ShowOptions {
        /** Seconds the slide between the two workspaces takes. Zero or absent switches at once. */
        duration?: number;
        /** How much of the slide a touchpad gesture has made already, 0 to 1. */
        progress?: number;
    }

    /** The seconds a workspace slide takes in the Lua shell and in the stock keys. */
    export const SLIDE_SECONDS: number;

    export interface WorkspaceCollection extends MutableCollection<Workspace> {
        /** Named groups of workspaces to show together. */
        readonly sets: WorkspaceSets;
        /** The area windows are laid out in, which is the primary screen; null while there is none. */
        readonly area: Rect | null;
        show(workspace: Workspace | string, options?: ShowOptions): void;
    }

    // -- Workspace sets and the slide gesture ---------------------------------

    export interface SetEntry {
        workspace: string;
        output?: string;
    }
    export interface WorkspaceSet {
        name: string;
        entries: SetEntry[];
    }
    export type SetError =
        | { kind: "empty_name" }
        | { kind: "duplicate_name"; name: string }
        | { kind: "empty_set"; name: string }
        | { kind: "undeclared_workspace"; set: string; workspace: string }
        | { kind: "duplicate_output"; set: string; output: string }
        | { kind: "unknown_set"; name: string };
    export type SetWarning = { kind: "output_fallback"; set: string; output: string };
    export interface SetPlacement {
        workspace: string;
        output: string;
    }
    export interface SetPlan {
        set: string;
        placements: SetPlacement[];
        warnings: SetWarning[];
        changed: boolean;
    }
    export type SetResult<T> = { ok: true; value: T } | { ok: false; error: SetError };
    export interface ActivateOptions {
        declared: ReadonlySet<string>;
        outputs: readonly string[];
        primary: string;
        active?: string | null;
    }
    /** Validates `incoming` against the declared workspace ids; on success the value is a copy of it. */
    export function defineSets(
        incoming: readonly WorkspaceSet[],
        declared: ReadonlySet<string>,
    ): SetResult<WorkspaceSet[]>;
    export function activateSet(
        list: readonly WorkspaceSet[],
        name: string,
        options: ActivateOptions,
    ): SetResult<SetPlan>;
    export function explainSetError(error: SetError): string;
    export class WorkspaceSetError extends Error {
        readonly error: SetError;
    }
    export interface SetsEnvironment {
        declared(): ReadonlySet<string>;
        outputs(): { names: readonly string[]; primary: string };
        show(workspace: string): void;
        report(message: string): void;
    }
    export class WorkspaceSets {
        constructor(environment: SetsEnvironment);
        readonly list: WorkspaceSet[];
        readonly active: string | null;
        define(list: readonly WorkspaceSet[]): void;
        activate(name: string): SetPlan;
    }

    export interface SlideClient {
        x: number;
        visible: boolean;
        readonly alive: boolean;
        animate(values: { x: number }, options: { duration: number; easing: "out-cubic" }): void;
    }
    export interface SlideWorkspace {
        readonly active: boolean;
        readonly index: number;
        readonly clients: Iterable<SlideClient>;
    }
    export interface SlideHost {
        readonly workspaces: {
            readonly area: { readonly width: number } | null;
            at(index: number): SlideWorkspace | undefined;
            show(workspace: SlideWorkspace, options: { duration: number; progress: number }): void;
            [Symbol.iterator](): Iterator<SlideWorkspace>;
        };
        after(seconds: number, run: () => void): void;
    }
    export interface WorkspaceSlideOptions extends Partial<BindingOptions> {
        travel?: number;
        commit?: number;
        seconds?: number;
        host?: SlideHost;
    }
    /** The three-finger workspace slide of the Lua shell (`Gestures.workspace_slide`), as a binding. */
    export class WorkspaceSlideBinding extends GestureBinding {
        travel: number;
        commit: number;
        seconds: number;
        constructor(options?: WorkspaceSlideOptions);
    }

    // -- The UI toolkit and the stock shell UI (docs/js-ui.md) -----------------

    export type Color = string;
    export type PaintOp =
        | {
              op: "rect";
              x: number;
              y: number;
              width: number;
              height: number;
              radius?: number;
              fill?: Color;
              stroke?: Color;
              strokeWidth?: number;
          }
        | { op: "line"; x1: number; y1: number; x2: number; y2: number; width?: number; color?: Color }
        | { op: "text"; text: string; x?: number; y?: number; size?: number; color?: Color; font?: string }
        | { op: "icon"; name: string; x?: number; y?: number; size?: number; color?: Color }
        | { op: "pushClip"; x: number; y: number; width: number; height: number; radius?: number }
        | { op: "popClip" }
        | { op: "pushOpacity"; alpha: number }
        | { op: "popOpacity" };

    export class UiPointerEvent {
        readonly kind: string;
        x: number;
        y: number;
        readonly button: number;
        readonly dx: number;
        readonly dy: number;
        readonly target: Canvas;
        stopped: boolean;
        stop(): void;
    }
    /** A key sent to the canvas that holds the keyboard focus. `key` is the keysym's name, `text` what it types. */
    export class UiKeyEvent {
        readonly key: string;
        readonly text: string;
        readonly pressed: boolean;
        readonly target: Canvas;
        stopped: boolean;
        stop(): void;
    }

    export interface CanvasOptions {
        x?: number;
        y?: number;
        width?: number;
        height?: number;
        visible?: boolean;
        opacity?: number;
        secure?: boolean;
    }
    /** A node that paints what `draw()` returns. */
    export class Canvas {
        constructor(options?: CanvasOptions);
        readonly node: number;
        parent: Canvas | null;
        readonly children: Canvas[];
        x: number;
        y: number;
        visible: boolean;
        opacity: number;
        secure: boolean;
        width: number | null;
        height: number | null;
        readonly alive: boolean;
        readonly focused: boolean;
        draw(): PaintOp[];
        refresh(): void;
        arrange(): void;
        add(child: Canvas): this;
        remove(child: Canvas): void;
        mount(parentNode: number): void;
        /** Makes this canvas the last child of the root, over every window. */
        mountTop(): void;
        /** Gives this canvas the seat's keyboard focus. */
        focus(): void;
        contains(other: Canvas | null): boolean;
        destroy(): void;
        onPress?(event: UiPointerEvent): void;
        onRelease?(event: UiPointerEvent): void;
        onMotion?(event: UiPointerEvent): void;
        onScroll?(event: UiPointerEvent): void;
        onEnter?(event: UiPointerEvent): void;
        onLeave?(event: UiPointerEvent): void;
        onKey?(event: UiKeyEvent): void;
    }
    export type Direction = "row" | "column" | "none";
    export interface BoxOptions extends CanvasOptions {
        direction?: Direction;
        gap?: number;
        padding?: number;
        grow?: number;
        fill?: Color;
        radius?: number;
        children?: readonly Canvas[];
    }
    export class Box extends Canvas {
        constructor(options?: BoxOptions);
        direction: Direction;
        gap: number;
        padding: number;
        grow: number;
        fill: Color;
        radius: number;
    }
    export interface TextMetrics {
        width: number;
        height: number;
        ascent: number;
    }
    export interface TextOptions extends BoxOptions {
        text?: string;
        size?: number;
        color?: Color;
        font?: string;
        ellipsize?: boolean;
    }
    export class Text extends Box {
        constructor(options?: TextOptions);
        text: string;
        size: number;
        color: Color;
        font: string;
        ellipsize: boolean;
        metrics(): TextMetrics;
    }
    export interface IconOptions extends BoxOptions {
        icon?: string;
        size?: number;
        color?: Color;
    }
    export class Icon extends Box {
        constructor(options?: IconOptions);
        icon: string;
        size: number;
        color: Color;
    }
    export class Grid extends Box {}
    export function encodePaintList(ops: readonly PaintOp[]): number[];
    export function uiCanvasCount(): number;

    export interface TitlebarStyle {
        height: number;
        padding: number;
        gap: number;
        radius: number;
        focusedFill: string;
        unfocusedFill: string;
        titleSize: number;
        titleColor: string;
        titleFont: string;
        buttons: readonly ("minimise" | "maximise" | "close")[];
        buttonSize: number;
        buttonColor: string;
        icons: Readonly<Record<"minimise" | "maximise" | "close", string>>;
    }
    export const defaultTitlebarStyle: Readonly<TitlebarStyle>;
    export interface TitlebarHooks {
        menu?(client: unknown): void;
        lower?(client: unknown): void;
        maximise?(client: unknown, on: boolean): void;
    }
    /** The bar above a window's content. A press does what the seat's `double_click`, `middle_click` and `right_click` say. */
    export class Titlebar extends Box {
        constructor(client: Client, style?: TitlebarStyle, hooks?: TitlebarHooks);
        readonly inset: number;
        readonly title: Text;
    }
    export class Decorations {
        constructor(nodeOf: (client: Client) => number, style?: TitlebarStyle, hooks?: TitlebarHooks);
        titlebarOf(client: Client): Titlebar | undefined;
        /** The height of the bar above `client`'s content; 0 without a bar or while fullscreen. */
        insetOf(client: Client): number;
        onAdded(client: Client): void;
        onRemoved(client: Client): void;
    }

    /** The text of an entry, the caret and the selection, counted in characters. */
    export class EntryModel {
        text: string;
        readonly length: number;
        readonly caret: number;
        readonly anchor: number;
        readonly selection: { start: number; end: number } | null;
        readonly selectedText: string;
        insert(text: string): void;
        backspace(): void;
        delete(): void;
        move(at: number, extend?: boolean): void;
        left(extend?: boolean): void;
        right(extend?: boolean): void;
        home(extend?: boolean): void;
        end(extend?: boolean): void;
        selectAll(): void;
        before(at: number): string;
    }
    export interface EntryOptions extends BoxOptions {
        text?: string;
        secret?: boolean;
        size?: number;
        color?: Color;
        font?: string;
        selectionFill?: Color;
    }
    /** A line of text you type into; give it the keyboard with `focus()`. */
    export class Entry extends Box {
        constructor(options?: EntryOptions);
        readonly model: EntryModel;
        text: string;
        secret: boolean;
        onSubmit?: (text: string) => void;
        shown(text: string): string;
    }
    export function formatTime(format: string, date: Date): string;
    export interface ClockOptions extends BoxOptions {
        format?: string;
        size?: number;
        color?: Color;
        now?: () => Date;
    }
    export class Clock extends Text {
        constructor(options?: ClockOptions);
        format: string;
        now: () => Date;
    }
    export type BarEdge = "top" | "bottom";
    export interface BarOptions extends BoxOptions {
        edge?: BarEdge;
        hideWhen?: () => boolean;
    }
    /** A strip along the top or bottom of the primary screen that takes its room from the windows. */
    export class Bar extends Box {
        constructor(options?: BarOptions);
        edge: BarEdge;
        hideWhen: (() => boolean) | null;
        readonly shown: boolean;
    }
    export function barInsets(): { top: number; bottom: number };

    export interface MenuEntry {
        label?: string;
        run?: (client: MenuClient) => void;
        enabled?: boolean;
        checked?: boolean;
        submenu?: readonly MenuEntry[] | ((client: MenuClient) => readonly MenuEntry[]);
        separator?: boolean;
    }
    export interface MenuClient {
        readonly alive: boolean;
        fullscreen: boolean;
        readonly tiled: boolean;
        readonly secure?: boolean;
        readonly workspace: { readonly id: string; readonly name: string; readonly active: boolean } | null;
        maximised: boolean;
        minimised: boolean;
        close(): void;
        moveWithKeys?(): void;
        resizeWithKeys?(): void;
    }
    export function menuHeight(entries: readonly MenuEntry[]): number;
    export function placeCard(
        anchor: { x: number; y: number },
        size: { width: number; height: number },
        area: { width: number; height: number },
    ): { x: number; y: number };
    export interface NavRow {
        readonly label: string;
        readonly enabled: boolean;
        readonly submenu: boolean;
    }
    /** Which row of a menu the keyboard is on. */
    export class MenuNavigator<R extends NavRow = NavRow> {
        constructor(rows: readonly R[]);
        readonly rows: readonly R[];
        readonly depth: 1 | 2;
        readonly subRows: readonly R[];
        readonly index: number;
        active(): readonly R[];
        selected(): R | null;
        isSelected(row: R): boolean;
        select(row: R): void;
        move(step: 1 | -1): void;
        home(): void;
        end(): void;
        jump(letter: string): void;
        openSubmenu(rows: readonly R[]): void;
        closeSubmenu(): void;
    }
    export type MenuKeyAction =
        | { do: "move"; step: 1 | -1 }
        | { do: "home" | "end" | "activate" | "right" | "left" | "escape" }
        | { do: "jump"; letter: string };
    export function menuKeyAction(key: string, text: string): MenuKeyAction | null;
    /** The menu now open: a layer over the screen with a card of entries. */
    export class WindowMenu extends Box {
        readonly client: MenuClient;
        readonly nav: MenuNavigator;
        readonly panel: Box;
        sub: Box | null;
    }
    /** The stock window menu. Assign `items` to change what it offers and `show` to draw another. */
    export class WindowMenus {
        readonly started: boolean;
        readonly current: WindowMenu | null;
        items(client: MenuClient): MenuEntry[];
        show(client: MenuClient, x?: number, y?: number): WindowMenu | null;
        open(client: MenuClient, x: number, y: number): WindowMenu | null;
        close(): boolean;
        isAbove(client: MenuClient): boolean;
        setAbove(client: MenuClient, on: boolean): void;
    }
    export const windowMenu: WindowMenus;

    export interface AlertClient {
        readonly alive: boolean;
        readonly unresponsive: boolean;
        readonly title: string;
        readonly secure?: boolean;
        kill(): void;
    }
    export type AlertAnswer = "wait" | "force" | "recovered";
    export interface AlertChanges<W> {
        open: W[];
        close: W[];
    }
    /** Which windows get an alert and which are waited on. */
    export class AlertTracker<W extends AlertClient> {
        isOpen(window: W): boolean;
        isWaiting(window: W): boolean;
        check(windows: Iterable<W>, locked: boolean): AlertChanges<W>;
        /** Returns whether the window is to be killed. */
        answered(window: W, answer: AlertAnswer): boolean;
    }
    export function cleanText(text: string, max: number): string;
    export class NotRespondingDialog extends Box {
        index: number;
        armed: boolean;
        onChosen?: (answer: "wait" | "force") => void;
        select(index: number): void;
        arm(): void;
        activate(index: number): void;
    }
    export interface AlertOptions {
        armSeconds?: number;
        intervalMs?: number;
    }
    export class NotRespondingAlerts {
        armSeconds: number;
        intervalMs: number;
        readonly started: boolean;
        cardOf(client: AlertClient): NotRespondingDialog | undefined;
        check(): void;
    }
    export const notRespondingAlerts: NotRespondingAlerts;

    export interface LauncherOptions extends EntryOptions {
        spawn?: (command: string) => void;
    }
    /** A command entry, 600 by 48: Return starts the typed command, Escape closes it. */
    export class Launcher extends Entry {
        constructor(options?: LauncherOptions);
    }
    export function openLauncher(options?: LauncherOptions): Launcher;

    export interface OverviewWorkspace {
        readonly active: boolean;
        readonly clients: Iterable<{ opacity: number }>;
    }
    export const DIMMED_OPACITY: 0.4;
    export class Overview {
        constructor(workspaces: () => Iterable<OverviewWorkspace>);
        readonly on: boolean;
        toggle(): boolean;
    }
    export const overview: Overview;

    export interface ToastOptions {
        fadeIn?: number;
        hold?: number;
        fadeOut?: number;
        now?: () => number;
    }
    export class Toast extends Box {
        constructor(text: string, options?: ToastOptions);
        readonly message: Text;
    }
    export function toast(text: string, options?: ToastOptions): Toast;

    export const gnoblin: Gnoblin;
}

/** The runtime object, also reachable as `import { gnoblin } from "gnoblin"`. */
declare const gnoblin: import("gnoblin").Gnoblin;
