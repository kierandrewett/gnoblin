// The `gnoblin/host` module, which the user-facing `gnoblin` module re-exports: the object model a config sees,
// over the host functions the Rust shell installs as `globalThis.__gnoblinHost` (crates/gnoblin-js/src/bindings).
//
// Identity: one JS object per tree node for the life of the generation, so `a === b` holds for the
// same client wherever it was reached from. Reads go to the host, which answers from the tree mirror
// and the writes this frame has queued. Writes are queued and travel as one transaction at the end of
// the frame. A destroyed node keeps its last values readable and rejects every write.
//
// Events: the host tells `plan` what changed in the frame. `plan` updates the objects and queues one
// invocation per interested listener; the host then runs them one by one under a time budget each, so a
// listener that throws or never returns does not stop the others.

import { desugarConfig, defaultConfig, mergeConfig, planStockLauncher } from "gnoblin/config";
import type {
    BindingAction,
    ConfigListeners,
    ConfigPlan,
    GnoblinConfig,
    GnoblinHost,
    PlannedKey,
    ResolvedAction,
} from "gnoblin/config";
import { DialogManager } from "gnoblin/dialogs";
import type { DialogEvent } from "gnoblin/dialogs";
import { StripLayout } from "gnoblin/layouts/strip";
import { Layout } from "gnoblin/layout";
import type { Gaps, LayoutContext } from "gnoblin/layout";
import {
    checkGaps,
    checkPlacements,
    checkWorkspaceOptions,
    fieldsChanged,
    initialWorkspace,
    layoutFields,
    rectsEqual,
    resolveLayout,
} from "gnoblin/workspace";
import type { CheckedPlacement, StockLayoutName, WorkspaceOptions } from "gnoblin/workspace";
import { FOCUS_DEFAULTS, FocusPolicy, focusSettings, seatFocusSettings } from "gnoblin/focus";
import type { FocusConfig, FocusSettings } from "gnoblin/focus";
import { InputManager } from "gnoblin/input";
import type { InputHost, InputMessage } from "gnoblin/input";
import { RULE_DEFAULTS, RuleEngine, compileRules } from "gnoblin/rules";
import type { RuleHost, RuleView } from "gnoblin/rules";
import { attachUi, runUiFrame, uiNodeGone, uiReleaseAll } from "gnoblin/ui";
import type { UiBackend } from "gnoblin/ui";
import { defaultTitlebarStyle, Decorations } from "gnoblin/titlebar";
import { inspect } from "gnoblin/inspect";
import { WorkspaceSets } from "gnoblin/sets";
import { snappedZone, unsnapClient } from "gnoblin/snap";
import type { SnapClient } from "gnoblin/snap";
import { windowMenu } from "gnoblin/menus";
import { notRespondingAlerts } from "gnoblin/alerts";
import { barInsets } from "gnoblin/widgets";
import {
    ClientStyle,
    MAX_STYLE_LIST,
    bindStyle,
    layeredList,
    mapStyle,
    replaceStyle,
    resolveStyle,
    styleEpoch,
    syncStyleLists,
} from "gnoblin/style";
import type { ClientStyleInit, ComputedStyle, Style, StyleLayer } from "gnoblin/style";
import { attachScene, sceneForget, sceneReleaseAll } from "gnoblin/scene";
import type { SceneBackend } from "gnoblin/scene";
import { Extensions, XRRuntime } from "gnoblin/xr";
import type { XRWorld } from "gnoblin/xr";

type NodeId = number;
type Scalar = string | number | boolean;
/** What a property holds: a scalar, or the three numbers of a rotation or scale. */
type HostValue = Scalar | number[];

/** A rectangle in the compositor's logical coordinates. */
interface Rect {
    x: number;
    y: number;
    width: number;
    height: number;
}

/** The data-only selection `where` takes: each key is a property, each value what it must equal. */
export type Query = Record<string, Scalar | Scalar[]>;

interface ErrorReport {
    site: string;
    error: unknown;
}

/** What the frame's dialog listeners decided about one observed request. */
interface Verdict {
    request: number;
    deny: boolean;
    reason: string;
}

interface Entry {
    plan(created: NodeId[], touched: NodeId[], destroyed: NodeId[]): number;
    next(): number;
    pending(): number;
    ready(nodes: NodeId[]): number;
    /** Tells the clients' listeners of the pointer's edges on windows; returns how many calls wait. */
    pointer(events: WindowPointerEvent[]): number;
    /** Hands the frame's claimed chords and gestures to the input matcher; returns how many calls wait. */
    input(messages: InputMessage[]): number;
    /** Cancels what the input matcher has in flight and releases every claim; returns how many calls wait. */
    cancelAll(): number;
    takeErrors(): ErrorReport[];
    /** What the module could not do, said once per generation: a style property no renderer draws. */
    takeNotices(): string[];
    /** The config document patch of the running generation, as JSON text: `{}` when it sets none. */
    document(): string;
    /** The commands the running generation's config starts with the session, as JSON text of argv lists. */
    autostart(): string;
    /** What the compositor sent for the dialog observation slot; returns how many calls wait. */
    dialogs(events: DialogEvent[]): number;
    /** The verdicts the frame's listeners gave, each once. */
    takeVerdicts(): Verdict[];
    /** Whether a dialog listener is attached, so the shell holds the observation slot. */
    dialogsWanted(): boolean;
    /** Lays out the workspaces that changed, once each; returns how many it laid out. */
    layout(): number;
    /** Runs the UI toolkit's frame: measurements and pointer events in, every canvas arranged and painted. */
    ui(): void;
    /** Renders a console value for the socket: JSON text of `{ type, value?, text, truncated }`. */
    render(value: unknown, maxBytes?: number): string;
    /** Gives a config that declared no workspace the stock one. */
    finishLoad(): void;
    /** Registers the stock config alone, for a shell that has no user entry to run. */
    loadDefault(): void;
    snapshot(): WorkspaceSnapshot;
    reconcile(previous: WorkspaceSnapshot): void;
}

interface Host {
    /** Checks a document patch (JSON) against the document in force: the validator's problems, or undefined. */
    config: { check(patch: string): string | undefined };
    kind(node: NodeId): string | undefined;
    get(node: NodeId, key: string): HostValue | undefined;
    published(node: NodeId, key: string): HostValue | undefined;
    label(node: NodeId): string | undefined;
    set(node: NodeId, key: string, value: HostValue): string | undefined;
    /** The same write, carried out over `seconds` along `samples` (progress from 0 to 1). */
    animate(node: NodeId, key: string, value: HostValue, seconds: number, samples: number[]): string | undefined;
    call(node: NodeId, method: string): string | undefined;
    /** Queues the moves that put a window on top of the others. */
    raise(node: NodeId): string | undefined;
    /** Queues the moves that put a window under the others. */
    lower(node: NodeId): string | undefined;
    windows(): NodeId[];
    screens(): NodeId[];
    spawn(command: string | string[]): string | undefined;
    input: InputHost;
    ui: Omit<UiBackend, "now">;
    /** The scene nodes the XR module makes and finds (`gnoblin/scene`, `gnoblin/xr`). */
    xr: {
        create(cls: string): NodeId | string;
        insert(node: NodeId, parent: NodeId, index: number): string | undefined;
        destroy(node: NodeId): string | undefined;
        nodes(cls: string): NodeId[];
        children(node: NodeId): NodeId[];
        parent(node: NodeId): NodeId | undefined;
        root(): NodeId;
        declared(): boolean;
    };
    entry?: Entry;
}

const host = (globalThis as { __gnoblinHost?: Host }).__gnoblinHost;
if (!host) {
    throw new Error("the gnoblin module needs the shell host and cannot run without it");
}

/** Only this module can make a Client, a Workspace or a Collection: the compositor owns their lifetime. */
const CREATE = Symbol("gnoblin.create");

/**
 * What a failed operation on a native object throws. `code` says why (`destroyed`, `read-only`,
 * `refused`, `spawn-failed`) and `target` names the object and member, as in `Client#7.x`.
 */
export class GnoblinError extends Error {
    readonly code: string;
    readonly target: string;

    constructor(code: string, target: string, message: string) {
        super(`${target}: ${message}`);
        this.name = "GnoblinError";
        this.code = code;
        this.target = target;
    }
}

/** A pointer edge on a window: where, in the window's own coordinates, and which button (0 for the others). */
export interface ClientPointerEvent {
    readonly x: number;
    readonly y: number;
    readonly button: number;
}

/** A pointer edge as the host hands it over: the node it is on, `enter`, `motion`, `leave`, `press` or `release`. */
interface WindowPointerEvent extends ClientPointerEvent {
    node: NodeId;
    kind: string;
}

/** Observes the events of a client: attach it to one client, or to a collection for every member. */
export class ClientListener {
    onTitleChanged?(client: Client, title: string): void;
    onAppIdChanged?(client: Client, appId: string): void;
    /** The pointer crossed into the window. `event` is in the window's own coordinates. */
    onPointerEnter?(client: Client, event: ClientPointerEvent): void;
    onPointerMotion?(client: Client, event: ClientPointerEvent): void;
    onPointerLeave?(client: Client, event: ClientPointerEvent): void;
    /** A button went down on the window. The window's client hears it too. */
    onPointerPress?(client: Client, event: ClientPointerEvent): void;
    onPointerRelease?(client: Client, event: ClientPointerEvent): void;
    onFocusChanged?(client: Client, focused: boolean): void;
    onGeometryChanged?(client: Client, geometry: Rect): void;
    onClosed?(client: Client): void;
    onReady?(client: Client): void;
}

/** Observes membership: an object joining or leaving a collection. */
export class CollectionListener {
    onAdded?(member: Client | Workspace, collection: Collection<Client | Workspace>): void;
    onRemoved?(member: Client | Workspace, collection: Collection<Client | Workspace>): void;
}

interface Source {
    listeners: object[];
}

/** A call the module itself queues (an input hook), run beside the listeners' under the same budget. */
interface Task {
    site: string;
    task(): void;
}

interface Invocation {
    source: Source;
    listener: object;
    method: string;
    args: unknown[];
}

const queue: (Invocation | Task)[] = [];
let reports: ErrorReport[] = [];

function handlerOf(listener: object, method: string): unknown {
    return Reflect.get(listener, method);
}

function describeListener(listener: object): string {
    const name = Reflect.get(listener, "constructor")?.name;
    return typeof name === "string" && name ? name : "listener";
}

/** Runs one queued call if its listener is still attached; a throw is reported, never propagated. */
function run(call: Invocation | Task): void {
    if ("task" in call) {
        try {
            call.task();
        } catch (error) {
            reports.push({ site: call.site, error });
        }
        return;
    }
    if (!call.source.listeners.includes(call.listener)) {
        return;
    }
    const handler = handlerOf(call.listener, call.method);
    if (typeof handler !== "function") {
        return;
    }
    try {
        handler.apply(call.listener, call.args);
    } catch (error) {
        reports.push({ site: `${describeListener(call.listener)}.${call.method}`, error });
    }
}

function queueFor(source: Source, method: string, args: unknown[]): void {
    for (const listener of source.listeners) {
        if (typeof handlerOf(listener, method) === "function") {
            queue.push({ source, listener, method, args });
        }
    }
}

function assertListener(listener: unknown, target: string): asserts listener is object {
    if (typeof listener !== "object" || listener === null) {
        throw new TypeError(`${target}: a listener must be an object, such as an instance of ClientListener`);
    }
}

/** A native failure code, as an error naming `target`. */
function refuse(code: string, target: string, what: string): never {
    switch (code) {
        case "unknown-node":
            throw new GnoblinError("destroyed", target, `the ${what} is gone`);
        case "read-only":
            throw new GnoblinError("read-only", target, "this property cannot be written");
        case "invalid-value":
            throw new TypeError(`${target}: the value has the wrong type for this property`);
        default:
            throw new GnoblinError("refused", target, `the host refused the write (${code})`);
    }
}

