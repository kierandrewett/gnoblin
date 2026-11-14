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
    import type { GnoblinSignals } from "gnoblin/schema/signals";

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
        /** Whether the window holds the keyboard focus. */
        readonly focused: boolean;
        /** The window's content rectangle in the compositor's logical pixels. Writing one moves or resizes the window. */
        x: number;
        y: number;
        width: number;
        height: number;
        /** Gives the window the keyboard focus: a write of the seat's `focus`, which is how a shell hands it out. */
        focus(): void;
        readonly committedSize: Size2D;
        /** The logical size the client is asked to take (its configure request). */
        size: Size2D;
        /** The geometry the layout gave the client. */
        bounds: Rect;
        readonly workspace: Workspace | null;
        /**
         * The window this one is a dialog of (`xdg_toplevel.set_parent`), or null. A dialog is centred in its parent when
         * it opens, moves with it, stays on its workspace and output, and is left out of the layout's tiling.
         */
        readonly parentWindow: Client | null;
        /**
         * Whether the window opened on an activation token the session's launcher minted, so `focus.newWindows` treats it
         * as launched. A program started with `gnoblin.spawn` is not marked: the shell's spawner mints no token.
         */
        readonly fromLauncher: boolean;
        /** The output that shows the client's workspace, or null while no output shows it. */
        readonly output: Output | null;
        /**
         * Moves the window to `workspace` and fits it to that workspace's work area; a window that leaves the shown
         * workspace hands its focus to one that stays, and its dialogs go with it. False, and nothing moves, for a
         * fullscreen window, a dialog, a dead window, a workspace not in `gnoblin.workspaces`, or the one it is on.
         */
        moveToWorkspace(workspace: Workspace): boolean;
        /** `moveToWorkspace` to the workspace `output` shows. False when the output is gone or shows none. */
        moveToOutput(output: Output): boolean;
        /**
         * Starts moving the window with the arrow keys, 16 pixels a press, inside the work area: Return, Enter and Space
         * keep the place, Escape puts the window back. The mode holds the keyboard while it runs and gives it back to
         * the window; it ends when the window closes and when the session locks. False, and nothing starts, for a window
         * that is fullscreen, maximised, tiled, dead or not showing; a snapped window is unsnapped first. The window
         * menu's Move runs it.
         */
        moveWithKeys(): boolean;
        /** Starts resizing the window with the arrow keys: Right and Down grow it, Left and Up shrink it, never below 120 by 80. */
        resizeWithKeys(): boolean;
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
        /**
         * The overrides this client has, and only those: a property never set here is `undefined` and comes from the
         * scopes above. Reading a list it does not override gives a copy of the inherited one; the first write to the
         * copy makes it this client's own. Docs: docs/js-style.md.
         */
        readonly style: ClientStyle;
        /** The style this client resolves to now: frozen, with the layer each value came from in `origin`. */
        readonly computedStyle: ComputedStyle;
        /** What of the style is not drawn and why, and the effects the compositor skipped. */
        readonly styleWarnings: readonly string[];
        /** Whether the window sits in a tiled layout, so its client drops the border and rounding it draws itself. */
        tiled: boolean;
        /** The window's place in depth; negative is in front of the camera. */
        z: number;
        /**
         * Moves properties to `values` over `options.duration` seconds along the easing curve; reading one back at once
         * gives the end value. A duration of 0 writes at once.
         */
        animate(values: ClientAnimation, options: AnimateOptions): void;
        /**
         * Assigns `props` as one `requestTransition` of this client: the same as staging each assignment in a callback.
         * A key that is not a writable property is a `TypeError`. Without `options` the change is atomic and immediate.
         */
        set(props: Record<string, unknown>, options?: TransitionOptions): Transition;
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

    /** The easings of a transition: the ones of `Client.animate`, and the CSS names of the spec (`ease-out` is `out`, `ease-in-out` is `in-out`, `ease-in` is `in-cubic`). */
    export type TransitionEasing = ClientEasing | "ease-out" | "ease-in-out" | "ease-in";

    export interface TransitionOptions {
        /** Milliseconds, 0 or more. Default 250. Zero commits the change at once, still as one transaction. */
        duration?: number;
        /** Default `"out"`. */
        easing?: TransitionEasing;
    }

    /** What a transition ends with: `cancelled` is true when `cancel()` or a newer transition took every property it animated. */
    export interface TransitionResult {
        readonly cancelled: boolean;
    }

    /** Stops a callback before it runs. */
    export interface TransitionSubscription {
        unsubscribe(): void;
    }

    /**
     * One `gnoblin.requestTransition`. There is no `progress`, `play`, `pause` or `reverse`: the compositor's tween player
     * cannot be driven by an external progress, so `autoplay: false` and `delay` are refused (docs/js-transitions.md).
     */
    export class Transition {
        readonly id: number;
        /** The milliseconds the animation takes: the duration asked for, or 0 while the seat asks for reduced motion. */
        readonly duration: number;
        /** True from the call until the transition ends or is cancelled. */
        readonly running: boolean;
        /** True once `cancel()` or a newer transition took every property it animated. */
        readonly cancelled: boolean;
        /** Resolves once with `{ cancelled }`; never rejects. */
        readonly finished: Promise<TransitionResult>;
        /** Calls `fn` once, as a queued call, when the transition ends. */
        onFinished(fn: (result: TransitionResult) => void): TransitionSubscription;
        /** Writes every property the transition still animates at its end value at once, and settles `finished` with `{ cancelled: true }`. */
        cancel(): void;
    }

    /** A scene container that owns a layout and holds clients. */
    export class Workspace extends Node {
        readonly clients: Collection<Client>;
        /** What the workspace is called where a name is shown. */
        name: string;
        /** The position in `gnoblin.workspaces`, or -1 while it is not in it. */
        readonly index: number;
        /** Inner and outer gap that every stock layout reads from `LayoutContext`. */
        gaps: Gaps;
        /**
         * Assigns `props` (`layout`, `gaps`, `name`) as one `requestTransition`: the windows tween from where they are
         * drawn to where the new layout puts them. Without `options` the change is atomic and immediate.
         */
        set(props: Record<string, unknown>, options?: TransitionOptions): Transition;
        /** Whether an output shows this workspace. With several outputs, several workspaces are active. */
        readonly active: boolean;
        /** The output that shows this workspace, or null while none does. */
        readonly output: Output | null;
        /** Shows this workspace on the output under the pointer (the primary output when no pointer says). */
        show(options?: ShowOptions): void;
        /**
         * Moves the keyboard focus `step` windows along the workspace's windows: 1 to the next, -1 to the previous. Nothing
         * moves when no window of the workspace has the focus, and a step past either end leaves it where it is.
         */
        walk(step: number): void;
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

    // -- Styles (docs/js-style.md) --------------------------------------------

    export interface CornerRadii {
        topLeft: number;
        topRight: number;
        bottomRight: number;
        bottomLeft: number;
    }
    /** How many entries a style list holds, and how many effects a window's node carries. */
    export const MAX_STYLE_LIST: 8;
    /** What every list element shares: it is written out as plain data. */
    export abstract class StyleElement {
        toJSON(): Record<string, unknown>;
    }
    export interface BorderInit {
        width?: number;
        color?: Color;
        offset?: number;
        inset?: boolean;
    }
    /** One stroke around the window. Not drawn by any renderer yet. */
    export class Border extends StyleElement {
        constructor(init?: BorderInit);
        /** At least 0. Default 1. */
        width: number;
        /** `#rgb`, `#rgba`, `#rrggbb` or `#rrggbbaa`. Default `#ffffff`. */
        color: Color;
        offset: number;
        inset: boolean;
    }
    export interface ShadowInit {
        x?: number;
        y?: number;
        blur?: number;
        spread?: number;
        color?: Color;
        inset?: boolean;
    }
    /** A shadow behind the window. An inset one is kept and not drawn. */
    export class Shadow extends StyleElement {
        constructor(init?: ShadowInit);
        x: number;
        y: number;
        /** From 0 to 100. Default 12. */
        blur: number;
        /** From -100 to 100. Default 0. */
        spread: number;
        /** Default `#00000080`. */
        color: Color;
        inset: boolean;
    }
    export class BlurFilter extends StyleElement {
        constructor(init?: { radius?: number });
        /** From 0 to 100. Default 12. The compositor blurs at a fixed radius: a positive one switches the blur on. */
        radius: number;
    }
    export class SaturateFilter extends StyleElement {
        constructor(init?: { amount?: number });
        /** From 0 to 2. Default 1. */
        amount: number;
    }
    export class BrightnessFilter extends StyleElement {
        constructor(init?: { amount?: number });
        /** From 0 to 2. Default 1. */
        amount: number;
    }
    export class ContrastFilter extends StyleElement {
        constructor(init?: { amount?: number });
        /** From 0 to 2. Default 1. */
        amount: number;
    }
    export type Filter = BlurFilter | SaturateFilter | BrightnessFilter | ContrastFilter;
    /** A filter, or the shorthand: `{ blur: 18 }` is a `BlurFilter` of radius 18. */
    export type FilterInit =
        Filter | { blur: number } | { saturate: number } | { brightness: number } | { contrast: number };
    export type BorderLike = Border | BorderInit;
    export type ShadowLike = Shadow | ShadowInit;
    export type StyleProperty =
        "opacity" | "filter" | "backdropFilter" | "cornerRadius" | "borders" | "shadows" | "background";
    export interface ClientStyleInit {
        opacity?: number;
        cornerRadius?: number | CornerRadii;
        background?: Color | null;
        borders?: readonly BorderLike[];
        shadows?: readonly ShadowLike[];
        filter?: readonly FilterInit[];
        backdropFilter?: readonly FilterInit[];
    }
    /**
     * What one scope sets; a property it does not set is `undefined` (lists: a copy of the inherited list) and
     * inherited. Assigning `undefined` is `unset`.
     */
    export class Style {
        constructor(init?: {
            opacity?: number;
            filter?: readonly FilterInit[];
            backdropFilter?: readonly FilterInit[];
        });
        /** From 0 to 1. Written to the window node's `opacity`. */
        opacity: number | undefined;
        /** The chain applied to the window's own content. No renderer draws it yet. */
        get filter(): Filter[];
        set filter(value: readonly FilterInit[] | undefined);
        /** The chain applied to what is behind the window. A blur in it is the one the compositor draws. */
        get backdropFilter(): Filter[];
        set backdropFilter(value: readonly FilterInit[] | undefined);
        /** Takes the named properties back to inheriting. */
        unset(...names: StyleProperty[]): void;
        /** Takes every property back to inheriting. */
        reset(): void;
        /** What this scope sets, as plain data. */
        toJSON(): Record<string, unknown>;
    }
    /** The style of a client, or the defaults a scope of clients gives. */
    export class ClientStyle extends Style {
        constructor(init?: ClientStyleInit);
        /** A number, or one radius per corner. Not drawn by any renderer yet. */
        cornerRadius: number | Readonly<CornerRadii> | undefined;
        /** A colour, or `null` for none. Not drawn by any renderer yet. */
        background: Color | null | undefined;
        get borders(): Border[];
        set borders(value: readonly BorderLike[] | undefined);
        /** Behind the window; the first entry is on top. */
        get shadows(): Shadow[];
        set shadows(value: readonly ShadowLike[] | undefined);
    }
    export interface BorderValue {
        readonly width: number;
        readonly color: Color;
        readonly offset: number;
        readonly inset: boolean;
    }
    export interface ShadowValue {
        readonly x: number;
        readonly y: number;
        readonly blur: number;
        readonly spread: number;
        readonly color: Color;
        readonly inset: boolean;
    }
    export type FilterValue =
        | { readonly kind: "blur"; readonly radius: number }
        | { readonly kind: "saturate"; readonly amount: number }
        | { readonly kind: "brightness"; readonly amount: number }
        | { readonly kind: "contrast"; readonly amount: number };
    /** A client's resolved style: frozen data, and the layer each property came from. */
    export interface ComputedStyle {
        readonly opacity: number;
        readonly cornerRadius: number | Readonly<CornerRadii>;
        readonly background: Color | null;
        readonly borders: readonly BorderValue[];
        readonly shadows: readonly ShadowValue[];
        readonly filter: readonly FilterValue[];
        readonly backdropFilter: readonly FilterValue[];
        /** `default`, `global`, `workspace:<id>`, `where(<query>)` or `client`. */
        readonly origin: Readonly<Record<StyleProperty, string>>;
    }
    /** The style a config sets for every client: `gnoblin.clients.style`. */
    export type StyleConfig = ClientStyleInit;

    // -- Collections (owner-spec section 5) -----------------------------------

    /**
     * A live, read-mostly set of native objects. It is not a JS `Array`: membership
     * of `gnoblin.clients` belongs to the compositor, and `add`/`remove` exist only
     * on explicitly mutable collections such as `gnoblin.workspaces`.
     */
    export interface Collection<T> extends Iterable<T> {
        readonly length: number;
        /**
         * The member that holds the keyboard focus, or null: the first seat's focus, and with no seat the member whose own
         * `focused` is set. Only clients take the focus, so another collection's is null.
         */
        readonly focused: T | null;
        at(index: number): T | undefined;
        get(id: string): T | undefined;
        find(predicate: (item: T) => boolean): T | undefined;
        /** A snapshot array. */
        filter(predicate: (item: T) => boolean): T[];
        /** A snapshot array. */
        map<U>(mapper: (item: T) => U): U[];
        /**
         * A live selection. The query is plain data so the compositor can evaluate it. `priority` ranks the selection's
         * `style` among the others a client is in: higher wins, and among equals the one whose style was made later.
         */
        where(query: Query<T>, options?: { priority?: number }): Collection<T>;
        /**
         * The defaults this collection of clients gives: all of them for `gnoblin.clients`, a workspace's own for its
         * `clients`, the selected ones for a `where()`. Only a collection of clients has one.
         */
        get style(): ClientStyle;
        set style(value: ClientStyle | ClientStyleInit);
        addListener(listener: CollectionListener<T> | MemberListener<T>): void;
        removeListener(listener: object): void;
    }

    /** A collection that accepts membership changes from scripts. */
    export interface MutableCollection<T> extends Collection<T> {
        add(item: T): void;
        /**
         * Removes a member given as itself or by id. True when one was removed; false when there was none or the collection
         * refused: `gnoblin.workspaces` keeps its last workspace, and a removed one hands its windows to the neighbour
         * before it (the one after it, for the first), which is shown where it was.
         */
        remove(idOrItem: string | T): boolean;
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
        /**
         * The client asked for a change of its window. Call `request.handle()` to answer it yourself: the first listener
         * that does wins, the listeners after it are not called and the built-in answer (`windowState`) does not run. The
         * client's own listeners are asked before the collections' that hold it.
         */
        onRequest?(client: Client, request: ClientRequest<Client>): void;
    }

    /**
     * Observes the outputs: attach it to one output, or to `gnoblin.outputs` for all of them. A listener on the
     * collection hears `onConnected` for an output plugged in while the shell runs, not for those there at start.
     */
    export class OutputListener extends MemberListener<Output> {
        /** The output appeared: it was plugged in, or made by the compositor. */
        onConnected?(output: Output): void;
        /** The output is gone. Its last values stay readable and `connected` is false. */
        onDisconnected?(output: Output): void;
        /** The output's mode (`WxH@Hz`) changed. */
        onModeChanged?(output: Output, mode: string): void;
        onScaleChanged?(output: Output, scale: number): void;
        /** The output's place or size in the compositor's coordinates changed. */
        onGeometryChanged?(output: Output, geometry: Rect): void;
    }

    /**
     * A connected output, the compositor's `screen` node. It shows one workspace at a time (`workspace`, `show`), and
     * the work area the layouts tile in is its rectangle less the bars on it. A destroyed one keeps its last values and
     * reads `connected === false`. Its properties are the compositor's: none is written from a script. Docs:
     * docs/shell-api/outputs.md.
     */
    export class Output extends NativeObject {
        /** The name the compositor gives the output: what a workspace set's entry and `outputs.where({ name })` use. */
        readonly name: string;
        readonly make: string;
        readonly model: string;
        /** The mode being driven, `WxH@Hz`. */
        readonly mode: string;
        /** The factor the output composes at: 2 is a HiDPI panel. */
        readonly scale: number;
        readonly transform: string;
        /** Whether this is the primary output, the one the stock bars and `gnoblin.workspaces.area` are on. */
        readonly primary: boolean;
        readonly enabled: boolean;
        readonly x: number;
        readonly y: number;
        readonly width: number;
        readonly height: number;
        readonly position: { x: number; y: number };
        /** The whole output in the compositor's coordinates. */
        readonly bounds: Rect;
        /** What a layout tiles in on this output: the output less its bars. Null for a gone output or one with no size. */
        readonly area: Rect | null;
        /** Whether the display is still connected: false once the compositor destroyed the output. */
        readonly connected: boolean;
        /** The workspace the output shows, or null when it shows none or is gone. */
        readonly workspace: Workspace | null;
        /**
         * Shows `workspace` (itself or its id) on this output. A workspace another output shows moves here and that
         * output takes the workspace this one showed, so no workspace is on two outputs.
         */
        show(workspace: Workspace | string, options?: ShowOptions): void;
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
          })
        | (TriggerBase & {
              /** An XR controller grab (`Grip+Drag`); only the bindings of `gnoblin/extensions/xr` read one. */
              readonly kind: "controller";
              readonly hand: "Left" | "Right" | null;
              readonly buttons: readonly ("Trigger" | "Grip" | "Thumbstick" | "Primary" | "Secondary")[];
              readonly action: "drag";
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
        /**
         * `options.only: true` first makes the device binding-only (`bindingOnly`); `only: false` is refused. An action of
         * `false` claims the trigger and does nothing with it.
         */
        bind(
            trigger: Trigger,
            action: ((event: BindingEvent) => void | false) | Binding | false,
            options?: DeviceBindOptions,
        ): Binding;
        /**
         * Whether the device is binding-only: its keys reach no client and fire only the bindings made for it or for every
         * device, so a macro pad never types into a window. Assigning `true` claims the device on the seat by its name,
         * `false` gives the claim up.
         */
        bindingOnly: boolean;
    }
    /** What `InputDevice.bind` takes beside the trigger and the action. */
    export interface DeviceBindOptions {
        /** `true` makes the device binding-only before the binding is made. The only value that means anything. */
        only?: true;
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
        /** The user asked for reduced motion: the shell's animations take no time. */
        readonly reducedMotion: boolean;
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
        bind(trigger: Trigger, action: ((event: BindingEvent) => void | false) | Binding | false): Binding;
        /** Whether the devices that report `name` are binding-only on this seat. */
        isBindingOnly(name: string): boolean;
        /**
         * Makes the devices that report `name` (a name from `devices`) binding-only: their keys reach no client and fire
         * only the bindings made for the device or for every device. A second call changes nothing; the device need not
         * be plugged in yet.
         */
        makeBindingOnly(name: string): void;
        /** Gives the binding-only claim on `name` up. False when there was none. */
        releaseBindingOnly(name: string): boolean;
        /** The topmost client under `position`, in tree order. */
        hitTest(position: Vec2): { client: Client; localPosition: Vec2 } | null;
        /**
         * Makes a layout's bindings live for the clients `owns` accepts (default: those whose workspace's layout it
         * is). The layouts of the workspaces are attached by the runtime; this is for any other.
         */
        attachLayout(layout: Layout, owns?: (client: Client) => boolean): void;
        detachLayout(layout: Layout): void;
        /**
         * Every binding registered on this context, whoever bound it, and where it ranks among the bindings of the
         * same trigger. Plain data, for the inspector.
         */
        describeBindings(): InputBindingInfo[];
    }
    /** What `InputContext.describeBindings` says of one registered binding. */
    export interface InputBindingInfo {
        readonly context: string;
        readonly id: string;
        readonly trigger: string;
        readonly kind: string;
        readonly owner: "context" | "layout" | "device";
        readonly layout: string | null;
        readonly device: string | null;
        readonly priority: number;
        readonly enabled: boolean;
        /** 1 is the binding a press of the trigger reaches first; a binding that declines hands the press to rank 2. */
        readonly rank: number;
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

    /** Start a process: `spawn("ghostty")`, `spawn("grim", "-g", "...")` or `spawn(["wpctl", "set-mute", "@DEFAULT_AUDIO_SINK@", "toggle"])`. */
    export function spawn(command: string | readonly string[], ...args: string[]): SpawnAction;

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
     * A window rule: data, so it can be checked and diffed. The query keys must all hold; every matching rule applies
     * in list order and the later wins for each field, and a rule with `first: true` ends the search where it matches.
     * Docs: docs/config/window_rule.md.
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
        /** When this rule matches, no later rule is consulted. */
        first?: boolean;
        /**
         * Apply again when appId, title or focus changes, and undo what the rule set when it stops matching. Default
         * true; `false` applies the rule once, when the window appears.
         */
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
        readonly first: boolean;
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
        setStyle(client: C, field: "cornerRadius" | "shadows", value: number | readonly unknown[] | undefined): void;
    }
    /** Applies rules to the windows of a collection it is attached to; `defineConfig` attaches one. */
    export class RuleEngine<C extends object> {
        constructor(rules: readonly CompiledRule[], host: RuleHost<C>);
        skipsFocus(client: C): boolean;
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
        /** Whether `client` opened on a token the launcher minted (`focus.newWindows` then decides). Absent: no. */
        fromLauncher?(client: C): boolean;
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
    /** What a snap keeps: the zone and the rectangle unsnapping goes back to. */
    export interface SnapRecord {
        zone: SnapZone;
        floating: Rect;
    }
    /** A copy of what the client's snap keeps, or null when it is not snapped. */
    export function snapRecord(client: object): SnapRecord | null;
    /** Snaps the client again from a record `snapRecord` took, without moving it. */
    export function restoreSnap(client: object, record: SnapRecord): void;
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

    /** What a request is: a consent dialog for a portal request, the compositor's passphrase prompt, or a polkit authentication. */
    export type DialogKind = "consent" | "secret" | "auth";

    /** A request the compositor is about to open a dialog for. It lives for the observation window and no longer. */
    export interface DialogRequest {
        readonly kind: DialogKind;
        /** The app id the compositor verified for whoever asks. Empty when the request names none. */
        readonly appId: string;
        /**
         * What is asked about: the portal interface of a consent request (`Screenshot`), the app's own prompt text of a
         * passphrase request (not to be trusted as identity), the program asking for an authentication.
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

    // -- Idle watches and holds -----------------------------------------------

    /** What a watch does when the session goes idle and when it comes back. Every member is optional. */
    export interface IdleWatchHandlers {
        /** The session has been idle for the watch's time and nothing inhibits idle. */
        onIdle?(watch: IdleWatch): void;
        /** The first input after an idle period, or an inhibitor that took the idle state away. */
        onResume?(watch: IdleWatch): void;
        /** The watch has ended; `reason` is `"cancelled"`. */
        onClosed?(watch: IdleWatch, reason: string): void;
    }

    /** One idle watch. It is not idle when it starts; every idle period runs `onIdle` until it is cancelled. */
    export class IdleWatch {
        /** `"idle-"` and a number. */
        readonly id: string;
        /** The idle time asked for, in milliseconds. */
        readonly ms: number;
        /** True while the session has been idle for `ms` with no inhibitor. */
        readonly idle: boolean;
        /** True until the watch has been cancelled. */
        readonly alive: boolean;
        /** Ends the watch; its `onClosed` runs once. */
        cancel(): void;
    }

    /** A hold on the session's idle state: while one is alive the session does not go idle. */
    export class IdleInhibitor {
        /** The name the hold is recorded under. */
        readonly holder: string;
        /** True until the hold is released or the compositor refused it. */
        readonly alive: boolean;
        release(): void;
    }

    export interface IdleMonitor {
        /** Milliseconds since the last input, as of the last reading; null before the compositor has said. */
        readonly idleMs: number | null;
        /** How many holds keep the session awake, as of the last reading; null before the compositor has said. */
        readonly inhibitors: number | null;
        /** The live watches, oldest first. */
        readonly watches: readonly IdleWatch[];
        /** `ms` is above 0 and at most 86400000; a runtime has at most 256 live watches. */
        watch(ms: number, handlers?: IdleWatchHandlers): IdleWatch;
        /** Holds the session awake under `holder`; holding again under a held name returns that hold. */
        inhibit(holder: string): IdleInhibitor;
    }

    // -- Authentication -------------------------------------------------------

    /**
     * A polkit authentication the compositor is about to open its dialog for. It lives for the observation window and
     * no longer. It carries nothing the password check needs: the compositor draws the dialog, asks for the password
     * in it and checks it with polkit's own helper, and a script can only refuse the request.
     */
    export interface AuthenticationRequest extends DialogRequest {
        readonly kind: "auth";
        /** The polkit action, for example `org.freedesktop.policykit.exec`. */
        readonly actionId: string;
        /** The sentence polkit wants shown, in the session's language. */
        readonly message: string;
        /** Always empty: polkit names a process, not an application, so the program asking is `subject`. */
        readonly appId: string;
        /** The program asking, as polkit reports it (`pkexec`, a process name), or the empty string. */
        readonly subject: string;
        /** Who may authenticate, as polkit writes them: `unix-user:alice`. */
        readonly identities: readonly string[];
    }

    /** Watches the session's polkit authentications. Every method is optional. */
    export class AuthenticationListener {
        /** An authentication is about to be asked of the user; `request.decline(reason)` refuses it. There is no `allow`. */
        onAuthenticationRequest?(request: AuthenticationRequest): void;
    }

    export interface AuthenticationManager {
        readonly listeners: readonly AuthenticationListener[];
        addListener(listener: AuthenticationListener): void;
        removeListener(listener: AuthenticationListener): void;
    }

    // -- Launch feedback ------------------------------------------------------

    /** Where a launch is. It begins `pending` and leaves it once, for one of the other three. */
    export type LaunchState = "pending" | "started" | "timed_out" | "ended";

    /** What a launch tells its owner. Every member is optional. */
    export interface LaunchHandlers {
        /** The deadline in milliseconds, 100 to 10000. Default 3000. */
        timeoutMs?: number;
        onChanged?(launch: Launch, fields: readonly string[]): void;
        onStarted?(launch: Launch): void;
        onTimedOut?(launch: Launch): void;
        /** The launch left `pending`; `reason` is the new state, `replaced`, or why the registry refused it. */
        onClosed?(launch: Launch, reason: string): void;
    }

    export class Launch {
        readonly token: string;
        readonly application: string;
        readonly timeoutMs: number;
        /** Unix time in milliseconds when `begin` was called. */
        readonly startedAt: number;
        readonly state: LaunchState;
        /** True while the launch is `pending`. */
        readonly alive: boolean;
        /** Counts the changes of the launch; 0 until the first. */
        readonly revision: number;
        /** Ends a pending launch; a launch that is not pending is unchanged. */
        finish(): void;
    }

    export interface LaunchFeedback {
        /** The launches the registry keeps (at most 64), oldest first, pending and finished. */
        list(): Launch[];
        /**
         * Tracks a launch of `application` under `token`, pending at once. A window of the application that appears or
         * takes focus starts the oldest pending launch it matches.
         */
        begin(token: string, application: string, options?: LaunchHandlers): Launch;
    }

    /** A command to run: a command line (split on spaces, started with no shell) or the program and its arguments. */
    export type CommandLine = string | readonly string[];

    /** A workspace of a config: its id and its layout, a stock layout by name or any layout object. */
    export interface WorkspaceConfig {
        id: string;
        /** What the workspace is called where a name is shown. Default: the id. */
        name?: string;
        layout?: WorkspaceOptions["layout"];
    }

    /** The Wayland protocols the compositor advertises; `false` stops advertising one. A protocol left out stays on. */
    export interface ProtocolsConfig {
        wlrLayerShell?: boolean;
        extForeignToplevelList?: boolean;
        wlrForeignToplevelManagement?: boolean;
        xdgDecoration?: boolean;
        kdeServerDecoration?: boolean;
        wlrScreencopy?: boolean;
        extImageCopyCapture?: boolean;
        extBackgroundEffectV1?: boolean;
        extDataControl?: boolean;
        extIdleNotify?: boolean;
        wlrGammaControl?: boolean;
        wlrOutputPowerManagement?: boolean;
        wlrOutputManagement?: boolean;
        extSessionLock?: boolean;
    }

    /** One point of a swipe path: from -1 to 1, x to the right and y downward, relative to where the swipe starts. */
    export interface GesturePointConfig {
        x: number;
        y: number;
    }

    /** A touchpad gesture that starts a command when it completes. */
    export interface GestureConfig {
        /** Unique, 1 to 64 letters, digits, `_` or `-`; diagnostics use it. */
        name: string;
        gesture: "swipe" | "pinch";
        /** From 2 to 5. */
        fingers: number;
        /** A pinch only: `in` (fingers together) or `out` (fingers apart). */
        direction?: "in" | "out";
        /** A swipe only: 2 to 16 points, starting at `{ x: 0, y: 0 }`. */
        path?: readonly GesturePointConfig[];
        /** The program and its arguments, run with no shell. */
        command: readonly string[];
        /** The least movement before the gesture can match: 16 to 240 for a swipe, 0.05 to 0.5 for a pinch. */
        threshold?: number;
        /** How closely a swipe must follow its path, from 0.05 to 0.5. */
        tolerance?: number;
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
        style?: StyleConfig;
        workspaces?: readonly WorkspaceConfig[];
        /** Keyed by connector name, for example `"DP-1"`. */
        outputs?: Record<string, OutputSetting>;
        keys?: Record<string, KeyAction>;
        /**
         * Touchpad gestures that run an action when the fingers lift, by the way they went: `Swipe4: { up:
         * "window.toggle-maximise" }`, `Pinch3: { in: "window.minimise", out: fn }`. The trigger is the gesture and its
         * finger count. A gesture here replaces the stock binding of the same trigger (the three-finger workspace slide);
         * giving one gesture to `keys` as well is an error.
         */
        gestureActions?: Record<string, GestureActionConfig>;
        /**
         * Named groups of workspaces to show together, each entry on an output. `gnoblin.workspaces.activateSet(name)` makes
         * one live. A list that does not check (an empty or repeated name, a set with no entries, a workspace that is not
         * declared, two entries for one output) refuses the config.
         */
        workspaceSets?: readonly WorkspaceSetConfig[];
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
        /**
         * Commands to start once per session, each a command line or an argv. A reload starts only the entries it adds,
         * never the ones already started this session; removing an entry stops nothing.
         */
        autostart?: readonly CommandLine[];
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
        /** Which Wayland protocols the compositor advertises, by name. Committed to the config document. */
        protocols?: ProtocolsConfig;
        /** Touchpad swipes and pinches that start a command. Committed to the config document as a list. */
        touchpadGestures?: readonly GestureConfig[];
    }

    /** An identity function with a typed parameter, plus the registration the runtime reads. */
    export function defineConfig(config: Config): Config;

    // -- The global object ----------------------------------------------------

    export interface Gnoblin {
        /**
         * The optional extensions. `extensions.xr` is null until the XR program has a session and is an `XRExtension`
         * while it does (docs/xr.md); a held `XRSession` reads `alive === false` once the session ends.
         */
        readonly extensions: Extensions;
        /** Compositor-managed: read-only membership. */
        readonly clients: Collection<Client>;
        readonly workspaces: WorkspaceCollection;
        readonly outputs: Collection<Output>;
        /** The compositor's seats as input contexts, with the bindings that run on their chords and drags. */
        readonly input: InputManager;
        /**
         * Policy over the compositor's own consent, passphrase and authentication dialogs. Attached listeners may refuse a
         * request before its dialog opens; nothing here allows one or reads a secret.
         */
        readonly dialogs: DialogManager;
        /** The programs this shell started: `spawn`, `list`, `byId`, and listeners told when one ends. */
        readonly process: ProcessTable;
        /** Calls that run later on the frame loop, and the monotonic clock. */
        readonly timers: Timers;
        /** The session's state, and the verbs that log out, suspend, power off or lock it. */
        readonly session: Session;
        /** What a script asks of the process that runs it. */
        readonly runtime: Runtime;
        /** The windows in the order they were last focused. */
        readonly focus: FocusHistory;
        /** Watches for the session going idle, and holds that keep it awake. */
        readonly idle: IdleMonitor;
        /** The session's polkit authentications: listeners that watch them and may refuse one. Nothing here can allow one. */
        readonly authentication: AuthenticationManager;
        /** Launch feedback: the busy state between asking for an application and its window. */
        readonly launches: LaunchFeedback;
        /** The touchpad gesture actions (`workspace.next` and four more) and `onSwipe`, `onPinch` and `direction`. */
        readonly gestures: GestureTools;
        /** The duration the shell gives an animation: `seconds`, or 0 while the seat asks for reduced motion. */
        motion(seconds: number): number;
        /**
         * Runs `callback` once, now, with every property write staged, and draws the change as one animation: each written
         * property moves from where it is drawn to the staged value over `options.duration` milliseconds, and a layout change
         * moves every window from its place. A failure leaves the previous state. Nothing animates under reduced motion.
         * The callback may not be async, call `requestTransition` or have a side effect (`spawn`, `raise`).
         */
        requestTransition(callback: () => void, options?: TransitionOptions): Transition;
        /**
         * Starts a program and returns its `Process`: a command line for `/bin/sh`, or an array of the program and
         * its arguments. The same as `gnoblin.process.spawn`.
         */
        spawn(command: string | readonly string[], options?: SpawnOptions): Process;
        show(workspace: Workspace | string, options?: ShowOptions): void;
        /** Runs `run` once, `seconds` from now: at the first frame after that time. */
        after(seconds: number, run: () => void): Timer;
        /** The build identity. A field the build does not know is `"unknown"`; remote credentials are removed. */
        version(): Version;
        /**
         * The live objects of one kind that match a table of field values or a predicate, in list order. The kinds
         * are `Window`, `Workspace` and `Output`. An unknown kind or field throws and names what works.
         */
        find(kind: "Window", filter?: FindFilter<Client>): Client[];
        find(kind: "Workspace", filter?: FindFilter<Workspace>): Workspace[];
        find(kind: "Output", filter?: FindFilter<Output>): Output[];
        /**
         * Calls `handler` for every `name` the compositor raises (`namespace.object.signal`; hyphens are accepted for
         * underscores). An unknown name throws and lists the near ones.
         */
        on<K extends keyof GnoblinSignals>(name: K, handler: (event: GnoblinSignals[K]) => void): Subscription;
        on(name: string, handler: (event: SignalEvent) => void): Subscription;
        /** Like `on`, for the first signal only. */
        once<K extends keyof GnoblinSignals>(name: K, handler: (event: GnoblinSignals[K]) => void): Subscription;
        once(name: string, handler: (event: SignalEvent) => void): Subscription;
        /** Detaches a subscription, or every `handler` attached to the event `name`. */
        off(target: Subscription): void;
        off(name: string, handler: (event: never) => void): void;
    }

    // -- The host API: operations, timers, processes, the session ------------

    export type FindFilter<T> = Partial<Record<keyof T & string, string | number | boolean>> | ((object: T) => unknown);

    export interface SignalEvent {
        /** `namespace.object.signal`, with underscores. */
        readonly name: string;
        /** The identity of the object that raised it. */
        readonly object: number;
        readonly [field: string]: unknown;
    }

    /** Stops a handler or a callback before it runs. Calling it twice is harmless. */
    export interface Subscription {
        unsubscribe(): void;
    }

    /** The codes of `docs/shell-api/errors.md`, and the reload's own. */
    export type OperationErrorCode =
        | "invalid_argument"
        | "not_found"
        | "unsupported"
        | "denied"
        | "busy"
        | "cancelled"
        | "timed_out"
        | "unavailable"
        | "internal"
        | "config_rejected";

    export interface OperationError {
        readonly code: OperationErrorCode;
        readonly message: string;
    }

    export type OperationStatus = "pending" | "succeeded" | "failed";

    /** The result of a call that finishes later. Settled once, with a value or with a typed error. */
    export class Operation<T> {
        readonly id: number;
        readonly method: string;
        readonly status: OperationStatus;
        readonly value: T | undefined;
        readonly error: OperationError | undefined;
        /** Calls `fn(value, error)` once when the operation settles, as a queued call under the listener budget. */
        onComplete(fn: (value: T | undefined, error: OperationError | undefined) => void): Subscription;
        /** A new operation that succeeds with `fn(value)`, or fails with the same error. */
        map<U>(fn: (value: T) => U): Operation<U>;
        /** A new operation that fails `timed_out` if this one has not settled in `seconds`. Does not cancel this one. */
        timeout(seconds: number): Operation<T>;
        /** Stops waiting: the operation fails `cancelled`. False when it had already settled. */
        cancel(): boolean;
    }

    /** A pending call of a function at a time. */
    export class Timer {
        readonly id: number;
        /** The time it runs at, in milliseconds on the monotonic clock. */
        readonly at: number;
        readonly pending: boolean;
        /** Stops the timer. False when it had already run or was cancelled. */
        cancel(): boolean;
    }

    export interface Timers {
        /** The monotonic clock in milliseconds. Only the difference between two readings is meaningful. */
        now(): number;
        /** How many timers are waiting. */
        readonly size: number;
        /** Runs `fn` once, `ms` milliseconds from now (0 to a day), as a queued call under the listener budget. */
        after(ms: number, fn: () => void): Timer;
    }

    export type LockState = "unlocked" | "covering" | "locked" | "failsafe";

    export interface SessionStatus {
        /** `running` while the shell has a connection to the compositor, `unavailable` when it has none. */
        readonly state: "running" | "unavailable";
        readonly lockAvailable: boolean;
        /** How many lock state changes the session has published. */
        readonly revision: number;
        /** Present only while `lockAvailable` is true: unavailable does not mean unlocked. */
        readonly lockState?: LockState;
    }

    export interface Accepted {
        readonly accepted: true;
    }

    export interface LockRequest {
        readonly dispatched: boolean;
    }

    export interface Session {
        status(): SessionStatus;
        logout(): Operation<Accepted>;
        poweroff(): Operation<Accepted>;
        reboot(): Operation<Accepted>;
        suspend(): Operation<Accepted>;
        hibernate(): Operation<Accepted>;
        lock(): Operation<LockRequest>;
    }

    export interface ReloadResult {
        readonly ok: true;
        readonly action: "config reload";
        readonly settingsRevision: number;
        readonly warnings: readonly string[];
        readonly notApplied: readonly string[];
    }

    export interface Runtime {
        /** Asks for the reload `gnoblinctl config reload` asks for. Throws while the configuration loads. */
        reloadConfig(): Operation<ReloadResult>;
    }

    export interface Version {
        readonly gnoblin: string;
        readonly gnome: string;
        readonly mutter: string;
        readonly js: string;
        readonly api: string;
        readonly gitRemote: string;
        readonly gitSha: string;
        readonly buildId: string;
    }

    export interface FocusHistoryFilter {
        workspaceId?: string;
        monitorId?: string;
        /** 1 to 256; 50 by default. */
        limit?: number;
    }

    export interface FocusHistory {
        /** The live windows, most recently focused first, then the rest in list order. */
        history(filter?: FocusHistoryFilter): Client[];
    }

    export type StreamMode = "inherit" | "discard" | "capture";
    export type ProcessSignal = "term" | "kill" | "int" | "hup" | "usr1" | "usr2" | "stop" | "cont";

    export interface SpawnOptions {
        cwd?: string;
        env?: Record<string, string>;
        stdin?: string;
        stdout?: StreamMode;
        stderr?: StreamMode;
        outputLimit?: number;
        /** Seconds before the group gets SIGTERM, and SIGKILL two seconds later. */
        timeout?: number;
    }

    export interface ProcessExit {
        readonly exitCode: number | null;
        readonly signal: string | null;
        readonly timedOut: boolean;
        readonly stdout: string | null;
        readonly stderr: string | null;
        readonly stdoutTruncated: boolean;
        readonly stderrTruncated: boolean;
    }

    export class Process {
        readonly id: number;
        readonly pid: number;
        readonly argv: readonly string[];
        readonly alive: boolean;
        readonly status: "running" | "exited";
        readonly exitCode: number | null;
        readonly signal: string | null;
        readonly timedOut: boolean;
        readonly exit: ProcessExit | null;
        wait(): Operation<ProcessExit>;
        kill(signal?: ProcessSignal): Operation<null>;
    }

    export class ProcessListener {
        /** A process ended. `code` is its exit status, or null when a signal ended it. */
        onExit?(process: Process, code: number | null): void;
    }

    export interface ProcessTable {
        spawn(command: string | readonly string[], options?: SpawnOptions): Process;
        list(): Process[];
        byId(id: number): Process | null;
        addListener(listener: ProcessListener): void;
        removeListener(listener: ProcessListener): void;
    }

    export interface ShowOptions {
        /** Seconds the slide between the two workspaces takes. Zero or absent switches at once. */
        duration?: number;
        /** How much of the slide a touchpad gesture has made already, 0 to 1. */
        progress?: number;
        /** The output to show it on. Left out, the one under the pointer, else the primary one. */
        output?: Output;
    }

    /** The seconds a workspace slide takes in the Lua shell and in the stock keys. */
    export const SLIDE_SECONDS: number;

    /** One set of `defineConfig({ workspaceSets })`: its name and the workspaces it shows. */
    export interface WorkspaceSetConfig {
        name: string;
        entries: readonly { readonly workspace: string | number; readonly output?: string }[];
    }

    export interface WorkspaceCollection extends MutableCollection<Workspace> {
        /** Named groups of workspaces to show together. */
        readonly sets: WorkspaceSets;
        /**
         * Makes the set `name` live: each entry shows its workspace on its output, and one whose output is not connected
         * shows on the primary output, with a report. The live set again changes nothing. False, with a report, for an
         * unknown set or a workspace that has gone; `sets.activate` throws instead.
         */
        activateSet(name: string): boolean;
        /** The area windows are laid out in on the primary output; null while there is none. */
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
        /** Shows `workspace` on the output named `output`; left out, where the shell would show it. */
        show(workspace: string, output?: string): void;
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
        /** The seconds an animation takes: 0 while the seat asks for reduced motion. */
        motion?(seconds: number): number;
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
    /** The shortest swipe `onSwipe` acts on, in touchpad units. */
    export const SWIPE_THRESHOLD: number;
    /** How far from 1 a pinch's scale must be for `onPinch` to act on it. */
    export const PINCH_THRESHOLD: number;
    export type SwipeDirection = "left" | "right" | "up" | "down";
    export type PinchDirection = "in" | "out";
    /** The five actions the configuration names. `window.minimize` is accepted for `window.minimise`. */
    export type GestureActionName =
        | "workspace.next"
        | "workspace.previous"
        | "window.close"
        | "window.minimise"
        | "window.minimize"
        | "window.toggle-maximise";
    export const GESTURE_ACTION_NAMES: readonly GestureActionName[];
    /** The five stock actions. Each takes nothing: it acts on the workspace under the pointer or the focused window. */
    export interface GestureActions {
        /** Shows the workspace after the shown one; nothing at the last. */
        workspaceNext(): void;
        workspacePrevious(): void;
        /** Asks the focused window to close; nothing without one. */
        windowClose(): void;
        /** Minimises the focused window, which hands the keyboard to the next one. */
        windowMinimise(): void;
        /** Maximises the focused window, or restores it when it is maximised. */
        windowToggleMaximise(): void;
    }
    /** What a gesture runs: the name of a stock action, or a function that gets the gesture's last event. */
    export type GestureAction = GestureActionName | ((event: DragEvent) => void);
    export type SwipeMap = { [way in SwipeDirection]?: GestureAction };
    export type PinchMap = { [way in PinchDirection]?: GestureAction };
    /** What `defineConfig({ gestureActions })` takes for one gesture: a swipe's four directions or a pinch's two. */
    export type GestureActionConfig = SwipeMap & PinchMap;
    export interface GestureActionOptions {
        id?: string;
        trigger?: string;
        /** Touchpad units for a swipe (default 48), the distance of the scale from 1 for a pinch (default 0.12). */
        threshold?: number;
        /** What a named action runs. Default the `gnoblin` global's `gestures.actions`. */
        actions?: GestureActions;
    }
    /** A binding that acts when a swipe ends, for the direction it went. Made by `onSwipe`. */
    export class SwipeActionBinding extends GestureBinding {
        readonly threshold: number;
        constructor(map: SwipeMap, options?: GestureActionOptions);
    }
    /** A binding that acts when a pinch ends: `out` for a spread and `in` for a gather. Made by `onPinch`. */
    export class PinchActionBinding extends GestureBinding {
        readonly threshold: number;
        constructor(map: PinchMap, options?: GestureActionOptions);
    }
    /**
     * The way a gesture has gone: `left`, `right`, `up` or `down` by its longer axis, or null while it has moved less than
     * `threshold` (default 48) touchpad units.
     */
    export function gestureDirection(
        event: { readonly dx?: number; readonly dy?: number },
        threshold?: number,
    ): SwipeDirection | null;
    /** A binding for `InputContext.bind` that runs `map[direction]` when a swipe ends, if it went that way. */
    export function onSwipe(map: SwipeMap, threshold?: number, options?: GestureActionOptions): SwipeActionBinding;
    /** A binding that runs `map.out` or `map.in` when a pinch ends. */
    export function onPinch(map: PinchMap, threshold?: number, options?: GestureActionOptions): PinchActionBinding;
    /** The gesture helpers of the `gnoblin` global. */
    export interface GestureTools {
        readonly actions: GestureActions;
        direction: typeof gestureDirection;
        onSwipe(map: SwipeMap, threshold?: number): SwipeActionBinding;
        onPinch(map: PinchMap, threshold?: number): PinchActionBinding;
    }
    export function gestureActionBinding(
        trigger: string,
        map: GestureActionConfig,
        gesture: "swipe" | "pinch",
    ): GestureBinding;
    export const defaultTitlebarStyle: Readonly<TitlebarStyle>;
    export interface TitlebarHooks {
        menu?(client: unknown): void;
        lower?(client: unknown): void;
        maximise?(client: unknown, on: boolean): void;
        minimise?(client: unknown): void;
        /** The seconds an animation takes: 0 while the seat asks for reduced motion. */
        motion?(seconds: number): number;
        /** A button went down on the bar itself: the window is pressed, so it takes the focus. */
        press?(client: unknown, event: { x: number; y: number; button: number }): void;
        release?(client: unknown, event: { x: number; y: number; button: number }): void;
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
    /** The output a bar sits on, as far as the bar needs it. An `Output` satisfies it. */
    export interface BarOutput {
        readonly alive: boolean;
        readonly x: number;
        readonly y: number;
        readonly width: number;
        readonly height: number;
    }
    export interface BarOptions extends BoxOptions {
        edge?: BarEdge;
        hideWhen?: () => boolean;
        /** The output the bar sits on. Left out, the primary output; a bar whose output is unplugged is hidden. */
        output?: BarOutput | null;
    }
    /** A strip along the top or bottom of an output (the primary one unless `output` names another) that takes its room from the windows there. */
    export class Bar extends Box {
        constructor(options?: BarOptions);
        edge: BarEdge;
        hideWhen: (() => boolean) | null;
        output: BarOutput | null;
        /** The output the bar sits on now: its own, or the primary one. */
        readonly onOutput: BarOutput | null;
        readonly shown: boolean;
    }
    /** The room the bars take from the top and the bottom of `output` (the primary output when none is given). */
    export function barInsets(output?: BarOutput | null): { top: number; bottom: number };
    /** Makes every bar give way (hidden, taking no room) while `covered` returns true for its output; null stops it. */
    export function barsGiveWayTo(covered: ((output: BarOutput | null) => boolean) | null): void;
    /** Tells the bars which output is the primary one, where a bar with no `output` sits. */
    export function barsOnOutputs(primary: (() => BarOutput | null) | null): void;

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

    /** How far an arrow key moves or resizes a window in the keyboard mode, in pixels. */
    export const REPOSITION_STEP: number;
    /** The least width a keyboard resize keeps. */
    export const REPOSITION_MIN_WIDTH: number;
    /** The least height a keyboard resize keeps. */
    export const REPOSITION_MIN_HEIGHT: number;
    export type RepositionMode = "move" | "resize";
    /** What the keyboard move and resize mode needs of a window. A `Client` is one. */
    export interface RepositionClient {
        readonly alive: boolean;
        readonly fullscreen: boolean;
        readonly maximised: boolean;
        readonly tiled: boolean;
        readonly secure?: boolean;
        readonly frameInset?: number;
        readonly workspace: { readonly active: boolean } | null;
        bounds: Rect;
        floatingBounds?: Rect | null;
    }
    /** What the keyboard mode needs of the shell around it. */
    export interface RepositionWorld {
        workArea(client: RepositionClient): Rect | null;
        dragConstraint(): unknown;
        focus(client: RepositionClient): void;
    }
    /** The node that holds the keyboard while the keyboard mode runs. */
    export class RepositionGrab extends Box {
        readonly mode: RepositionMode;
        readonly client: RepositionClient;
        readonly from: Rect;
    }
    /** The keyboard move and resize mode: at most one window at a time. */
    export class Repositioning {
        start(world: RepositionWorld): void;
        readonly current: RepositionClient | null;
        readonly grab: RepositionGrab | null;
        begin(client: RepositionClient, mode: RepositionMode): boolean;
        step(grab: RepositionGrab, dx: number, dy: number): void;
        finish(keep: boolean): boolean;
    }
    export const repositioning: Repositioning;

    /** Every request the compositor makes of a window. `menu` is answered by the window menu. */
    export type ClientRequestKind =
        | "maximize"
        | "unmaximize"
        | "fullscreen"
        | "unfullscreen"
        | "minimize"
        | "unminimize"
        | "close"
        | "raise"
        | "lower"
        | "activate"
        | "move"
        | "resize"
        | "menu";
    export const CLIENT_REQUEST_KINDS: readonly ClientRequestKind[];
    /** How near two sides of a window a resize request's pointer must be for the corner between them to move. */
    export const RESIZE_CORNER_ZONE: number;
    /** A drag a request started ends after this many milliseconds even when no button edge arrives. */
    export const REQUEST_DRAG_LIMIT_MS: number;
    /** A request a client made of its window. A listener answers it with `handle()`; see `ClientListener.onRequest`. */
    export class ClientRequest<C = unknown> {
        constructor(kind: ClientRequestKind, client: C, x?: number, y?: number);
        readonly kind: ClientRequestKind;
        readonly client: C;
        /** Where a `menu` request is anchored; 0 for every other kind. */
        readonly x: number;
        readonly y: number;
        readonly handled: boolean;
        /** Takes the request: no later listener hears it and the built-in answer does not run. */
        handle(): void;
    }
    /** What the request policy reads and writes of a window; the host's `Client` satisfies it. */
    export interface StateClient {
        readonly alive: boolean;
        fullscreen: boolean;
        maximised: boolean;
        minimised: boolean;
        readonly tiled: boolean;
        bounds: Rect;
        floatingBounds?: Rect | null;
        readonly frameInset?: number;
        minSize?: { width: number; height: number };
        maxSize?: { width: number; height: number };
    }
    /** What the request policy needs of the shell around the windows. */
    export interface WindowStateWorld<C extends StateClient> {
        clientOf(node: number): C | undefined;
        siblings(client: C): readonly C[];
        raise(client: C): void;
        lower(client: C): void;
        close(client: C): void;
        focused(): C | null;
        focus(client: C | null): void;
        onActiveWorkspace(client: C): boolean;
        output(client?: C): Rect | null;
        workArea(client?: C): Rect | null;
        framed(client: C, frame: Rect): Rect;
        snapped(client: C): boolean;
        unsnap(client: C): void;
        dispatch(client: C, request: ClientRequest<C>): void;
        report(site: string, error: unknown): void;
        now(): number;
        dragConstraint(): string | undefined;
        edgeTiling(): boolean;
    }
    /** The stock answers to a window's requests, and the drags that `move` and `resize` start. */
    /** The rectangles a window goes back to when it stops being maximised or fullscreen. */
    export interface KeptRects {
        floating: Rect | null;
        windowed: Rect | null;
    }
    export class WindowState {
        readonly started: boolean;
        readonly dragging: boolean;
        start<C extends StateClient>(world: WindowStateWorld<C>): void;
        /**
         * The rectangles `client` returns to when it leaves maximised or fullscreen. A reload carries them to the next
         * generation: the host reads this before it goes and `keep`s it in the next.
         */
        kept(client: StateClient): KeptRects;
        /** Gives `client` back the rectangles `kept` took. */
        keep(client: StateClient, kept: Partial<KeptRects>): void;
        /** Answers `kind` for `client` as the built-in policy does. */
        answer(client: StateClient, kind: ClientRequestKind): void;
        /** Fills the work area, keeping the floating rectangle. False when maximised or fullscreen already. */
        maximise(client: StateClient): boolean;
        unmaximise(client: StateClient): boolean;
        /** Covers the whole output and goes last among its siblings, keeping the windowed rectangle. False when already. */
        enterFullscreen(client: StateClient): boolean;
        leaveFullscreen(client: StateClient): boolean;
        /** Hides the window and hands the keyboard focus to the topmost sibling still showing. False when already. */
        minimise(client: StateClient): boolean;
        unminimise(client: StateClient): boolean;
        /** Unminimise, focus and raise a window on the active workspace. False when it is on another workspace. */
        present(client: StateClient): boolean;
        raise(client: StateClient): void;
        lower(client: StateClient): void;
        beginMove(client: StateClient): boolean;
        beginResize(client: StateClient): boolean;
        pointerEdge(kind: string): void;
    }
    export const windowState: WindowState;
    /** Whether a listener handled the compositor's request, as the window menu asks before it opens. */
    export function isRequestHandled(request: { readonly node: number; readonly request: string }): boolean;

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
    /** The compositor's answer to a password check: `result` is `ok`, or the wait in milliseconds under `rate-limited`. */
    export interface LockAnswer {
        token: number;
        result: string;
        error: string;
    }
    /** What the stock lock screen needs of the shell host. A test attaches a fake one. */
    export interface LockBackend {
        state(): string;
        node(): number | undefined;
        screen(): number[] | undefined;
        engage(engaged: boolean): string | undefined;
        authenticate(password: string): number | undefined;
        takeAnswers(): LockAnswer[];
    }
    /** Everything that sets how the lock screen looks and what it says. */
    export interface LockStyle {
        scrimFill: string;
        cardWidth: number;
        cardHeight: number;
        cardRadius: number;
        cardPadding: number;
        cardGap: number;
        cardFill: string;
        cardTop: number;
        clockSize: number;
        clockColor: string;
        entryHeight: number;
        entryRadius: number;
        entryFill: string;
        entrySize: number;
        entryColor: string;
        entryLimit: number;
        messageSize: number;
        hintColor: string;
        errorColor: string;
        hint: string;
        checking: string;
        denied: string;
        busy: string;
        failed: string;
        rateLimited: string;
    }
    export const defaultLockStyle: Readonly<LockStyle>;
    /** The time of day as `HH:MM`, in the machine's time zone. */
    export function formatClock(milliseconds: number): string;
    /** The password entry of the lock screen: it keeps what was typed as code points, never as a string. */
    export class PasswordEntry {
        enabled: boolean;
        limit: number;
        readonly length: number;
        onSubmit: ((password: string) => void) | null;
        /** Overwrites what was typed, then forgets it. */
        clear(): void;
    }
    /** The stock lock screen: up while the session is locked, down when it is not. */
    export class LockScreen {
        readonly style: LockStyle;
        readonly shown: boolean;
        constructor(backend: LockBackend, style?: LockStyle);
        frame(): void;
        release(): void;
    }
    /** The lock screen of the running generation. */
    export function lockScreen(): LockScreen | null;

    // -- Inspector (owner-spec section 18) ------------------------------------

    /** What a registered kind reports of one instance, for the console's renderer. */
    export interface InspectView {
        /** The kind's name, as it prints: `Client`, `Workspace`. */
        readonly name: string;
        /** The instance's id; it prints as `Name#id`. */
        readonly id?: string;
        /** Everything worth showing of the instance when it is the value itself. */
        readonly fields: Readonly<Record<string, unknown>>;
        /** The few fields shown when the instance is one of many, or nested. Defaults to `fields`. */
        readonly brief?: Readonly<Record<string, unknown>>;
        /** A collection's members; it renders as an array of their briefs. */
        readonly items?: readonly unknown[];
    }
    /** What the console gets back for one value. */
    export interface Rendered {
        readonly type: string;
        readonly value?: unknown;
        readonly text: string;
        readonly truncated: boolean;
    }
    export interface RenderOptions {
        /** The most text the rendering may hold before it stops adding to it. Default 65536. */
        readonly maxBytes?: number;
        /** How deep objects and arrays are walked. Default 6. */
        readonly maxDepth?: number;
    }
    /** A client as plain data. */
    export interface ClientInfo {
        readonly id: string;
        readonly appId: string;
        readonly title: string;
        readonly focused: boolean;
        readonly fullscreen: boolean;
        readonly floating: boolean;
        readonly x: number;
        readonly y: number;
        readonly width: number;
        readonly height: number;
        /** The id of the workspace the client is on, or null. */
        readonly workspace: string | null;
    }
    /** A workspace as plain data. */
    export interface WorkspaceInfo {
        readonly id: string;
        readonly name: string;
        readonly index: number;
        readonly active: boolean;
        /** The layout's class name. */
        readonly layout: string;
        /** The ids of the clients on it. */
        readonly clients: readonly string[];
    }
    /** One binding registered on an input context, and where it ranks among the ones with the same trigger. */
    export interface BindingInfo {
        readonly context: string;
        readonly id: string;
        readonly trigger: string;
        readonly kind: string;
        /** Who bound it: the context itself, a layout attached to it, or one device of it. */
        readonly owner: "context" | "layout" | "device";
        readonly layout: string | null;
        readonly device: string | null;
        readonly priority: number;
        readonly enabled: boolean;
        /** 1 is the binding a press reaches first, 2 the one it falls to when 1 declines. */
        readonly rank: number;
    }
    /**
     * The inspector helpers: plain data about the running shell. `inspect` is also a global of the console
     * (`gnoblinctl console`), so `inspect.clients()` needs no import there.
     */
    export const inspect: {
        /** Every client as plain data. */
        clients(): ClientInfo[];
        /** Every workspace as plain data, with the ids of the clients on it. */
        workspaces(): WorkspaceInfo[];
        /** The bindings registered on every input context; a shell with no seat has none. */
        bindings(): BindingInfo[];
        /** The effective config as plain data: the user's over the stock one; null when `defineConfig` never ran. */
        config(): unknown;
        /** The value as the console shows it: JSON text of `{ type, value?, text, truncated }`. */
        render(value: unknown, options?: RenderOptions): string;
        /** The value as plain JSON-able data. */
        plain(value: unknown, options?: RenderOptions): unknown;
        /** Teaches the renderer what an instance of `ctor` shows. */
        register<T>(ctor: abstract new (...args: never[]) => T, describe: (value: T) => InspectView): void;
    };

    export const gnoblin: Gnoblin;

    // -- Optional extensions and scene nodes (design/js-xr.md) -----------------

    /** A scene node a script made: the XR room is one, a View and a Camera3D are others. See `gnoblin/extensions/xr`. */
    export { Camera3D, View } from "gnoblin/extensions/xr";

    /** Hears the XR extension's sessions begin and end. Override what you need. */
    export interface XRSessionListener {
        onSessionStarted?(session: import("gnoblin/extensions/xr").XRSession): void;
        onSessionEnded?(session: import("gnoblin/extensions/xr").XRSession): void;
    }

    export interface XROptions {
        /** Whether the stock shell builds a room when a session starts. Default true. */
        room?: boolean;
        /** The ring and the bindings of the stock room. */
        layout?: import("gnoblin/extensions/xr").SpatialLayoutOptions;
        /** Whether a client gets a panel. Secure clients never do. */
        accept?: (client: Client) => boolean;
    }

    export interface Extensions {
        /** The XR extension while the program has a session; null otherwise, so `xr?.session` is the guard. */
        readonly xr: import("gnoblin/extensions/xr").XRExtension | null;
        addListener(listener: XRSessionListener): void;
        removeListener(listener: XRSessionListener): void;
        /** What the stock shell does for an extension: `configure("xr", { room: false })` builds no room of its own. */
        configure(name: "xr", options: XROptions): void;
        /** Throws unless the extension can be used now. `gnoblin/extensions/xr` calls it as it loads. */
        require(name: "xr"): void;
    }
}

/**
 * The XR extension (owner-spec.md section 13, through the out-of-process program of D34 and D35: design/js-xr.md).
 * Importing this module when no XR extension is there fails at load with a message that says what to do.
 */
declare module "gnoblin/extensions/xr" {
    import type { Binding, BindingCollection, Client, DragEvent } from "gnoblin";

    /** A point or a direction. */
    export class Vector3 {
        x: number;
        y: number;
        z: number;
        constructor(x?: number, y?: number, z?: number);
        static zero(): Vector3;
        static from(parts: readonly number[]): Vector3;
        clone(): Vector3;
        add(other: Vector3): Vector3;
        subtract(other: Vector3): Vector3;
        scale(factor: number): Vector3;
        dot(other: Vector3): number;
        cross(other: Vector3): Vector3;
        length(): number;
        normalize(): Vector3;
        distanceTo(other: Vector3): number;
        toArray(): [number, number, number];
    }
    /** A half line from `origin` along `direction`. */
    export class Ray {
        readonly origin: Vector3;
        readonly direction: Vector3;
        constructor(origin: Vector3, direction: Vector3);
        at(distance: number): Vector3;
    }
    export class Plane {
        readonly point: Vector3;
        readonly normal: Vector3;
        constructor(point: Vector3, normal: Vector3);
        /** Where `ray` crosses the plane; null when parallel to it or when it is behind the ray. */
        intersect(ray: Ray): Vector3 | null;
    }
    /** A pose: a position, a rotation (Euler radians about X, Y, then Z) and one uniform scale. */
    export class Transform3D {
        position: Vector3;
        scale: number;
        rotation: Vector3;
        constructor(position?: Vector3, rotation?: readonly number[], scale?: number);
        static identity(): Transform3D;
        static translation(x: number, y: number, z: number): Transform3D;
        clone(): Transform3D;
        /** `other` placed inside `this`. */
        multiply(other: Transform3D): Transform3D;
        inverse(): Transform3D;
        transformDirection(direction: Vector3): Vector3;
        transformPoint(point: Vector3): Vector3;
        plane(): Plane;
    }

    export interface NodeOptions {
        name?: string;
        x?: number;
        y?: number;
        z?: number;
        rotation?: readonly [number, number, number];
        scale?: readonly [number, number, number];
        visible?: boolean;
        opacity?: number;
        input?: boolean;
    }
    /** A node of the scene a script made. Adding children and destroying it go through the tree. */
    export class Node {
        constructor(options?: NodeOptions);
        readonly alive: boolean;
        readonly id: string;
        name: string;
        x: number;
        y: number;
        z: number;
        rotation: readonly [number, number, number];
        /** The scale along each axis; a panel is scaled by the first component. */
        scale: readonly [number, number, number];
        visible: boolean;
        opacity: number;
        /** Whether a pointer ray can hit the node. */
        input: boolean;
        transform: Transform3D;
        readonly worldTransform: Transform3D;
        readonly parent: Node | null;
        /** The children this module made, in tree order: a snapshot. */
        readonly children: Node[];
        add<T extends Node>(child: T, index?: number): T;
        find(name: string): Node | undefined;
        destroy(): void;
    }
    export interface Camera3DOptions extends NodeOptions {
        projection?: "orthographic" | "perspective" | "stereo";
        metres?: number;
        near?: number;
        far?: number;
        fov?: number;
        ipd?: number;
    }
    export class Camera3D extends Node {
        constructor(options?: Camera3DOptions);
        projection: "orthographic" | "perspective" | "stereo";
        metres: number;
        near: number;
        far: number;
        fov: number;
        ipd: number;
        readonly layerBudget: number;
    }
    export interface ViewOptions extends NodeOptions {
        content?: Client | null;
        camera?: Camera3D | null;
        width?: number;
        height?: number;
    }
    /** Draws a client as a picture. The client is not reparented or moved. A workspace cannot be shown yet. */
    export class View extends Node {
        constructor(options?: ViewOptions);
        content: Client | null;
        readonly orphaned: boolean;
        camera: Camera3D | null;
        width: number;
        height: number;
    }

    export type ControllerButton = "Trigger" | "Grip" | "Thumbstick" | "Primary" | "Secondary";
    export interface ControllerTriggerParts {
        readonly kind: "controller";
        readonly text: string;
        readonly modifiers: readonly never[];
        readonly hand: "Left" | "Right" | null;
        readonly buttons: readonly ControllerButton[];
        readonly action: "drag";
    }
    /** Parses `Grip+Drag`, `Right.Grip+Drag`, `Trigger+Grip+Drag`. Throws a TypeError naming the wrong token. */
    export function parseControllerTrigger(trigger: unknown, what?: string): ControllerTriggerParts;

    export interface XRClient {
        readonly alive: boolean;
        readonly secure: boolean;
        readonly minimised: boolean;
        width: number;
        height: number;
    }

    /** One tracked hand: a `controller` node the XR program writes. */
    export class XRController {
        readonly id: string;
        readonly alive: boolean;
        readonly hand: string;
        readonly tracked: boolean;
        /** The aim ray's pose in the room's placement frame, in room units. */
        readonly pose: Transform3D;
        readonly ray: Ray;
        readonly trigger: number;
        readonly grip: number;
        readonly thumbstick: { x: number; y: number };
        /** The buttons down as of the last frame (a pull is down at 0.75, up at 0.25). */
        readonly buttons: ReadonlySet<ControllerButton>;
        /** The inputs the shell withholds from the window under the ray now: the union of every owner's claim. */
        readonly claimed: readonly string[];
        /**
         * Claims inputs (`trigger`, `grip`, `thumbstick`, `primary`, `secondary`, lowercase) for `owner`; its earlier claim
         * is replaced and other owners' claims stand. Throws a `TypeError` naming `Controller#id.claimed` for anything else.
         */
        claim(owner: object, inputs: readonly string[]): void;
        /** Drops `owner`'s claim. */
        release(owner: object): void;
    }

    export class XRList<T extends { readonly id: string }> implements Iterable<T> {
        readonly size: number;
        get(id: string): T | undefined;
        at(index: number): T | undefined;
        addListener(listener: { onAdded?(item: T): void; onRemoved?(item: T): void }): void;
        removeListener(listener: object): void;
        [Symbol.iterator](): Iterator<T>;
    }

    export interface XRCamera3DOptions extends Omit<Camera3DOptions, "near" | "far"> {
        tracking?: XRTrackedPose;
        referenceSpace?: "local";
        /** In metres. */
        near?: number;
        far?: number;
    }
    /** The stereo camera that follows the headset: the camera named `head`, which the XR program writes. */
    export class XRCamera3D extends Camera3D {
        constructor(options?: XRCamera3DOptions);
        readonly tracking: XRTrackedPose | null;
        readonly referenceSpace: "local";
    }

    export type XRComposition = "auto" | "layers" | "projection";
    export interface XRViewportOptions {
        content: Node;
        camera: Camera3D;
        composition?: XRComposition;
    }
    /** The room and the camera the headset shows. Assigning it to `session.viewport` names them for the program. */
    export class XRViewport {
        constructor(options: XRViewportOptions);
        readonly content: Node;
        readonly camera: Camera3D;
        composition: XRComposition;
    }

    /** The head: the pose the program writes onto the camera named `head`. */
    export class XRTrackedPose {
        readonly available: boolean;
        /** In the room's placement frame, in room units; null with no head camera. */
        readonly pose: Transform3D | null;
    }

    export interface XRDragEvent extends DragEvent {
        /** Always null: the device is the controller, in `controller`. */
        readonly device: null;
        readonly controller: XRController;
        /** The panel under the ray when the drag started. */
        readonly view?: View;
        readonly ray: Ray;
        readonly hit?: Vector3;
        readonly hand: string;
        readonly room: XRRoom;
        /** Metres per room unit. */
        readonly unit: number;
    }
    export interface XRBindingOptions {
        id: string;
        trigger: string;
        enabled?: boolean;
        priority?: number;
    }
    /** The base of the bindings that follow a controller from the press of its buttons to their release. */
    export class XRControllerDragBinding extends Binding {
        constructor(options: XRBindingOptions);
        readonly parsed: ControllerTriggerParts;
        onStart(event: XRDragEvent): object | false;
        onUpdate(event: XRDragEvent, state: object): void;
        onEnd(event: XRDragEvent, state: object): void;
        onCancel(event: XRDragEvent, state: object): void;
        /** The controller inputs the binding takes from the window under the ray while its drag runs: its trigger's buttons. */
        claims(): readonly string[];
    }
    export interface ViewMoveOptions extends Partial<XRBindingOptions> {
        /** `controller`: the panel follows the hand. `ray`: it follows where the ray points. */
        tracking?: "controller" | "ray";
        rotation?: "follow" | "keep";
        distance?: "preserve";
    }
    /** The 6-DoF grab. It writes the View's placement only. */
    export class ViewMoveBinding extends XRControllerDragBinding {
        constructor(options?: ViewMoveOptions);
        tracking: "controller" | "ray";
        rotation: "follow" | "keep";
    }
    export interface ViewResizeOptions extends Partial<XRBindingOptions> {
        pixelsPerMetre?: number;
        minWidth?: number;
        minHeight?: number;
    }
    /** Resizes a panel by its corner: the View's scale on every update, one new client size on release if needed. */
    export class ViewResizeBinding extends XRControllerDragBinding {
        constructor(options?: ViewResizeOptions);
        pixelsPerMetre: number;
        minWidth: number;
        minHeight: number;
    }

    export interface XRRingParams {
        radius: number;
        eye: number;
        mmPerPx: number;
        focusScale: number;
        focusPull: number;
        gap: number;
        halfArc: number;
        maxPanels: number;
        stackStep: number;
    }
    export const RING_DEFAULTS: Readonly<XRRingParams>;
    export interface RingView<I> {
        readonly id: I;
        readonly width: number;
        readonly height: number;
    }
    export interface RingPose {
        x: number;
        y: number;
        z: number;
        rotation: [number, number, number];
        scale: number;
        focused: boolean;
        behind: number;
    }
    export interface RingResult {
        poses: RingPose[];
        order: number[];
    }
    /** Where each panel goes on the ring, in millimetres; a pure function of the panels' sizes and the focus. */
    export function arrangeRing<I>(
        views: readonly RingView<I>[],
        focus: I | null,
        params?: Readonly<XRRingParams>,
    ): RingResult;

    export interface SpatialLayoutOptions {
        /** Metres from the viewer to a panel's centre. Default 1.6. */
        radius?: number;
        /** Metres from the floor to a panel's centre. Default 1.4. */
        eye?: number;
        /** Radians between neighbouring panels. Default 3 degrees. */
        spacing?: number;
        metresPerPixel?: number;
        focusScale?: number;
        focusPull?: number;
        halfArc?: number;
        maxPanels?: number;
        stackStep?: number;
        /** The triggers of the stock bindings by id; `null` leaves one out. */
        triggers?: { move?: string | null; resize?: string | null };
    }
    /** The room's arrangement and its bindings: `layout.bindings.remove("move")` turns the grab off. */
    export class SpatialLayout {
        constructor(options?: SpatialLayoutOptions);
        readonly bindings: BindingCollection;
        readonly ring: XRRingParams;
        arrange<I>(views: readonly RingView<I>[], focus: I | null): RingResult;
    }

    export interface XRRoomOptions extends NodeOptions {
        layout?: SpatialLayout | null;
        accept?: (client: XRClient) => boolean;
    }
    export interface HeadPanelOptions {
        /** Metres right of, above and in front of the head. Default 0, 0, -1.2. */
        x?: number;
        y?: number;
        z?: number;
        /** Millimetres of panel for a pixel of window. */
        scale?: number;
    }
    export interface XRHit {
        readonly view: View;
        readonly point: Vector3;
        readonly local: { x: number; y: number };
    }
    /** The panel the ray strikes: the room's last View first, because the room lists them back to front. */
    export function pickView(room: XRRoom, ray: Ray): XRHit | null;
    /** The node the XR program reads, with the head camera and one panel for each client. */
    export class XRRoom extends Node {
        constructor(options?: XRRoomOptions);
        /** The camera the program writes the head pose onto: the one named `head`. */
        head: Camera3D;
        layout: SpatialLayout | null;
        accept: (client: XRClient) => boolean;
        /** Metres per room unit. */
        readonly unit: number;
        readonly views: View[];
        viewOf(client: XRClient): View | undefined;
        /** A panel the user moved is pinned: the layout leaves its place alone until `unpin`. */
        pin(view: View): void;
        unpin(view: View): void;
        isPinned(view: View): boolean;
        /** Shows `client` in a panel fixed to the head. */
        pinToHead(client: XRClient, options?: HeadPanelOptions): View;
    }

    export class XRListener {
        onSessionStarted(session: XRSession): void;
        onSessionEnded(session: XRSession): void;
    }
    export class XRSession {
        readonly id: string;
        /** False once the program ended the session. */
        readonly alive: boolean;
        readonly connected: boolean;
        readonly tracking: "tracked" | "untracked";
        readonly head: XRTrackedPose;
        readonly controllers: XRList<XRController>;
        /** The `xr` seat as an input context. */
        readonly input: object | null;
        readonly layerBudget: number;
        viewport: XRViewport | null;
        /**
         * How long, in milliseconds, a chord must have been down before a drag starts, and so before it claims its
         * buttons. Default 50; a value above 60 reads back as 60, because the XR program holds a press back from the
         * window for 150 ms while it waits for the claim.
         */
        chordSettleMs: number;
    }
    export class XRExtension {
        readonly available: true;
        readonly sessions: XRList<XRSession>;
        readonly session: XRSession | null;
        readonly Camera3D: typeof XRCamera3D;
        readonly Viewport: typeof XRViewport;
        readonly SpatialLayout: typeof SpatialLayout;
        readonly ViewMoveBinding: typeof ViewMoveBinding;
        readonly ViewResizeBinding: typeof ViewResizeBinding;
        readonly Room: typeof XRRoom;
        /** The stock room, or the one a script assigned to the viewport. */
        room: XRRoom | null;
    }
}

/** The runtime object, also reachable as `import { gnoblin } from "gnoblin"`. */
declare const gnoblin: import("gnoblin").Gnoblin;
