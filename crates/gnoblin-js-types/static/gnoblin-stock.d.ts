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
        readonly fullscreen: boolean;
        style: ClientStyle;
        readonly computedStyle: Readonly<ClientStyle>;
        close(): void;
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

    export class ClientListener extends MemberListener<Client> {
        onTitleChanged?(client: Client, title: string): void;
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
    export type PointerTrigger = `${ModifierPrefix}${MouseButton}` | `${ModifierPrefix}${MouseButton}+${"Drag" | "Click"}`;
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
        /** Gestures only: `begin`, `update`, `end` or `cancel`. */
        readonly phase?: string;
        /** Gestures only: movement since the begin in touchpad units, the spread (1 is unchanged) and the rotation in radians. */
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
    /** Super+MouseLeft drag: the client follows the pointer. */
    export class ClientMoveBinding extends PointerDragBinding {
        constructor(options?: Partial<PointerDragBindingOptions>);
    }
    export type ResizeCorner = "top-left" | "top-right" | "bottom-left" | "bottom-right";
    /** Super+MouseRight drag: the corner chosen at the press follows the pointer; it never changes mid-drag. */
    export class ClientResizeBinding extends PointerDragBinding {
        edges: "quadrant" | ResizeCorner;
        constructor(options?: Partial<PointerDragBindingOptions> & { edges?: "quadrant" | ResizeCorner });
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
        /**
         * Claims the chord on the seat and runs `action` on it. Returns the binding; `unbind()` releases the
         * claim once no binding needs it. A drag trigger needs a `PointerDragBinding`, not a function.
         */
        bind(trigger: Trigger, action: ((event: BindingEvent) => void | false) | Binding): Binding;
        /** The topmost client under `position`, in tree order. */
        hitTest(position: Vec2): { client: Client; localPosition: Vec2 } | null;
        /** Makes a layout's bindings live for the clients `owns` accepts (default: those whose workspace shows it). */
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

    export type LayoutName = "floating" | "master-stack" | "grid" | "monocle" | "dwindle";

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
        constructor(options?: { placement?: "smart" | "center" | "cascade"; cascadeStep?: number; triggers?: LayoutTriggers });
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
        constructor(options?: { split?: "aspect" | "horizontal" | "vertical"; ratio?: number; gap?: number });
    }
    export class ArcLayout extends Layout {
        constructor(options?: { radius?: number; spacing?: number });
    }

    // -- Transitions (owner-spec section 14) ----------------------------------

    export type Easing = "linear" | "ease-in" | "ease-out" | "ease-in-out" | { cubicBezier: [number, number, number, number] };

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
    export type KeyAction = "close" | SpawnAction | ShowAction | ((context: { gnoblin: Gnoblin }) => void);

    export interface SpawnAction {
        readonly kind: "spawn";
        readonly argv: readonly string[];
    }
    export interface ShowAction {
        readonly kind: "show";
        readonly workspace: string;
    }

    /** Start a process: `spawn("ghostty")`, `spawn("grim", "-g", "...")`. */
    export function spawn(command: string, ...args: string[]): SpawnAction;

    /** Show a workspace on the display under the pointer. */
    export function show(workspace: Workspace | string): ShowAction;
    export namespace show {
        /** `show.numbered("Super+{n}", ["desktop", "dev"])` binds Super+1 and Super+2. */
        function numbered(pattern: `${string}{n}${string}`, workspaces: readonly string[]): Record<string, ShowAction>;
    }

    export interface WindowRule {
        /** Data-only match, so the compositor can evaluate it. */
        appId?: string;
        title?: string;
        workspace?: string;
        style?: ClientStyle;
    }

    export interface OutputSetting {
        /** `"3440x1440@144"` or `"preferred"`. */
        mode?: string;
        scale?: number;
        transform?: "normal" | "90" | "180" | "270" | "flipped" | "flipped-90" | "flipped-180" | "flipped-270";
        vrr?: boolean;
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
        rules?: readonly WindowRule[];
        on?: ConfigEvents;
        /** The escape hatch: runs after the declarative part and may use any class. */
        setup?(gnoblin: Gnoblin): void | Promise<void>;
    }

    /** An identity function with a typed parameter, plus the registration the runtime reads. */
    export function defineConfig(config: Config): Config;

    // -- The global object ----------------------------------------------------

    export interface Gnoblin {
        /** Compositor-managed: read-only membership. */
        readonly clients: Collection<Client>;
        readonly workspaces: MutableCollection<Workspace>;
        readonly outputs: Collection<Output>;
        /** The compositor's seats as input contexts, with the bindings that run on their chords and drags. */
        readonly input: InputManager;
        /**
         * Run `staging` once, now, and send every property write it makes as one
         * transaction that the compositor animates. It is not called per frame, and
         * `spawn`, I/O and `await` are not allowed inside it.
         */
        requestTransition(staging: () => void, options?: TransitionOptions): Transition;
        spawn(command: string, ...args: string[]): void;
        show(workspace: Workspace | string): void;
    }

    export const gnoblin: Gnoblin;
}

/** The runtime object, also reachable as `import { gnoblin } from "gnoblin"`. */
declare const gnoblin: import("gnoblin").Gnoblin;