const CLIENT_KEYS = [
    "title",
    "app_id",
    "focused",
    "fullscreen",
    "maximised",
    "minimised",
    "x",
    "y",
    "width",
    "height",
    "tiled",
    "visible",
    "opacity",
    "z",
    "rotation",
    "scale",
    "secure",
];

/** What listeners were last told about a client, to raise an event only for a real change. */
interface Seen {
    title: string;
    appId: string;
    focused: boolean;
    x: number;
    y: number;
    width: number;
    height: number;
}

interface NodeState {
    node: NodeId;
    alive: boolean;
    /** The last values of a destroyed node, which stay readable. */
    frozen: Record<string, HostValue | undefined> | null;
    listeners: object[];
    readied: boolean;
    seen: Seen | null;
    /** The script's own flag: a floating client is left out of its workspace's tiling. */
    floating: boolean;
}

const states = new WeakMap<object, NodeState>();

function stateOf(object: object): NodeState {
    const state = states.get(object);
    if (!state) {
        throw new TypeError("not an object the compositor made");
    }
    return state;
}

function read<T extends Scalar>(object: object, key: string): T {
    const state = stateOf(object);
    return (state.frozen ? state.frozen[key] : host.get(state.node, key)) as T;
}

function freeze(object: object, keys: string[]): void {
    const state = stateOf(object);
    const frozen: Record<string, HostValue | undefined> = {};
    for (const key of keys) {
        frozen[key] = host.get(state.node, key);
    }
    state.frozen = frozen;
    state.alive = false;
}

/** What the compositor last published for the properties that raise events. */
function publishedValues(node: NodeId): Seen {
    return {
        title: host.published(node, "title") as string,
        appId: host.published(node, "app_id") as string,
        focused: host.published(node, "focused") as boolean,
        x: host.published(node, "x") as number,
        y: host.published(node, "y") as number,
        width: host.published(node, "width") as number,
        height: host.published(node, "height") as number,
    };
}

/** The queued write of one number property, or the error that names the member. */
function writeNumber(client: Client, key: string, value: unknown): void {
    const state = stateOf(client);
    const target = `Client#${host.label(state.node)}.${key}`;
    if (!state.alive) {
        throw new GnoblinError("destroyed", target, "the client is gone");
    }
    if (typeof value !== "number" || !Number.isFinite(value)) {
        throw new TypeError(`${target}: must be a finite number`);
    }
    const code = host.set(state.node, key, value);
    if (code != null) {
        refuse(code, target, "client");
    }
}

/** The queued write of one flag property, or the error that names the member. */
function writeFlag(client: Client, key: string, value: unknown): void {
    const state = stateOf(client);
    const target = `Client#${host.label(state.node)}.${key}`;
    if (!state.alive) {
        throw new GnoblinError("destroyed", target, "the client is gone");
    }
    if (typeof value !== "boolean") {
        throw new TypeError(`${target}: must be true or false`);
    }
    const code = host.set(state.node, key, value);
    if (code != null) {
        refuse(code, target, "client");
    }
}

type Vector3 = readonly [number, number, number];

/** The queued write of a property of three numbers (`rotation`, `scale`), or the error that names the member. */
function writeVector(client: Client, key: string, value: unknown): void {
    const state = stateOf(client);
    const target = `Client#${host.label(state.node)}.${key}`;
    if (!state.alive) {
        throw new GnoblinError("destroyed", target, "the client is gone");
    }
    if (
        !Array.isArray(value) ||
        value.length !== 3 ||
        value.some((n) => typeof n !== "number" || !Number.isFinite(n))
    ) {
        throw new TypeError(`${target}: must be three finite numbers`);
    }
    const code = host.set(state.node, key, [value[0], value[1], value[2]]);
    if (code != null) {
        refuse(code, target, "client");
    }
}

/** How an animation moves from its start to its end, as the Lua shell's `ease` table named them. */
export type ClientEasing = "linear" | "out" | "out-cubic" | "in-cubic" | "in-out";

const EASINGS: Readonly<Record<ClientEasing, (t: number) => number>> = {
    linear: (t) => t,
    out: (t) => 1 - (1 - t) ** 2,
    "out-cubic": (t) => 1 - (1 - t) ** 3,
    "in-cubic": (t) => t ** 3,
    "in-out": (t) => (t < 0.5 ? 2 * t * t : 1 - (2 - 2 * t) ** 2 / 2),
};

/** The curve is sent as this many steps plus the start: 33 samples, as the Lua shell's `node:to` sends. */
const CURVE_STEPS = 32;

/** The properties of a client `animate` can move, and the kind of value each takes. */
const ANIMATED_KEYS: Readonly<Record<string, "number" | "vector">> = {
    x: "number",
    y: "number",
    width: "number",
    height: "number",
    opacity: "number",
    z: "number",
    rotation: "vector",
    scale: "vector",
};

/** What `Client.animate` takes: the values to move to, each of the kind its property holds. */
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
    /** Default `"out"`, as the Lua shell's. */
    easing?: ClientEasing;
}

function callMethod(client: Client, method: string): void {
    const state = stateOf(client);
    const target = `Client#${host.label(state.node)}.${method}()`;
    if (!state.alive) {
        throw new GnoblinError("destroyed", target, "the client is gone");
    }
    const code = host.call(state.node, method);
    if (code != null) {
        refuse(code, target, "client");
    }
}

const clientsByNode = new Map<NodeId, Client>();
/** The workspace each client is on. A workspace is the script's, so the compositor does not know. */
const membership = new WeakMap<Client, Workspace>();
/** The clients that were there when the shell booted: a layout does not place them again. */
const preexisting = new WeakSet<Client>();

/** A toplevel window the compositor manages. */
export class Client {
    constructor(token: symbol, node: NodeId) {
        if (token !== CREATE) {
            throw new TypeError("a Client is made by the compositor, not by a script");
        }
        states.set(this, {
            node,
            alive: true,
            frozen: null,
            listeners: [],
            readied: false,
            seen: publishedValues(node),
            floating: false,
        });
    }

    get id(): string {
        return host.label(stateOf(this).node) ?? "?";
    }

    get alive(): boolean {
        return stateOf(this).alive;
    }

    get appId(): string {
        return read<string>(this, "app_id");
    }

    get title(): string {
        return read<string>(this, "title");
    }

    get focused(): boolean {
        return read<boolean>(this, "focused");
    }

    /** Whether the window covers its output. Writing asks the compositor to make it fullscreen or restore it. */
    get fullscreen(): boolean {
        return read<boolean>(this, "fullscreen");
    }

    set fullscreen(value: boolean) {
        writeFlag(this, "fullscreen", value);
    }

    /** Whether the window is kept out of screen captures. */
    get secure(): boolean {
        return read<boolean>(this, "secure");
    }

    set secure(value: boolean) {
        writeFlag(this, "secure", value);
    }

    /** Whether the client has left a liveness ping unanswered too long. The compositor writes it, and clears it on an answer. */
    get unresponsive(): boolean {
        return read<boolean>(this, "unresponsive");
    }

    /**
     * The overrides this client has, and only those: a property that was never set here is `undefined` and comes from
     * the scopes above (`gnoblin.clients.style`, the workspace's, a `where()` selection's). Reading a list this client
     * does not override gives a copy of the inherited one; the first write to the copy makes it this client's own.
     */
    get style(): ClientStyle {
        return clientStyle(this);
    }

    /** The style this client resolves to now: frozen, with the layer each value came from in `origin`. */
    get computedStyle(): ComputedStyle {
        return computedStyleOf(this);
    }

    /** What of the client's style is not drawn and why, and the effects the compositor skipped. */
    get styleWarnings(): readonly string[] {
        return styleWarningsOf(this);
    }

    /**
     * The room a titlebar takes above the window's content, in pixels: 0 for a window with no titlebar or a fullscreen one.
     * `bounds` is the content's rectangle; the frame a layout places is the content with this much added on top (see
     * docs/js-ui.md, "Titlebar").
     */
    get frameInset(): number {
        return decorations?.insetOf(this) ?? 0;
    }

    /**
     * Whether the window floats on a workspace whose layout tiles: a floating client is not given to the layout's
     * `compute`, so it keeps the place it has. Only the script knows this; the compositor does not.
     */
    get floating(): boolean {
        return stateOf(this).floating;
    }

    set floating(value: boolean) {
        if (typeof value !== "boolean") {
            throw new TypeError(`Client#${host.label(stateOf(this).node)}.floating: must be true or false`);
        }
        const state = stateOf(this);
        if (state.floating !== value) {
            state.floating = value;
            const own = wsOf(this);
            if (own) {
                own.dirty = true;
            }
        }
    }

    get x(): number {
        return read<number>(this, "x");
    }

    set x(value: number) {
        writeNumber(this, "x", value);
    }

    get y(): number {
        return read<number>(this, "y");
    }

    set y(value: number) {
        writeNumber(this, "y", value);
    }

    get width(): number {
        return read<number>(this, "width");
    }

    set width(value: number) {
        writeNumber(this, "width", value);
    }

    get height(): number {
        return read<number>(this, "height");
    }

    set height(value: number) {
        writeNumber(this, "height", value);
    }

    /** The layout geometry. Assigning writes all four values, or none when one is not a finite number. */
    get bounds(): Rect {
        return { x: this.x, y: this.y, width: this.width, height: this.height };
    }

    set bounds(rect: Rect) {
        const target = `Client#${host.label(stateOf(this).node)}.bounds`;
        for (const key of ["x", "y", "width", "height"] as const) {
            if (typeof rect?.[key] !== "number" || !Number.isFinite(rect[key])) {
                throw new TypeError(`${target}.${key}: must be a finite number`);
            }
        }
        this.x = rect.x;
        this.y = rect.y;
        this.width = rect.width;
        this.height = rect.height;
    }

    /** Whether the window is maximised. Writing asks the compositor to maximise or restore it. */
    get maximised(): boolean {
        return read<boolean>(this, "maximised");
    }

    set maximised(value: boolean) {
        writeFlag(this, "maximised", value);
    }

    /** Whether the window is minimised. Writing asks the compositor to minimise or restore it. */
    get minimised(): boolean {
        return read<boolean>(this, "minimised");
    }

    set minimised(value: boolean) {
        writeFlag(this, "minimised", value);
    }

    /** Whether the window sits in a tiled layout, so its client drops the border and rounding it draws itself. */
    get tiled(): boolean {
        return read<boolean>(this, "tiled");
    }

    set tiled(value: boolean) {
        writeFlag(this, "tiled", value);
    }

    /** Whether the window is drawn and hit. A workspace that is not shown hides its windows. */
    get visible(): boolean {
        return read<boolean>(this, "visible");
    }

    set visible(value: boolean) {
        writeFlag(this, "visible", value);
    }

    /** How opaque the window is drawn, from 0 to 1 (the compositor clamps it). */
    get opacity(): number {
        return read<number>(this, "opacity");
    }

    set opacity(value: number) {
        writeNumber(this, "opacity", value);
    }

    /** The window's place in depth; negative is in front of the camera. */
    get z(): number {
        return read<number>(this, "z");
    }

    set z(value: number) {
        writeNumber(this, "z", value);
    }

    /** Rotation about each axis in radians. Assigning takes three numbers. */
    get rotation(): Vector3 {
        return read<never>(this, "rotation") as Vector3;
    }

    set rotation(value: Vector3) {
        writeVector(this, "rotation", value);
    }

    /** Scale along each axis. Assigning takes three numbers. */
    get scale(): Vector3 {
        return read<never>(this, "scale") as Vector3;
    }

    set scale(value: Vector3) {
        writeVector(this, "scale", value);
    }

    /** The workspace the client is on, or null. */
    get workspace(): Workspace | null {
        return membership.get(this) ?? null;
    }

    addListener(listener: ClientListener): void {
        assertListener(listener, `Client#${host.label(stateOf(this).node)}.addListener`);
        const { listeners } = stateOf(this);
        if (!listeners.includes(listener)) {
            listeners.push(listener);
        }
    }

