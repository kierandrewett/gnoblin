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

    // -- Bindings (owner-spec section 11) -------------------------------------

    export type Modifier = "Super" | "Ctrl" | "Alt" | "Shift";
    /**
     * A key chord (`"Super+Return"`), a pointer chord (`"Super+MouseLeft"`), or a drag
     * (`"Super+MouseLeft+Drag"`). Proposed grammar: modifiers joined with `+`, then the key.
     */
    export type Trigger = `${Modifier}+${string}` | `XF86${string}`;

    export interface BindingOptions {
        id?: string;
        trigger: Trigger;
        enabled?: boolean;
        priority?: number;
    }

    export class Binding extends NativeObject {
        id: string;
        trigger: Trigger;
        enabled: boolean;
        priority: number;
        constructor(options: BindingOptions);
    }
    export class KeyBinding extends Binding {}
    export class PointerBinding extends Binding {}
    export class GestureBinding extends Binding {}
    export class WorkspaceSwitchBinding extends GestureBinding {}

    export interface DragEvent {
        context: InputContext;
        device: InputDevice;
        /** Set when a managed toplevel is hit. */
        client?: Client;
        layout?: Layout;
        /** Workspace logical coordinates for an ordinary pointer drag. */
        position?: Vec2;
        /** Coordinates local to the hit target. */
        localPosition?: Vec2;
        /** Seconds since the previous event; never negative. */
        deltaTime: number;
        timestamp: number;
        modifiers: ReadonlySet<string>;
    }
    export type DragStartEvent = DragEvent;
    export type DragUpdateEvent = DragEvent;
    export type DragEndEvent = DragEvent;
    export type DragCancelEvent = DragEvent;

    /**
     * `onStart` returns a fresh per-gesture state object, or `false` to decline. Each
     * later callback receives that state, so one binding instance can serve two
     * input contexts at once.
     */
    export abstract class DragBinding<S extends object = object> extends Binding {
        onStart(event: DragStartEvent): S | false;
        onUpdate(event: DragUpdateEvent, state: S): void;
        onEnd(event: DragEndEvent, state: S): void;
        onCancel(event: DragCancelEvent, state: S): void;
    }
    export class PointerDragBinding<S extends object = object> extends DragBinding<S> {}
    export class ClientMoveBinding extends PointerDragBinding {
        constructor(options?: Partial<BindingOptions> & { snap?: number });
    }
    export class ClientResizeBinding extends PointerDragBinding {
        constructor(options?: Partial<BindingOptions> & { edges?: "nearest" | "quadrant" });
    }
    export class ClientSwapBinding extends PointerDragBinding {
        constructor(options?: Partial<BindingOptions>);
    }
    export class SplitResizeBinding extends PointerDragBinding {
        constructor(options?: Partial<BindingOptions>);
    }

    export interface InputDevice {
        readonly id: string;
        readonly name: string;
    }
    export interface InputContext {
        readonly id: string;
        readonly devices: Collection<InputDevice>;
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
        readonly bindings: MutableCollection<Binding>;
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