    removeListener(listener: ClientListener): void {
        const { listeners } = stateOf(this);
        const at = listeners.indexOf(listener);
        if (at >= 0) {
            listeners.splice(at, 1);
        }
    }

    /** Asks the client to close. The client is `alive` until the compositor destroys the window. */
    close(): void {
        callMethod(this, "close");
    }

    /** Gives the window the keyboard focus: a write of the seat's `focus`, which is how a shell hands it out. */
    focus(): void {
        callMethod(this, "focus");
    }

    /**
     * Moves properties to `values` over `options.duration` seconds along the easing curve. Reading a property back
     * at once gives the end value; the compositor draws the way there. A duration of 0 writes at once.
     */
    animate(values: ClientAnimation, options: AnimateOptions): void {
        const state = stateOf(this);
        const target = `Client#${host.label(state.node)}.animate()`;
        if (!state.alive) {
            throw new GnoblinError("destroyed", target, "the client is gone");
        }
        const duration = options?.duration;
        if (typeof duration !== "number" || !Number.isFinite(duration) || duration < 0) {
            throw new TypeError(`${target}: duration must be a finite number of seconds of at least 0`);
        }
        const curve = EASINGS[options.easing ?? "out"];
        if (curve === undefined) {
            throw new TypeError(`${target}: easing must be one of ${Object.keys(EASINGS).join(", ")}`);
        }
        const entries = Object.entries(values).sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0));
        for (const [key, value] of entries) {
            const kind = ANIMATED_KEYS[key];
            if (kind === undefined) {
                throw new TypeError(`${target}: ${key} cannot be animated`);
            }
            if (kind === "number") {
                if (typeof value !== "number" || !Number.isFinite(value)) {
                    throw new TypeError(`${target}: ${key} must be a finite number`);
                }
            } else if (
                !Array.isArray(value) ||
                value.length !== 3 ||
                value.some((n) => typeof n !== "number" || !Number.isFinite(n))
            ) {
                throw new TypeError(`${target}: ${key} must be three finite numbers`);
            }
        }
        const samples = Array.from({ length: CURVE_STEPS + 1 }, (_, i) => curve(i / CURVE_STEPS));
        for (const [key, value] of entries) {
            const sent = Array.isArray(value) ? [value[0], value[1], value[2]] : (value as number);
            const code =
                duration === 0
                    ? host.set(state.node, key, sent)
                    : host.animate(state.node, key, sent, duration, samples);
            if (code != null) {
                refuse(code, target, "client");
            }
        }
    }

    /** Puts the window on top of every other window. */
    raise(): void {
        const state = stateOf(this);
        const target = `Client#${host.label(state.node)}.raise()`;
        if (!state.alive) {
            throw new GnoblinError("destroyed", target, "the client is gone");
        }
        const code = host.raise(state.node);
        if (code != null) {
            refuse(code, target, "client");
        }
    }

    /** Puts the window under every other window. */
    lower(): void {
        const state = stateOf(this);
        const target = `Client#${host.label(state.node)}.lower()`;
        if (!state.alive) {
            throw new GnoblinError("destroyed", target, "the client is gone");
        }
        const code = host.lower(state.node);
        if (code != null) {
            refuse(code, target, "client");
        }
    }

    /**
     * Kills the process that owns the window with SIGKILL: the not-responding alert's Force quit. The compositor refuses,
     * and does nothing, when the owner is unknown, has gone, changed since the window mapped, or is the compositor itself
     * or init.
     */
    kill(): void {
        callMethod(this, "kill");
    }

    toString(): string {
        return `Client#${host.label(stateOf(this).node)}`;
    }
}

interface WorkspaceState {
    id: string;
    name: string;
    layout: Layout;
    gaps: Gaps;
    /** Whether the workspace is in `gnoblin.workspaces`. */
    attached: boolean;
    clients: MutableCollection<Client>;
    clientsState: CollectionState;
    /** The clients that joined and left since the layout last heard of it, for `onClientAdded` and `onClientRemoved`. */
    arrived: Client[];
    departed: Client[];
    /** Whether the layout has to compute again the next time the workspace is shown. */
    dirty: boolean;
    /** The layout's own fields when the workspace was last looked at, to notice a change. */
    fields: Map<PropertyKey, unknown> | null;
    gapsSeen: [number, number];
    /** The area the layout last computed for. */
    area: Rect | null;
    /** The focused client the layout last computed for: a change of focus is a reason to compute again. */
    focusSeen: Client | null;
}

const wsStates = new WeakMap<object, WorkspaceState>();

function wsState(workspace: unknown): WorkspaceState {
    const state = typeof workspace === "object" && workspace !== null ? wsStates.get(workspace) : undefined;
    if (!state) {
        throw new TypeError("not a Workspace");
    }
    return state;
}

/**
 * A place windows are laid out in. The script makes it and adds it to `gnoblin.workspaces`; `clients` holds
 * its windows, the same `Client` objects as `gnoblin.clients`. Removing a workspace, or a window from one,
 * never closes the window.
 */
export class Workspace {
    constructor(options: WorkspaceOptions) {
        const checked = checkWorkspaceOptions(options);
        const clients = new MutableCollection<Client>(CREATE, {
            ...rootState("Client"),
            mutation: {
                add: (client) => transferClient(this, client as Client),
                remove: (client) => detachClient(this, client as Client),
                clear: () => clearClients(this),
            },
        });
        wsStates.set(this, {
            id: checked.id,
            name: checked.name,
            layout: checked.layout,
            gaps: checked.gaps,
            attached: false,
            clients,
            clientsState: collectionState(clients),
            arrived: [],
            departed: [],
            dirty: true,
            focusSeen: null,
            fields: null,
            gapsSeen: [checked.gaps.inner, checked.gaps.outer],
            area: null,
        });
    }

    get id(): string {
        return wsState(this).id;
    }

    get name(): string {
        return wsState(this).name;
    }

    set name(value: string) {
        if (typeof value !== "string") {
            throw new TypeError(`${this}.name must be a string`);
        }
        wsState(this).name = value;
    }

    /** A workspace is the script's own and stays usable after it leaves `gnoblin.workspaces`. */
    get alive(): boolean {
        return true;
    }

    /** The position in `gnoblin.workspaces`, or -1 while it is not in it. */
    get index(): number {
        return workspaceRoot.list.indexOf(this);
    }

    /** Whether the primary display shows this workspace. */
    get active(): boolean {
        return activeWorkspace === this;
    }

    get layout(): Layout {
        return wsState(this).layout;
    }

    /** A layout instance, or the name of a stock layout. The windows stay where they are until it computes. */
    set layout(value: Layout | StockLayoutName) {
        const state = wsState(this);
        state.layout = resolveLayout(value, `${this}`);
        state.fields = null;
        state.dirty = true;
    }

    get gaps(): Gaps {
        return wsState(this).gaps;
    }

    set gaps(value: { inner?: number; outer?: number }) {
        const state = wsState(this);
        state.gaps = checkGaps(value, `${this}.gaps`);
        state.dirty = true;
    }

    get clients(): Collection<Client> {
        return wsState(this).clients;
    }

    /** Shows this workspace on the primary display. */
    show(): void {
        showWorkspace(this);
    }

    toString(): string {
        return `Workspace#${wsState(this).id}`;
    }
}

type Member = Client | Workspace;

interface Mutation {
    add(member: Member): void;
    remove(member: Member): void;
    clear(): void;
}

interface CollectionState extends Source {
    self: Collection<Member>;
    /** The collection a `where` narrows; null for a root collection, which holds `list`. */
    parent: CollectionState | null;
    query: Query | null;
    list: Member[];
    /** The members a view last told its listeners of, kept only while it has listeners. */
    members: Set<Member> | null;
    keys: readonly string[];
    /** Set on a collection scripts may change: what `add`, `remove` and `clear` do. */
    mutation?: Mutation;
    /** What a `where()` selection ranks by among the selections that style a client: higher wins. */
    priority?: number;
}

const collections = new WeakMap<object, CollectionState>();
/** The views with listeners: the only ones whose membership the plan keeps up to date. */
const tracked: CollectionState[] = [];

function collectionState(collection: object): CollectionState {
    const state = collections.get(collection);
    if (!state) {
        throw new TypeError("not a gnoblin collection");
    }
    return state;
}

const QUERY_KEYS: Record<string, readonly string[]> = {
    Client: ["id", "appId", "title", "focused", "fullscreen", "x", "y", "width", "height"],
    Workspace: ["id", "name", "index", "active"],
};

function matches(query: Query, member: Member): boolean {
    for (const [key, want] of Object.entries(query)) {
        const have = Reflect.get(member, key);
        if (Array.isArray(want) ? !want.includes(have) : have !== want) {
            return false;
        }
    }
    return true;
}

/** Whether `member` belongs to the collection now, worked out from the objects and not from what was told. */
function contains(state: CollectionState, member: Member): boolean {
    if (state.parent === null) {
        return state.list.includes(member);
    }
    return contains(state.parent, member) && matches(state.query as Query, member);
}

function itemsOf(state: CollectionState): Member[] {
    if (state.parent === null) {
        return state.list.slice();
    }
    const query = state.query as Query;
    return itemsOf(state.parent).filter((member) => matches(query, member));
}

function rootOf(state: CollectionState): CollectionState {
    return state.parent === null ? state : rootOf(state.parent);
}

/** A live, array-like view of the compositor's objects. Not an Array, and its membership is not writable. */
export class Collection<T extends Member> implements Iterable<T> {
    constructor(token: symbol, state: Omit<CollectionState, "self">) {
        if (token !== CREATE) {
            throw new TypeError("a Collection is made by the compositor, not by a script");
        }
        collections.set(this, { ...state, self: this as unknown as Collection<Member> });
    }

    get length(): number {
        const state = collectionState(this);
        return state.parent === null ? state.list.length : itemsOf(state).length;
    }

    at(index: number): T | undefined {
        return itemsOf(collectionState(this)).at(index) as T | undefined;
    }

    /**
     * The member that holds the keyboard focus, or null: the first seat's focus, and with no seat the member whose
     * own `focused` is set. Only clients take the focus, so another collection's is null.
     */
    get focused(): T | null {
        return focusedAmong(itemsOf(collectionState(this))) as T | null;
    }

    get(id: string): T | undefined {
        return itemsOf(collectionState(this)).find((member) => member.id === id) as T | undefined;
    }

    find(predicate: (item: T) => boolean): T | undefined {
        return (itemsOf(collectionState(this)) as T[]).find((item) => predicate(item));
    }

    /** A snapshot: the array does not follow later changes. */
    filter(predicate: (item: T) => boolean): T[] {
        return (itemsOf(collectionState(this)) as T[]).filter((item) => predicate(item));
    }

    /** A snapshot, like `filter`. */
    map<U>(mapper: (item: T) => U): U[] {
        return (itemsOf(collectionState(this)) as T[]).map((item) => mapper(item));
    }

    /**
     * A live selection: it follows additions, removals and changes of the properties the query names. The
     * query is data, so the compositor side can evaluate it too: each key is a property and each value is
     * what it must equal, or an array of values of which one must. `priority` ranks the selection's `style`
     * among the others a client is in: the higher wins, and among equals the one whose style was made later.
     */
    where(query: Query, options: { priority?: number } = {}): Collection<T> {
        const state = collectionState(this);
        if (typeof query !== "object" || query === null || Array.isArray(query)) {
            throw new TypeError("where: the query must be an object of property values");
        }
        for (const [key, want] of Object.entries(query)) {
            if (!state.keys.includes(key)) {
                throw new TypeError(`where: no property \`${key}\` to select on (known: ${state.keys.join(", ")})`);
            }
            const values = Array.isArray(want) ? want : [want];
            if (!values.every((value) => ["string", "number", "boolean"].includes(typeof value))) {
                throw new TypeError(`where: \`${key}\` must be a string, number or boolean, or an array of them`);
            }
        }
        const priority = options?.priority ?? 0;
        if (typeof priority !== "number" || !Number.isFinite(priority)) {
            throw new TypeError("where: priority must be a finite number");
        }
        return new Collection<T>(CREATE, {
            parent: state,
            query: { ...query },
            list: [],
            members: null,
            keys: state.keys,
            listeners: [],
            priority,
        });
    }

    /**
     * The defaults this collection of clients gives: `gnoblin.clients.style` to all of them, a workspace's `clients` to
     * its own, a `where()` selection to the clients in it. Only the properties set here are overridden; see `Client.style`.
     * Assigning a `ClientStyle` or an object of properties replaces what this scope sets.
     */
    get style(): ClientStyle {
        return scopeStyle(collectionState(this)).style;
    }

    set style(value: ClientStyle | ClientStyleInit) {
        replaceStyle(scopeStyle(collectionState(this)).style, value);
    }

    /**
     * Subscribes `listener` to this collection. A `CollectionListener` is told of every member now, as if
     * each had just been added (replay on attach, so a config that reloads sees the clients that are
     * already there), and of every later addition and removal. A `ClientListener` is attached to every
     * current and future member and detaches from a member that leaves. `removeListener` undoes both.
     */
    addListener(listener: ClientListener | CollectionListener): void {
        assertListener(listener, "Collection.addListener");
        const state = collectionState(this);
        if (state.listeners.includes(listener)) {
            return;
        }
        state.listeners.push(listener);
        if (state.parent !== null && state.members === null) {
            state.members = new Set(itemsOf(state));
            tracked.push(state);
        }
        if (typeof handlerOf(listener, "onAdded") === "function") {
            for (const member of itemsOf(state)) {
                run({ source: state, listener, method: "onAdded", args: [member, this] });
            }
        }
    }

    removeListener(listener: ClientListener | CollectionListener): void {
        const state = collectionState(this);
        const at = state.listeners.indexOf(listener);
        if (at < 0) {
            return;
        }
        state.listeners.splice(at, 1);
        if (state.parent !== null && state.listeners.length === 0 && state.members !== null) {
            state.members = null;
            tracked.splice(tracked.indexOf(state), 1);
        }
    }

    [Symbol.iterator](): Iterator<T> {
        return (itemsOf(collectionState(this)) as T[])[Symbol.iterator]();
    }
}

/**
 * A collection a script may change. `gnoblin.workspaces` and a workspace's `clients` are these;
 * `gnoblin.clients` is not, because the compositor owns which windows exist.
 */
export class MutableCollection<T extends Member> extends Collection<T> {
    private mutation(what: string): Mutation {
        const { mutation } = collectionState(this);
        if (!mutation) {
            throw new GnoblinError("read-only", `Collection.${what}`, "this collection's membership cannot be changed");
        }
        return mutation;
    }

    /** Adds a member. On a workspace's `clients` this moves the client there from its workspace. */
    add(member: T): void {
        this.mutation("add").add(member);
    }

    /** Removes a member, given as itself or by id. A member that is not in the collection is ignored. */
    remove(member: T | string): void {
        const target = typeof member === "string" ? this.get(member) : member;
        if (target !== undefined && collectionState(this).list.includes(target)) {
            this.mutation("remove").remove(target);
        }
    }

    clear(): void {
        this.mutation("clear").clear();
    }
}

/** The workspaces of the script, which also say which of them the primary display shows. */
class WorkspaceCollection extends MutableCollection<Workspace> {
    /** Named groups of workspaces to show together, each entry on an output. */
    readonly sets = new WorkspaceSets({
        declared: () => new Set((workspaceRoot.list as Workspace[]).map((workspace) => workspace.id)),
        outputs: () => {
            const screens = host.screens();
            const names = screens.map((id) => String(host.get(id, "name") ?? ""));
            const primary = screens.find((id) => host.get(id, "primary") === true) ?? screens[0];
            return { names, primary: primary === undefined ? "" : String(host.get(primary, "name") ?? "") };
        },
        show: (id) => showWorkspace(id, { duration: SLIDE_SECONDS }),
        report: (message) => {
            reports.push({ site: "gnoblin.workspaces.sets", error: new Error(message) });
        },
    });

    /** The area windows are laid out in, which is the primary screen; null while the tree has none. */
    get area(): Rect | null {
        return screenArea();
    }

    /**
     * Shows a workspace on the primary display, given as itself or by id. With `options.duration` the windows slide
     * from one workspace to the other; without it the switch is at once.
     */
    show(workspace: Workspace | string, options?: ShowOptions): void {
        showWorkspace(workspace, options);
    }
}

function rootState(kind: string): Omit<CollectionState, "self"> {
    return {
        parent: null,
        query: null,
        list: [],
        members: null,
        keys: QUERY_KEYS[kind],
        listeners: [],
    };
}

const clientRoot = collectionState(new Collection<Client>(CREATE, rootState("Client")));
const workspaceRoot = collectionState(
    new WorkspaceCollection(CREATE, {
        ...rootState("Workspace"),
        mutation: {
            add: (workspace) => addWorkspace(workspace as Workspace),
            remove: (workspace) => removeWorkspace(workspace as Workspace),
            clear: () => {
                for (const workspace of [...workspaceRoot.list]) {
                    removeWorkspace(workspace as Workspace);
                }
            },
        },
    }),
);

/** The collections that hold `member` now: its root and every tracked view with it as a member. */
function holders(top: CollectionState, member: Member): CollectionState[] {
    return [top, ...tracked.filter((view) => view.members?.has(member) && rootOf(view) === top)];
}

/** Tells every tracked view of `top` that `member` joined or left, when its query says so. */
function recompute(top: CollectionState, member: Member): void {
    for (const view of tracked) {
        if (rootOf(view) !== top || view.members === null) {
            continue;
        }
        const was = view.members.has(member);
        const is = contains(view, member);
        if (was && !is) {
            view.members.delete(member);
            queueFor(view, "onRemoved", [member, view.self]);
        } else if (!was && is) {
            view.members.add(member);
            queueFor(view, "onAdded", [member, view.self]);
        }
    }
}

function emitMember(client: Client, held: CollectionState[], method: string, args: unknown[]): void {
    queueFor(stateOf(client), method, args);
    for (const state of held) {
        queueFor(state, method, args);
    }
}

function wsOf(client: Client): WorkspaceState | null {
    const workspace = membership.get(client);
    return workspace ? wsState(workspace) : null;
}

/** Every collection that holds `client` now: `gnoblin.clients`, its workspace's `clients`, and their live selections. */
function clientHolders(client: Client): CollectionState[] {
    const own = wsOf(client);
    const all = holders(clientRoot, client);
    return own ? [...all, ...holders(own.clientsState, client)] : all;
}

/** The workspace the primary display shows, or null when there is none. */
let activeWorkspace: Workspace | null = null;
/**
 * False until the first frame: the entry module is still adding workspaces, and the windows that were
 * there at boot join one only when it is done, or a reload could not put them back where they were.
 */
let opened = false;

function guard(site: string, body: () => void): void {
    try {
        body();
    } catch (error) {
        reports.push({ site, error });
    }
}

/** Puts `client` in `workspace`, which it is not in. */
function link(workspace: Workspace, client: Client, arrived: boolean): void {
    const state = wsState(workspace);
    membership.set(client, workspace);
    state.clientsState.list.push(client);
    state.dirty = true;
    if (arrived) {
        state.arrived.push(client);
    }
    queueFor(state.clientsState, "onAdded", [client, state.clientsState.self]);
    recompute(state.clientsState, client);
}

/** Takes `client`, which is alive, out of `workspace`. */
function unlink(workspace: Workspace, client: Client): void {
    const state = wsState(workspace);
    membership.delete(client);
    state.clientsState.list.splice(state.clientsState.list.indexOf(client), 1);
    state.dirty = true;
    forgetArrival(state, client);
    queueFor(state.clientsState, "onRemoved", [client, state.clientsState.self]);
    recompute(state.clientsState, client);
}

/** A client that leaves before its layout heard of it never was there for the layout. */
function forgetArrival(state: WorkspaceState, client: Client): void {
    const at = state.arrived.indexOf(client);
    if (at >= 0) {
        state.arrived.splice(at, 1);
    } else {
        state.departed.push(client);
    }
}

/** Moves `client` to `target` (or out of every workspace when it is null). `arrived` says a layout may place it. */
function moveClient(client: Client, target: Workspace | null, arrived: boolean): void {
    const from = membership.get(client) ?? null;
    if (from === target) {
        return;
    }
    if (from) {
        unlink(from, client);
    }
    if (target) {
        link(target, client, arrived);
    }
}

/** `workspace.clients.add(client)`: the client leaves its workspace and joins this one. */
function transferClient(workspace: Workspace, client: Client): void {
    const target = `${workspace}.clients.add`;
    if (!wsState(workspace).attached) {
        throw new GnoblinError("detached", target, "add the workspace to gnoblin.workspaces first");
    }
    if (!(client instanceof Client) || !clientsByNode.has(stateOf(client).node)) {
        throw new GnoblinError("destroyed", target, "the client is gone or is not a client of the compositor");
    }
    moveClient(client, workspace, true);
}

/** `workspace.clients.remove(client)`: the client has no workspace until it is given one. */
function detachClient(workspace: Workspace, client: Client): void {
    if (membership.get(client) === workspace) {
        moveClient(client, null, false);
    }
}

function clearClients(workspace: Workspace): void {
    for (const client of [...wsState(workspace).clientsState.list] as Client[]) {
        detachClient(workspace, client);
    }
}

/** Makes `next` the workspace the primary display shows. */
function setActive(next: Workspace | null): void {
    const before = activeWorkspace;
    if (before === next) {
        return;
    }
    activeWorkspace = next;
    if (next) {
        wsState(next).dirty = true;
    }
    for (const changed of [before, next]) {
        if (changed && wsState(changed).attached) {
            recompute(workspaceRoot, changed);
        }
    }
}

/** Seconds a workspace slide takes, as the Lua shell's `Desk:show`. */
export const SLIDE_SECONDS = 0.25;

/** How `show` moves to a workspace. */
export interface ShowOptions {
    /** Seconds the slide between the two workspaces takes. Zero or absent switches at once. */
    duration?: number;
    /** How much of the slide a touchpad gesture has made already, 0 to 1: the slide carries on from there. */
    progress?: number;
}

/** A slide the next layout pass starts: the windows of `from` leave and the windows of `to` come in. */
interface Slide {
    from: Workspace;
    to: Workspace;
    /** 1 when `to` comes after `from`, so the windows move left. */
    direction: 1 | -1;
    duration: number;
    progress: number;
}

const slides: Slide[] = [];
/** The workspaces that are not shown but keep their windows drawn until a slide away from them ends. */
const holding = new Map<Workspace, number>();

/** `gnoblin.workspaces.show(workspace, options)`. */
function showWorkspace(workspace: Workspace | string, options: ShowOptions = {}): void {
    const target = typeof workspace === "string" ? workspaceRoot.list.find((w) => w.id === workspace) : workspace;
    if (target === undefined) {
        throw new GnoblinError("unknown-workspace", "gnoblin.workspaces.show", `there is no workspace "${workspace}"`);
    }
    if (!wsState(target).attached) {
        throw new GnoblinError("detached", `${target}.show`, "add the workspace to gnoblin.workspaces first");
    }
    const duration = options.duration ?? 0;
    const progress = options.progress ?? 0;
    if (typeof duration !== "number" || !Number.isFinite(duration) || duration < 0) {
        throw new TypeError(`${target}.show: duration must be a finite number of seconds of at least 0`);
    }
    if (typeof progress !== "number" || !(progress >= 0 && progress <= 1)) {
        throw new TypeError(`${target}.show: progress must be a number from 0 to 1`);
    }
    const from = activeWorkspace;
    setActive(target as Workspace);
    if (from !== null && from !== target && duration > 0) {
        slides.push({
            from,
            to: target as Workspace,
            direction: workspaceRoot.list.indexOf(target) > workspaceRoot.list.indexOf(from) ? 1 : -1,
            duration,
            progress,
        });
    }
}

/** Work that waits for a time: the end of a slide. Run at the start of each layout pass. */
const timers: { at: number; run: () => void }[] = [];

function after(seconds: number, run: () => void): void {
    if (typeof seconds !== "number" || !Number.isFinite(seconds) || seconds < 0) {
        throw new TypeError("gnoblin.after: seconds must be a finite number of at least 0");
    }
    if (typeof run !== "function") {
        throw new TypeError("gnoblin.after: needs a function to run");
    }
    timers.push({ at: host.input.now() + seconds * 1000, run });
}

function runTimers(): void {
    if (timers.length === 0) {
        return;
    }
    const now = host.input.now();
    const due = timers.filter((timer) => timer.at <= now);
    for (const timer of due) {
        timers.splice(timers.indexOf(timer), 1);
        guard("gnoblin.after", timer.run);
    }
}

/** Gives each window that has no workspace the one the initial-workspace rule names. */
function adoptOrphans(): void {
    if (!opened) {
        return;
    }
    for (const client of [...clientRoot.list] as Client[]) {
        if (membership.has(client)) {
            continue;
        }
        const target = initialWorkspace(workspaceRoot.list as Workspace[]);
        if (target === null) {
            return;
        }
        moveClient(client, target, !preexisting.has(client));
    }
}

function addWorkspace(workspace: Workspace): void {
    const state = wsState(workspace);
    if (state.attached) {
        return;
    }
    if (workspaceRoot.list.some((other) => other.id === state.id)) {
        throw new GnoblinError(
            "duplicate",
            "gnoblin.workspaces.add",
            `there is already a workspace with id "${state.id}"`,
        );
    }
    state.attached = true;
    state.dirty = true;
    workspaceRoot.list.push(workspace);
    queueFor(workspaceRoot, "onAdded", [workspace, workspaceRoot.self]);
    recompute(workspaceRoot, workspace);
    if (activeWorkspace === null) {
        setActive(workspace);
    }
    adoptOrphans();
}

/** Takes the workspace out of `gnoblin.workspaces`. Its windows move to the first workspace left; none is closed. */
function removeWorkspace(workspace: Workspace): void {
    const state = wsState(workspace);
    if (!state.attached) {
        return;
    }
    const held = holders(workspaceRoot, workspace);
    state.attached = false;
    workspaceRoot.list.splice(workspaceRoot.list.indexOf(workspace), 1);
    for (const view of held) {
        view.members?.delete(workspace);
        queueFor(view, "onRemoved", [workspace, view.self]);
    }
    const target = (workspaceRoot.list[0] as Workspace | undefined) ?? null;
    if (activeWorkspace === workspace) {
        activeWorkspace = null;
        setActive(target);
    }
    for (const client of [...state.clientsState.list] as Client[]) {
        moveClient(client, target, true);
    }
}

/** A config that declared no workspace gets the stock one, so there is always somewhere for a window to go. */
function ensureDefaultWorkspace(): void {
    if (workspaceRoot.list.length > 0) {
        return;
    }
    for (const planned of desugarConfig(defaultConfig).workspaces) {
        addWorkspace(new Workspace({ id: planned.id, layout: planned.layout, gaps: planned.gaps }));
    }
}

/** The area windows are laid out in: the size of the primary screen. Null when the tree has no screen yet. */
function screenArea(): Rect | null {
    const screens = host.screens();
    const node = screens.find((id) => host.get(id, "primary") === true) ?? screens[0];
    if (node === undefined) {
        return null;
    }
    const width = host.get(node, "width");
    const height = host.get(node, "height");
    if (typeof width !== "number" || typeof height !== "number" || !(width > 0) || !(height > 0)) {
        return null;
    }
    const insets = barInsets();
    return {
        x: 0,
        y: insets.top,
        width,
        height: Math.max(1, height - insets.top - insets.bottom),
    };
}

/** Writes the parts of `rect` the client does not have already. */
function writeBounds(client: Client, rect: Rect): void {
    for (const key of ["x", "y", "width", "height"] as const) {
        if (client[key] !== rect[key]) {
            writeNumber(client, key, rect[key]);
        }
    }
}

/**
 * The content's rectangle for a frame: the titlebar lies above the content, so the frame's top edge is the bar's and the
 * content starts a bar lower. A client with no titlebar is the frame.
 */
function framed(client: Client, frame: Rect): Rect {
    const inset = client.frameInset;
    if (!(inset > 0)) {
        return frame;
    }
    return { x: frame.x, y: frame.y + inset, width: frame.width, height: Math.max(0, frame.height - inset) };
}

function sameVector(a: Vector3, b: Vector3): boolean {
    return a[0] === b[0] && a[1] === b[1] && a[2] === b[2];
}

/** Whether each client's layout wants it shown, from the last placement it had. Default shown. */
const shownByLayout = new WeakMap<Client, boolean>();
/**
 * The `visible` this module last decided for each client. A client the module never hid is shown. It is what the
 * module compares with, so a window the compositor hid itself (a minimised one) is not shown by a layout that has not
 * changed its mind.
 */
const writtenVisible = new WeakMap<Client, boolean>();

/** Decides `shown` for `client`, and writes it unless the client already has that value (a reloaded shell's windows). */
function writeShown(client: Client, shown: boolean): void {
    writtenVisible.set(client, shown);
    if (client.visible !== shown) {
        client.visible = shown;
    }
}

/**
 * Writes what a layout's placements say beyond the rectangle: whether the window is tiled, hidden, and where it is in
 * depth. A client the layout gave no placement is as if placed with the defaults, so a window that leaves a tiled
 * layout for a floating one stops being tiled. Only what differs from the client's value is written.
 */
function applyPlacements(children: readonly Client[], placements: readonly CheckedPlacement<Client>[]): void {
    const placed = new Map(placements.map((placement) => [placement.node, placement]));
    for (const client of children) {
        const placement = placed.get(client);
        if (placement?.bounds) {
            writeBounds(client, framed(client, placement.bounds));
        }
        const tiled = placement?.tiled ?? false;
        if (client.tiled !== tiled) {
            client.tiled = tiled;
        }
        shownByLayout.set(client, placement?.visible ?? true);
        const z = placement?.z ?? 0;
        if (client.z !== z) {
            client.z = z;
        }
        const rotation = placement?.rotation ?? [0, 0, 0];
        if (!sameVector(client.rotation, rotation)) {
            client.rotation = rotation;
        }
        const scale = placement?.scale ?? [1, 1, 1];
        if (!sameVector(client.scale, scale)) {
            client.scale = scale;
        }
    }
}

/**
 * Shows the windows of the shown workspace (and of a workspace a slide is leaving) that their layout shows, and hides
 * the rest. A window is written only when what it should be differs from what this module last wrote.
 */
function syncVisibility(): void {
    for (const workspace of workspaceRoot.list as Workspace[]) {
        const shown = workspace === activeWorkspace || holding.has(workspace);
        for (const client of wsState(workspace).clientsState.list as Client[]) {
            const want = shown && (shownByLayout.get(client) ?? true);
            if (client.alive && (writtenVisible.get(client) ?? true) !== want) {
                writeShown(client, want);
            }
        }
    }
}

/**
 * Starts the slides `show` asked for, once the layout has put every window at rest: the windows of the workspace
 * that comes in start a screen's width away on the side it comes from and move to where the layout put them, and
 * the windows of the one that leaves move a screen's width the other way. The leaving windows stay drawn until
 * the slide ends, then go back to where they rest, hidden.
 */
function startSlides(area: Rect | null): void {
    for (const slide of slides.splice(0)) {
        if (area === null) {
            continue;
        }
        const { from, to, direction, duration, progress } = slide;
        const travel = direction * area.width;
        for (const client of wsState(to).clientsState.list as Client[]) {
            const rest = client.x;
            client.x = rest + travel * (1 - progress);
            client.animate({ x: rest }, { duration, easing: "out-cubic" });
        }
        const resting = new Map<Client, number>();
        for (const client of wsState(from).clientsState.list as Client[]) {
            const rest = client.x;
            resting.set(client, rest);
            if (progress > 0) {
                client.x = rest - travel * progress;
            }
            client.animate({ x: rest - travel }, { duration, easing: "out-cubic" });
        }
        holding.set(from, (holding.get(from) ?? 0) + 1);
        after(duration, () => {
            const left = (holding.get(from) ?? 1) - 1;
            if (left > 0) {
                holding.set(from, left);
                return;
            }
            holding.delete(from);
            for (const [client, rest] of resting) {
                if (client.alive && membership.get(client) === from && activeWorkspace !== from) {
                    writeShown(client, false);
                    client.x = rest;
                }
            }
        });
    }
}

// ---- styles ------------------------------------------------------------------------------------------------------

/** The style a collection of clients carries, and where it ranks among the selections. */
interface ScopeStyle {
    style: ClientStyle;
    /** The order the scope's style was first asked for in: the tie-break between selections of equal priority. */
    order: number;
}

const scopeStyles = new WeakMap<CollectionState, ScopeStyle>();
/** The `where()` selections that have a style, in the order they got it. */
const styledSelections: CollectionState[] = [];
let styleOrder = 0;

/** What a client's style last resolved from and wrote to its node. */
interface ClientStyleState {
    /** The client's own overrides, made the first time a script reads `client.style`. */
    style: ClientStyle | null;
    applied: boolean;
    /** The style epoch and the layers the last application was made for. */
    epoch: number;
    layers: string;
    computed: ComputedStyle | null;
    notes: string[];
    /** The effects last written, as JSON; null before the first write. */
    effects: string | null;
    /** The opacity last written, or the node's own when nothing was written yet. */
    opacity: number | null;
}

const clientStyles = new WeakMap<Client, ClientStyleState>();
/** The texts `takeNotices` has not handed out yet, and every text already said this generation. */
const notices: string[] = [];
const noticed = new Set<string>();

function say(text: string): void {
    if (!noticed.has(text)) {
        noticed.add(text);
        notices.push(text);
    }
}

function styleState(client: Client): ClientStyleState {
    let state = clientStyles.get(client);
    if (state === undefined) {
        state = {
            style: null,
            applied: false,
            epoch: -1,
            layers: "",
            computed: null,
            notes: [],
            effects: null,
            opacity: null,
        };
        clientStyles.set(client, state);
    }
    return state;
}

function workspaceOfScope(state: CollectionState): Workspace | undefined {
    return (workspaceRoot.list as Workspace[]).find((workspace) => wsState(workspace).clientsState === state);
}

/** `gnoblin.clients`, `Workspace#dev.clients`, and `.where({...})` after either: the front of a scope's error paths. */
function scopePath(state: CollectionState): string {
    if (state.parent !== null) {
        return `${scopePath(state.parent)}.where(${JSON.stringify(state.query)})`;
    }
    return state === clientRoot ? "gnoblin.clients" : `Workspace#${workspaceOfScope(state)?.id ?? "?"}.clients`;
}

/** The name `computedStyle.origin` gives a scope. */
function scopeOrigin(state: CollectionState): string {
    if (state.parent !== null) {
        return scopePath(state).replace(/^gnoblin\.clients\./, "");
    }
    return state === clientRoot ? "global" : `workspace:${workspaceOfScope(state)?.id ?? "?"}`;
}

/** The layers of the scopes above a collection's own: what a list it does not override is inherited from. */
function layersAbove(state: CollectionState): StyleLayer[] {
    const layers: StyleLayer[] = [];
    const global = scopeStyles.get(clientRoot);
    if (global !== undefined && state !== clientRoot) {
        layers.push({ origin: "global", style: global.style });
    }
    const root = rootOf(state);
    const scope = root === clientRoot ? undefined : scopeStyles.get(root);
    if (scope !== undefined && state !== root) {
        layers.push({ origin: scopeOrigin(root), style: scope.style });
    }
    return layers;
}

/** The scope of `state`, made when first asked for. */
function scopeStyle(state: CollectionState): ScopeStyle {
    if (state.keys !== QUERY_KEYS.Client) {
        throw new TypeError("style: only a collection of clients has a style");
    }
    let scope = scopeStyles.get(state);
    if (scope === undefined) {
        const style = new ClientStyle();
        scope = { style, order: styleOrder };
        styleOrder += 1;
        scopeStyles.set(state, scope);
        if (state.parent !== null) {
            styledSelections.push(state);
        }
        bindStyle(style, {
            path: () => `${scopePath(state)}.style`,
            below: (name) => layeredList(layersAbove(state), name),
        });
    }
    return scope;
}

/** The scopes a client sits under, lowest precedence first: global, its workspace, then its `where()` selections. */
function scopeLayers(client: Client): StyleLayer[] {
    const layers: StyleLayer[] = [];
    const global = scopeStyles.get(clientRoot);
    if (global !== undefined) {
        layers.push({ origin: "global", style: global.style });
    }
    const own = wsOf(client);
    const workspace = own === null ? undefined : scopeStyles.get(own.clientsState);
    if (own !== null && workspace !== undefined) {
        layers.push({ origin: `workspace:${own.id}`, style: workspace.style });
    }
    const selected = styledSelections
        .filter((state) => contains(state, client))
        .sort(
            (a, b) =>
                (a.priority ?? 0) - (b.priority ?? 0) ||
                (scopeStyles.get(a) as ScopeStyle).order - (scopeStyles.get(b) as ScopeStyle).order,
        );
    for (const state of selected) {
        layers.push({ origin: scopeOrigin(state), style: (scopeStyles.get(state) as ScopeStyle).style });
    }
    return layers;
}

function clientStyle(client: Client): ClientStyle {
    const state = styleState(client);
    if (state.style === null) {
        const style = new ClientStyle();
        const path = (): string => `Client#${host.label(stateOf(client).node)}.style`;
        bindStyle(style, {
            path,
            below: (name) => layeredList(scopeLayers(client), name),
            guard: () => {
                if (!stateOf(client).alive) {
                    throw new GnoblinError("destroyed", path(), "the client is gone");
                }
            },
        });
        state.style = style;
    }
    return state.style;
}

/** `scopeLayers` and the client's own overrides on top. */
function layersOf(client: Client): StyleLayer[] {
    const own = clientStyles.get(client)?.style;
    const layers = scopeLayers(client);
    return own === null || own === undefined ? layers : [...layers, { origin: "client", style: own }];
}

function computedStyleOf(client: Client): ComputedStyle {
    if (!stateOf(client).alive) {
        return clientStyles.get(client)?.computed ?? resolveStyle([]);
    }
    return resolveStyle(layersOf(client));
}

function styleWarningsOf(client: Client): readonly string[] {
    const notes = (clientStyles.get(client)?.notes ?? []).map((note) => `not applied: style.${note}`);
    const skipped = (host.get(stateOf(client).node, "skipped") as unknown as string[] | undefined) ?? [];
    return Object.freeze([...notes, ...skipped.map((text) => `the compositor does not draw the effect ${text}`)]);
}

/** Every style a script can have changed an array of in place. */
function* everyStyle(): Generator<Style> {
    const scopes = [clientRoot, ...(workspaceRoot.list as Workspace[]).map((w) => wsState(w).clientsState)];
    for (const scope of [...scopes, ...styledSelections]) {
        const found = scopeStyles.get(scope);
        if (found !== undefined) {
            yield found.style;
        }
    }
    for (const client of clientRoot.list as Client[]) {
        const own = clientStyles.get(client)?.style;
        if (own !== null && own !== undefined) {
            yield own;
        }
    }
}

/** Writes what a client's resolved style becomes on its node: the effects list and the opacity, when they changed. */
function applyClientStyle(client: Client, state: ClientStyleState, layers: StyleLayer[]): void {
    const node = stateOf(client).node;
    const label = `Client#${host.label(node)}.style`;
    const computed = resolveStyle(layers);
    state.computed = computed;
    const mapping = mapStyle(computed);
    state.notes = mapping.unapplied;
    for (const text of mapping.unapplied) {
        say(`not applied: style.${text}`);
    }
    const refused = (what: string, code: string | undefined | null): void => {
        if (code != null && code !== "unknown-node") {
            reports.push({
                site: label,
                error: new GnoblinError("refused", label, `the host refused the ${what} (${code})`),
            });
        }
    };
    if (mapping.effects.length > MAX_STYLE_LIST) {
        const error = new RangeError(
            `${label}: a window carries at most ${MAX_STYLE_LIST} effects, the style makes ${mapping.effects.length}`,
        );
        state.notes = [...state.notes, error.message];
        reports.push({ site: label, error });
    } else {
        const json = JSON.stringify(mapping.effects);
        // A window an earlier generation styled keeps its effects: the first application clears them when the style
        // has none, which is how a line removed from the config takes its effect away on reload.
        const published = host.published(node, "effects") as unknown as unknown[] | undefined;
        const unchanged =
            state.effects === null
                ? mapping.effects.length === 0 && (published ?? []).length === 0
                : json === state.effects;
        if (!unchanged) {
            refused("effects", host.set(node, "effects", mapping.effects as unknown as HostValue));
        }
        state.effects = json;
    }
    const before = state.opacity ?? (host.published(node, "opacity") as number);
    if (computed.opacity !== before) {
        refused("opacity", host.set(node, "opacity", computed.opacity));
    }
    state.opacity = computed.opacity;
}

/**
 * Once per frame: checks the arrays scripts changed in place, then resolves and writes the style of each client whose
 * style or layers moved. A client is looked at again when any style changed (the epoch) or the scopes it sits under
 * did (a `where()` selection it joined or left, a workspace move); one the pass already did and nothing moved for
 * costs a comparison.
 */
function applyStyles(): void {
    for (const style of everyStyle()) {
        for (const error of syncStyleLists(style)) {
            reports.push({ site: "gnoblin.style", error });
        }
    }
    const epoch = styleEpoch();
    for (const client of clientRoot.list as Client[]) {
        const state = styleState(client);
        const layers = layersOf(client);
        const key = layers.map((layer) => layer.origin).join("\n");
        if (state.applied && state.epoch === epoch && state.layers === key) {
            continue;
        }
        state.applied = true;
        state.epoch = epoch;
        state.layers = key;
        guard(`Client#${host.label(stateOf(client).node)}.style`, () => applyClientStyle(client, state, layers));
    }
}

/**
 * The layout driver, run once at the end of each frame after every listener. For each workspace it first
 * notices what changed: a layout's own enumerable fields (a snapshot diff, because a field is a plain
 * property the host never sees written), the workspace's gaps, and the area. Then the layout hears of the
 * windows that came and went, and the shown workspace, when it is dirty, computes once and its placements
 * are written. A workspace that is not shown stays dirty until it is.
 */
function layout(): number {
    runTimers();
    const area = screenArea();
    let laidOut = 0;
    for (const workspace of workspaceRoot.list as Workspace[]) {
        const state = wsState(workspace);
        guard(`${workspace}.layout`, () => {
            const fields = layoutFields(state.layout);
            if (fieldsChanged(state.fields, fields)) {
                state.dirty = true;
            }
            if (state.gaps.inner !== state.gapsSeen[0] || state.gaps.outer !== state.gapsSeen[1]) {
                state.gapsSeen = [state.gaps.inner, state.gaps.outer];
                state.dirty = true;
            }
            state.fields = fields;
            if (area === null) {
                return;
            }
            const children = [...state.clientsState.list] as Client[];
            const focused = children.find((client) => client.focused) ?? null;
            if (focused !== state.focusSeen) {
                state.focusSeen = focused;
                state.dirty = true;
            }
            const context: LayoutContext<Client> = {
                workspace,
                children,
                area: { ...area },
                gap: { ...state.gaps },
                focused,
            };
            const target = state.layout;
            for (const client of state.departed.splice(0)) {
                guard(`${workspace}.layout.onClientRemoved`, () => target.onClientRemoved?.(client, context));
            }
            for (const client of state.arrived.splice(0)) {
                if (client.alive && membership.get(client) === workspace) {
                    guard(`${workspace}.layout.onClientAdded`, () => target.onClientAdded?.(client, context));
                }
            }
            if (activeWorkspace === workspace && (state.dirty || !rectsEqual(state.area, area))) {
                const before = state.area;
                state.dirty = false;
                state.area = { ...area };
                if (before !== null && !rectsEqual(before, area)) {
                    guard(`${workspace}.layout.onAreaChanged`, () => target.onAreaChanged?.(before, { ...area }));
                }
                guard(`${workspace}.layout.compute`, () => {
                    const site = `${workspace}.layout.compute`;
                    // A client the script floats (a window rule, `client.floating`) keeps its own place.
                    const tiled = children.filter((client) => !stateOf(client).floating);
                    applyPlacements(
                        tiled,
                        checkPlacements(target.compute({ ...context, children: tiled }), tiled, site),
                    );
                });
                laidOut += 1;
            }
            // What the hooks and compute did to the layout's own fields is not a change for the next frame.
            state.fields = layoutFields(state.layout);
        });
    }
    startSlides(area);
    guard("gnoblin.workspaces.visibility", syncVisibility);
    guard("gnoblin.style", applyStyles);
    return laidOut;
}

interface WorkspaceSnapshot {
    active: string | null;
    workspaces: { id: string; clients: number[] }[];
}

/** Which window is on which workspace, for the next generation to reconcile against. */
function snapshotWorkspaces(): WorkspaceSnapshot {
    return {
        active: activeWorkspace?.id ?? null,
        workspaces: (workspaceRoot.list as Workspace[]).map((workspace) => ({
            id: workspace.id,
            clients: (wsState(workspace).clientsState.list as Client[]).map((client) => stateOf(client).node),
        })),
    };
}

/**
 * Puts the windows back on the workspaces of the same id. A window whose workspace this generation does not
 * declare moves to the first workspace; one that stays is not offered to the layout as an arrival, so a
 * floating window keeps the place the user gave it.
 */
function reconcileWorkspaces(previous: WorkspaceSnapshot): void {
    for (const entry of previous.workspaces) {
        const kept = (workspaceRoot.list as Workspace[]).find((workspace) => workspace.id === entry.id) ?? null;
        const target = kept ?? (workspaceRoot.list[0] as Workspace | undefined) ?? null;
        for (const node of entry.clients) {
            const client = clientsByNode.get(node);
            if (client) {
                moveClient(client, target, kept === null);
            }
        }
    }
    const shown = (workspaceRoot.list as Workspace[]).find((workspace) => workspace.id === previous.active);
    if (shown) {
        setActive(shown);
    }
}

/**
 * Wraps a node the first time it is seen. A node that appears while the shell runs is announced: its
 * collections are told it was added and its listeners that it is ready. A node that was there at boot is
 * not an event, because the config attaches after it exists.
 */
function adopt(node: NodeId, announce: boolean): void {
    const kind = host.kind(node);
    if (kind === "window" && !clientsByNode.has(node)) {
        const client = new Client(CREATE, node);
        clientsByNode.set(node, client);
        clientRoot.list.push(client);
        if (announce) {
            queueFor(clientRoot, "onAdded", [client, clientRoot.self]);
            recompute(clientRoot, client);
            stateOf(client).readied = true;
            emitMember(client, holders(clientRoot, client), "onReady", [client]);
            // A window that appears joins the workspace the primary display shows.
            const target = opened ? initialWorkspace(workspaceRoot.list as Workspace[]) : null;
            if (target !== null) {
                moveClient(client, target, true);
            }
        } else {
            preexisting.add(client);
        }
    }
}

/** Removes a destroyed member from its collections, after freezing its last values. */
function leave(top: CollectionState, member: Member, map: Map<NodeId, Member>, keys: string[]): CollectionState[] {
    const node = stateOf(member).node;
    const held = holders(top, member);
    freeze(member, keys);
    map.delete(node);
    top.list.splice(top.list.indexOf(member), 1);
    for (const state of held) {
        state.members?.delete(member);
    }
    return held;
}

function release(node: NodeId): void {
    const client = clientsByNode.get(node);
    if (!client) {
        return;
    }
    const own = wsOf(client);
    const ownHeld = own ? holders(own.clientsState, client) : [];
    const held = leave(clientRoot, client, clientsByNode, CLIENT_KEYS);
    input.clientGone(client);
    if (own) {
        membership.delete(client);
        own.clientsState.list.splice(own.clientsState.list.indexOf(client), 1);
        own.dirty = true;
        forgetArrival(own, client);
        for (const state of ownHeld) {
            state.members?.delete(client);
        }
    }
    const all = [...held, ...ownHeld];
    emitMember(client, all, "onClosed", [client]);
    for (const state of all) {
        queueFor(state, "onRemoved", [client, state.self]);
    }
}

/** Raises the events one client's changed properties imply, to the listeners that held it before the change. */
function changed(client: Client): void {
    const state = stateOf(client);
    const seen = state.seen as Seen;
    const now = publishedValues(state.node);
    const held = clientHolders(client);
    if (now.title !== seen.title) {
        seen.title = now.title;
        emitMember(client, held, "onTitleChanged", [client, now.title]);
    }
    if (now.appId !== seen.appId) {
        seen.appId = now.appId;
        emitMember(client, held, "onAppIdChanged", [client, now.appId]);
    }
    if (now.focused !== seen.focused) {
        seen.focused = now.focused;
        emitMember(client, held, "onFocusChanged", [client, now.focused]);
    }
    if (now.x !== seen.x || now.y !== seen.y || now.width !== seen.width || now.height !== seen.height) {
        const { x, y, width, height } = now;
        Object.assign(seen, { x, y, width, height });
        emitMember(client, held, "onGeometryChanged", [client, { x, y, width, height }]);
    }
}

function plan(created: NodeId[], touched: NodeId[], destroyed: NodeId[]): number {
    // The destroyed list is deepest first. Departures go before arrivals so a collection never holds a
    // dead member while it is told of a new one.
    for (const node of destroyed) {
        uiNodeGone(node);
        release(node);
    }
    sceneForget(destroyed);
    for (const node of created) {
        adopt(node, true);
    }
    for (const node of new Set(touched)) {
        const client = clientsByNode.get(node);
        if (client) {
            changed(client);
            recompute(clientRoot, client);
            const own = wsOf(client);
            if (own) {
                recompute(own.clientsState, client);
            }
        }
    }
    return queue.length;
}

/** The listener method that hears each pointer edge on a window. */
const POINTER_METHODS: Readonly<Record<string, string>> = {
    enter: "onPointerEnter",
    motion: "onPointerMotion",
    leave: "onPointerLeave",
    press: "onPointerPress",
    release: "onPointerRelease",
};

/** Queues the clients' pointer listeners for the edges the compositor reported on windows this frame. */
function pointer(events: WindowPointerEvent[]): number {
    for (const event of events) {
        const client = clientsByNode.get(event.node);
        const method = POINTER_METHODS[event.kind];
        if (client && method && stateOf(client).alive) {
            const { x, y, button } = event;
            emitMember(client, clientHolders(client), method, [client, { x, y, button }]);
        }
    }
    return queue.length;
}

function pending(): number {
    return queue.length;
}

function next(): number {
    const call = queue.shift();
    if (call) {
        run(call);
    }
    return queue.length;
}

/** Queues `onReady` for the clients that were there at boot, parents first, once the config has loaded. */
function ready(nodes: NodeId[]): number {
    ensureDefaultWorkspace();
    opened = true;
    adoptOrphans();
    for (const node of nodes) {
        const client = clientsByNode.get(node);
        if (client && !stateOf(client).readied) {
            stateOf(client).readied = true;
            emitMember(client, clientHolders(client), "onReady", [client]);
        }
    }
    return queue.length;
}

function takeErrors(): ErrorReport[] {
    const taken = reports;
    reports = [];
    return taken;
}

/** What the config says about the seat's settings: a null leaves what the compositor published. */
const seatOverrides: { edgeTiling: boolean | null; dragConstraint: string | null } = {
    edgeTiling: null,
    dragConstraint: null,
};

/** The host's input functions, with the config's `edgeTiling` and `dragConstraint` over the seat's own. */
const seatHost: InputHost = {
    ...host.input,
    dragConstraint: (seat) => seatOverrides.dragConstraint ?? host.input.dragConstraint(seat),
    settings(seat) {
        const published = host.input.settings(seat);
        return published && seatOverrides.edgeTiling !== null
            ? { ...published, edgeTiling: seatOverrides.edgeTiling }
            : published;
    },
};

const input = new InputManager(seatHost, {
    clientOf: (node) => clientsByNode.get(node),
    stack: () => host.windows().flatMap((node) => clientsByNode.get(node) ?? []),
    workArea: screenArea,
    layouts: () => workspaceRoot.list.map((workspace) => wsState(workspace as Workspace).layout),
    schedule: (site, task) => {
        queue.push({ site, task });
    },
    report: (site, error) => {
        reports.push({ site, error });
    },
    error: (code, target, message) => new GnoblinError(code, target, message),
});

attachUi({ ...host.ui, now: () => host.input.now() }, (site, error) => {
    reports.push({ site, error });
});

/** Where a window that a rule made fullscreen or maximised was, so turning the rule off puts it back. */
const beforeCover = new WeakMap<Client, Rect>();

/** Fullscreen or maximised: the flag, and the area the window fills. A fullscreen window does not maximise. */
function cover(client: Client, key: "fullscreen" | "maximised", on: boolean): void {
    if (on) {
        if (key === "maximised" && client.fullscreen) {
            return;
        }
        if (!beforeCover.has(client)) {
            beforeCover.set(client, client.bounds);
        }
        client[key] = true;
        const area = screenArea();
        if (area !== null) {
            writeBounds(client, framed(client, area));
        }
        return;
    }
    client[key] = false;
    const before = beforeCover.get(client);
    if (before !== undefined && !client.fullscreen && !client.maximised) {
        beforeCover.delete(client);
        writeBounds(client, before);
    }
}

/**
 * Whether the first seat holds its keyboard focus on `client`. The stock policy and the rules read the seat, not
 * `client.focused`: a shell gives focus by writing the seat's `focus`, and a real compositor was seen not to keep a
 * window's own `focused` current while a shell holds the tree.
 */
function hasFocus(client: Client): boolean {
    const seat = host.input.seats()[0];
    return seat !== undefined && clientsByNode.get(host.input.focus(seat)) === client;
}

/**
 * The member of `list` that holds the keyboard focus. With a seat it is the one the seat's focus names; a tree with
 * no seat has no seat focus to read, so the member whose own `focused` is set is the answer.
 */
function focusedAmong(list: readonly Member[]): Member | null {
    const seat = host.input.seats()[0];
    if (seat === undefined) {
        return list.find((member) => member instanceof Client && member.focused) ?? null;
    }
    const held = clientsByNode.get(host.input.focus(seat));
    return list.find((member) => member === held) ?? null;
}

/** What a window rule reads and writes of a client: the host's end of `RuleEngine`. */
const ruleHost: RuleHost<Client> = {
    view: (client): RuleView => ({
        appId: client.appId,
        title: client.title,
        focused: hasFocus(client),
        workspace: client.workspace?.id ?? null,
    }),
    alive: (client) => client.alive,
    isArrival: (client) => !preexisting.has(client),
    moveToWorkspace(client, id) {
        const target = workspaceRoot.list.find((workspace) => workspace.id === id);
        if (target === undefined) {
            return false;
        }
        transferClient(target as Workspace, client);
        return true;
    },
    setFloating: (client, on) => {
        client.floating = on;
    },
    setSize: (client, size) => {
        (client as unknown as { size?: { width: number; height: number } }).size = size;
    },
    setBounds: (client, rect) => {
        client.bounds = rect;
        (client as unknown as { floatingBounds?: Rect }).floatingBounds = { ...rect };
    },
    bounds: (client) => client.bounds,
    setFullscreen: (client, on) => cover(client, "fullscreen", on),
    setMaximised: (client, on) => cover(client, "maximised", on),
    setSecure: (client, on) => {
        client.secure = on;
    },
    setOpacity: (client, value) => {
        // A rule's opacity is the client's own style, the highest layer; the value an undone rule restores (1) is
        // the style going back to inheriting.
        if (value === RULE_DEFAULTS.opacity) {
            client.style.unset("opacity");
        } else {
            client.style.opacity = value;
        }
    },
    setStyle: (client, field, value) => {
        client.style[field] = value as never;
    },
};

/** The config's `focus` keys, over the seat's settings, over the documented defaults. */
let focusOverrides: FocusConfig = {};

function currentFocusSettings(): FocusSettings {
    const seat = host.input.seats()[0];
    const published = seat === undefined ? undefined : seatHost.settings(seat);
    const base = published === undefined ? FOCUS_DEFAULTS : seatFocusSettings(published);
    return focusSettings(focusOverrides, "config.focus", base);
}

/** The stock window rules and focus policy, once a config is registered. */
let stock: { rules: RuleEngine<Client>; focus: FocusPolicy<Client> } | null = null;

function installStock(plan: ConfigPlan): void {
    if (stock !== null) {
        clientRoot.self.removeListener(stock.rules);
        clientRoot.self.removeListener(stock.focus);
    }
    seatOverrides.edgeTiling = plan.edgeTiling;
    seatOverrides.dragConstraint = plan.dragConstraint;
    focusOverrides = plan.focus;
    const rules = new RuleEngine(compileRules(plan.rules), ruleHost);
    const focus = new FocusPolicy<Client>({
        settings: currentFocusSettings,
        focused: hasFocus,
        // A tree with no seat has nowhere to put the focus.
        focus: (client) => {
            if (host.input.seats().length > 0) {
                client.focus();
            }
        },
        clearFocus: () => input.primary.clearFocus(),
        raise: (client) => client.raise(),
        now: () => host.input.now(),
        isArrival: (client) => !preexisting.has(client),
        skipsFocus: (client) => rules.skipsFocus(client),
        alive: (client) => client.alive,
    });
    stock = { rules, focus };
    // The rules hear a new window first: `skipFocus` is what the focus policy reads when it hears the same one.
    clientRoot.self.addListener(rules);
    clientRoot.self.addListener(focus);
}

/** The stock titlebars, once a config asks for them. */
let decorations: Decorations | null = null;

function decorate(): void {
    if (decorations === null) {
        decorations = new Decorations(
            (client) => stateOf(client as Client).node,
            { ...defaultTitlebarStyle },
            {
                menu: (client) => {
                    if (windowMenu.started) {
                        windowMenu.show(client as Client);
                    }
                },
                lower: (client) => (client as Client).lower(),
                maximise: (client, on) => cover(client as Client, "maximised", on),
            },
        );
        clientRoot.self.addListener(decorations);
    }
}

/** The verdicts the dialog listeners gave since the shell last took them. */
let verdicts: Verdict[] = [];

const dialogs = new DialogManager({
    schedule: (site, task) => {
        queue.push({ site, task });
    },
    verdict: (request, deny, reason) => {
        verdicts.push({ request, deny, reason });
    },
});

/** The config document patch this generation declared, as JSON text. The shell commits it once the generation runs. */
let documentPatch = "{}";

/** The commands this generation's config starts with the session, as JSON text of argv lists. */
let autostartList = "[]";

/**
 * The `on` handlers of a config as listeners of `gnoblin.clients`: a membership listener for `clientAdded` and
 * `clientRemoved`, a client listener for `titleChanged` and `focusChanged`. A listener a config does not give is
 * not attached, so no call is queued for an event nobody asked about.
 */
function attachOn(on: ConfigListeners): void {
    if (on.clientAdded !== undefined || on.clientRemoved !== undefined) {
        const members = new CollectionListener();
        if (on.clientAdded !== undefined) {
            members.onAdded = (member) => {
                if (member instanceof Client) {
                    on.clientAdded?.(member);
                }
            };
        }
        if (on.clientRemoved !== undefined) {
            members.onRemoved = (member) => {
                if (member instanceof Client) {
                    on.clientRemoved?.(member);
                }
            };
        }
        clientRoot.self.addListener(members);
    }
    if (on.titleChanged !== undefined || on.focusChanged !== undefined) {
        const clients = new ClientListener();
        if (on.titleChanged !== undefined) {
            clients.onTitleChanged = (client, title) => on.titleChanged?.(client, title);
        }
        if (on.focusChanged !== undefined) {
            clients.onFocusChanged = (client, focused) => on.focusChanged?.(client, focused);
        }
        clientRoot.self.addListener(clients);
    }
}

/** Refuses a config whose document settings the validator would refuse, with the key path in the message. */
function checkDocument(patch: ConfigPlan["document"]): void {
    if (Object.keys(patch).length === 0) {
        return;
    }
    const problems = host.config.check(JSON.stringify(patch));
    if (problems != null) {
        throw new GnoblinError("invalid-config", "defineConfig", problems);
    }
}

/** The window menu, once a config asks for it: it answers the compositor's menu requests. */
function startMenus(): void {
    if (windowMenu.started) {
        return;
    }
    windowMenu.start({
        clientOf: (node) => clientsByNode.get(node),
        workspaces: () => workspaceRoot.list as Workspace[],
        moveToWorkspace: (client, workspace) => transferClient(workspace as Workspace, client as Client),
        stack: () => host.windows().flatMap((node) => clientsByNode.get(node) ?? []),
        raise: (client) => (client as Client).raise(),
        maximise: (client, on) => cover(client as Client, "maximised", on),
        leaveFullscreen: (client) => cover(client as Client, "fullscreen", false),
        snapped: (client) => snappedZone(client) !== null,
        unsnap: (client) => {
            unsnapClient(client as unknown as SnapClient);
        },
    });
}

/** The not-responding alert, once a config asks for it. */
function startAlerts(): void {
    if (!notRespondingAlerts.started) {
        notRespondingAlerts.start({
            clients: () => clientRoot.list as Client[],
            closeMenu: () => {
                windowMenu.close();
            },
        });
    }
}

/** The merged config of the last `__register`, which is what the inspector reports as the effective one. */
let effectiveConfig: GnoblinConfig | null = null;

/** The scene the XR module makes its nodes in: the host's, with clients as the content a View shows. */
const sceneBackend: SceneBackend = {
    create: (cls) => host.xr.create(cls),
    insert: (node, parent, index) => host.xr.insert(node, parent, index),
    destroy: (node) => host.xr.destroy(node),
    children: (node) => host.xr.children(node),
    parent: (node) => host.xr.parent(node),
    root: () => host.xr.root(),
    get: (node, key) => host.get(node, key),
    set: (node, key, value) => host.set(node, key, value),
    nodeOf: (content) => (content instanceof Client ? stateOf(content).node : undefined),
    contentOf: (node) => clientsByNode.get(node),
};
attachScene(sceneBackend);

/** The seat that is not the XR program's: the keyboard and pointer of the desktop. */
function mainSeat(): number | undefined {
    return host.input.seats().find((seat) => host.input.seatName(seat) !== "xr");
}

const xrWorld: XRWorld = {
    scene: sceneBackend,
    nodes: (cls) => host.xr.nodes(cls),
    kind: (node) => host.kind(node),
    seats: () => host.input.seats().map((node) => ({ node, name: host.input.seatName(node) })),
    focusedClient() {
        const seat = mainSeat();
        return seat === undefined ? null : (clientsByNode.get(host.input.focus(seat)) ?? null);
    },
    clients: () => [...clientRoot.list] as Client[],
    parentKind(client) {
        const parent = host.xr.parent(stateOf(client).node);
        return parent === undefined ? undefined : host.kind(parent);
    },
    inputContext(seat) {
        const name = host.input.seatName(seat);
        return input.contexts.find((context) => context.name === name) ?? null;
    },
    declared: () => host.xr.declared(),
    now: () => host.input.now(),
    report: (site, error) => {
        reports.push({ site, error });
    },
};

const xrRuntime = new XRRuntime(xrWorld);

class Gnoblin {
    /** The optional extensions: `gnoblin.extensions.xr` is null until the XR program has a session (docs/xr.md). */
    readonly extensions = new Extensions(xrRuntime);
    readonly clients = clientRoot.self as Collection<Client>;
    readonly workspaces = workspaceRoot.self as WorkspaceCollection;
    /** The seats of the compositor as input contexts, with the bindings that run on their chords and drags. */
    readonly input = input;
    /** Policy over the compositor's own consent and passphrase dialogs: listeners that may refuse a request. */
    readonly dialogs = dialogs;

    /**
     * Starts a program. A string is one command line for the host to run; an array is the program and its
     * arguments. The shell's host decides how, and a shell with no spawner rejects the call.
     */
    spawn(command: string | string[]): void {
        const valid =
            typeof command === "string"
                ? command.length > 0
                : Array.isArray(command) && command.length > 0 && command.every((part) => typeof part === "string");
        if (!valid) {
            throw new TypeError("gnoblin.spawn: expects a command string or a non-empty array of strings");
        }
        const message = host.spawn(command);
        if (message != null) {
            throw new GnoblinError("spawn-failed", "gnoblin.spawn", message);
        }
    }

    /** Shows a workspace, given as itself or by id, on the primary display. See `WorkspaceCollection.show`. */
    show(workspace: Workspace | string, options?: ShowOptions): void {
        showWorkspace(workspace, options);
    }

    /** Runs `run` once, `seconds` from now: at the first frame after that time. */
    after(seconds: number, run: () => void): void {
        after(seconds, run);
    }

    /**
     * The hook `defineConfig` calls with the config it was given. The config is laid over the stock one
     * (`mergeConfig`), then the document settings are checked (a refused value throws, which fails the stage and
     * keeps the running generation), the declared workspaces become real `Workspace` objects in
     * `gnoblin.workspaces`, the keys are bound on the first seat, the `on` handlers are attached and `setup` runs.
     * The document settings themselves are committed by the shell once this generation is the running one.
     */
    __register(config: GnoblinConfig): void {
        const merged = mergeConfig(defaultConfig, config);
        const plan = desugarConfig(merged);
        checkDocument(plan.document);
        effectiveConfig = merged;
        for (const planned of plan.workspaces) {
            addWorkspace(new Workspace({ id: planned.id, layout: planned.layout, gaps: planned.gaps }));
        }
        if (plan.decorations) {
            decorate();
        }
        if (plan.menus) {
            startMenus();
        }
        if (plan.alerts) {
            startAlerts();
        }
        bindKeys([...plan.bindings, ...planStockLauncher(merged)]);
        installStock(plan);
        replaceStyle(scopeStyle(clientRoot).style, plan.style);
        attachOn(merged.on ?? {});
        documentPatch = JSON.stringify(plan.document);
        autostartList = JSON.stringify(plan.autostart);
        plan.setup?.(gnoblin as unknown as GnoblinHost);
    }

    /** The config `__register` last merged over the stock one, for the inspector; null before any. */
    __effectiveConfig(): GnoblinConfig | null {
        return effectiveConfig;
    }
}

/** What a key of the config does when it is pressed. A binding action is not a handler: `bindKeys` binds it whole. */
function keyHandler(action: Exclude<ResolvedAction, BindingAction>): () => void {
    switch (action.kind) {
        case "spawn":
            return () => gnoblin.spawn([...action.argv]);
        case "show":
            return () => gnoblin.show(action.workspace, { duration: SLIDE_SECONDS });
        case "show-position":
            return () => {
                const target = workspaceRoot.list[action.position - 1];
                if (target !== undefined) {
                    showWorkspace(target, { duration: SLIDE_SECONDS });
                }
            };
        case "focus-column":
            return () => {
                const shown = activeWorkspace;
                if (shown !== null && shown.layout instanceof StripLayout) {
                    if (action.direction === "left") {
                        shown.layout.focusLeft(shown);
                    } else {
                        shown.layout.focusRight(shown);
                    }
                }
            };
        case "close":
            return () => (input.primary.focusedClient as Client | null)?.close();
        case "call":
            return () => action.handler({ gnoblin: gnoblin as unknown as GnoblinHost });
    }
}

/** Binds the config's keys on the first seat. A tree with no seat has no keyboard to bind, so its keys are left out. */
function bindKeys(keys: readonly PlannedKey[]): void {
    if (keys.length === 0) {
        return;
    }
    let seat;
    try {
        seat = input.primary;
    } catch (error) {
        if (error instanceof GnoblinError && error.code === "no-seat") {
            return;
        }
        throw error;
    }
    for (const { trigger, action } of keys) {
        seat.bind(trigger, action.kind === "binding" ? action.create() : keyHandler(action));
    }
}

export const gnoblin = new Gnoblin();

// What the console shows of the host's own objects. A client is the short block of what a person asks about a window;
// nested inside another value (a collection's member) it is only its id, app and title.
inspect.register(Client, (client) => ({
    name: "Client",
    id: client.id,
    fields: {
        appId: client.appId,
        title: client.title,
        focused: focusedAmong([client]) === client,
        fullscreen: client.fullscreen,
        floating: client.floating,
        x: client.x,
        y: client.y,
        width: client.width,
        height: client.height,
        workspace: client.workspace?.id ?? null,
        ...(client.alive ? {} : { alive: false }),
    },
    brief: { appId: client.appId, title: client.title },
}));
inspect.register(Workspace, (workspace) => ({
    name: "Workspace",
    id: workspace.id,
    fields: {
        name: workspace.name,
        index: workspace.index,
        active: workspace.active,
        layout: workspace.layout.constructor.name,
        clients: workspace.clients.length,
    },
    brief: { name: workspace.name, active: workspace.active },
}));
inspect.register(Collection, (collection) => ({
    name: "Collection",
    fields: {},
    items: [...collection],
}));

for (const node of host.windows()) {
    adopt(node, false);
}

Object.defineProperty(globalThis, "gnoblin", { value: gnoblin, enumerable: false });
// The console's helpers: `inspect.clients()` and the rest are typed at a console prompt without an import. Unlike
// `gnoblin` the name is a convenience and not part of the object model, so a script may replace it.
Object.defineProperty(globalThis, "inspect", { value: inspect, enumerable: false, writable: true, configurable: true });
host.entry = {
    render: (value, maxBytes) => inspect.render(value, { maxBytes }),
    ui: () => {
        runUiFrame();
        try {
            xrRuntime.frame();
        } catch (error) {
            reports.push({ site: "XRRuntime.frame", error });
        }
    },
    plan,
    next,
    pending,
    ready,
    takeErrors,
    takeNotices: () => notices.splice(0),
    document: () => documentPatch,
    autostart: () => autostartList,
    dialogs: (events) => {
        dialogs.deliver(events);
        return queue.length;
    },
    takeVerdicts: () => {
        const taken = verdicts;
        verdicts = [];
        return taken;
    },
    dialogsWanted: () => dialogs.wanted,
    layout,
    finishLoad: ensureDefaultWorkspace,
    loadDefault: () => gnoblin.__register(defaultConfig),
    snapshot: snapshotWorkspaces,
    reconcile: reconcileWorkspaces,
    pointer,
    input: (messages) => {
        input.deliver(messages);
        try {
            stock?.focus.frame();
        } catch (error) {
            reports.push({ site: "FocusPolicy.frame", error });
        }
        return queue.length;
    },
    cancelAll: () => {
        input.cancelAll();
        xrRuntime.release();
        sceneReleaseAll();
        uiReleaseAll();
        return queue.length;
    },
};
