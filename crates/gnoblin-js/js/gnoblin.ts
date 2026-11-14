// The `gnoblin/host` module, re-exported by the user-facing `gnoblin` module. The Rust shell installs
// `globalThis.__gnoblinHost`; this module turns that bridge into stable JS objects and the config API.
//
// The module owns all per-generation object identity and mutable state. The host bridge owns the tree mirror,
// applies writes as one transaction per frame, and reports published values back to this module.
//
// Frame flow: `plan` processes destroyed, created and touched nodes, updating objects and queuing listener calls.
// The shell drains those calls under per-listener budgets, then `layout` computes placements and styles.
// `ready` completes initial adoption; `cancelAll` releases generation-owned resources on retirement.
//
// Sections below follow that flow and the object model:
// host bridge and errors; listeners and shared object state; Client, Workspace, Output and collections;
// workspaces, desks, geometry, layout and styles; snapshots and reconciliation; host events and input;
// stock policy, decorations and services; config registration; `gnoblin` facade and inspector setup.
// Public classes and helpers are exported for the stock `gnoblin` index and user configuration imports.
//
// Reads use the host mirror plus staged writes. Destroyed nodes retain their last values for reads and reject writes.
// Listener errors are collected rather than propagated, so one failing listener does not stop the frame.

import { desugarConfig, defaultConfig, mergeConfig, outputRequest, planStockLauncher } from "gnoblin/config";
import type {
    BindingAction,
    ConfigListeners,
    ConfigPlan,
    GnoblinConfig,
    GnoblinHost,
    OutputConfig,
    PlannedKey,
    ResolvedAction,
} from "gnoblin/config";
import { DialogManager } from "gnoblin/dialogs";
import type { DialogEvent } from "gnoblin/dialogs";
import { AuthenticationManager } from "gnoblin/authentication";
import { IdleMonitor } from "gnoblin/idle";
import type { IdleCommand, IdleReading } from "gnoblin/idle";
import { LaunchFeedback } from "gnoblin/launches";
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
import { attachUi, uiNodeGone, uiReleaseAll } from "gnoblin/ui";
import type { UiBackend } from "gnoblin/ui";
import { attachLock, lockRelease, runLockFrame } from "gnoblin/lock";
import type { LockBackend } from "gnoblin/lock";
import { defaultTitlebarStyle, Decorations } from "gnoblin/titlebar";
import { inspect } from "gnoblin/inspect";
import { WorkspaceSetError, WorkspaceSets } from "gnoblin/sets";
import { restoreSnap, snapClient, snapRecord, snappedZone, unsnapClient } from "gnoblin/snap";
import type { SnapClient, SnapZone } from "gnoblin/snap";
import { windowMenu } from "gnoblin/menus";
import { notRespondingAlerts } from "gnoblin/alerts";
import { barInsets, barsGiveWayTo, barsOnOutputs } from "gnoblin/widgets";
import { windowState } from "gnoblin/window_state";
import type { ClientRequest } from "gnoblin/window_state";
import { EASINGS, Staging, TransitionEngine, curveSamples, setProperties } from "gnoblin/transition";
import type { ClientEasing, Transition, TransitionOptions } from "gnoblin/transition";
import { snapshotStyles } from "gnoblin/style";
import { gestureDirection, onPinch, onSwipe } from "gnoblin/gesture_actions";
import type { GestureActions, GestureTools } from "gnoblin/gesture_actions";
import { repositioning } from "gnoblin/reposition";
import type { RepositionClient } from "gnoblin/reposition";
import {
    ClientStyle,
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
import { HostApi } from "gnoblin/session";
import type { SignalEvent, Version } from "gnoblin/session";
import type { Subscription, Timer } from "gnoblin/operation";
import type { Process, SpawnOptions } from "gnoblin/process";

// ---- Host contract and shared types ----
// Owns the bridge and data shapes shared by the object model and frame loop.

/** The host tree node handle, represented as a JavaScript number by the shell bridge. */

type NodeId = number;
/** A property scalar accepted by the host bridge. */

type Scalar = string | number | boolean;
/** What a property holds: a scalar, the three numbers of a rotation or scale, or a list of names (a claim list). */
type HostValue = Scalar | number[] | string[];

/** A rectangle in the compositor's logical coordinates. */
interface Rect {
    x: number;
    y: number;
    width: number;
    height: number;
}

/** The data-only selection `where` takes: each key is a property, each value what it must equal. */
export type Query = Record<string, Scalar | Scalar[]>;

/** A listener or internal task failure retained until the shell takes frame errors. */

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

/** The callbacks and state access the shell uses to drive one generation. */

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
    /** What the script needs of the compositor beyond the tree: bit 1 is idle state. */
    wants(): number;
    /** The compositor's newest idle reading; returns how many calls wait. */
    idleReading(reading: IdleReading): number;
    /** The idle connection ended: every watch is active again; returns how many calls wait. */
    idleLost(): number;
    /** The idle holds the script asked for or gave back, each once. */
    takeIdleCommands(): IdleCommand[];
    /** The names of the idle holds that are alive, for the shell to give back when the generation ends. */
    idleHolders(): string[];
    /** The compositor refused the idle hold `holder`. */
    idleFailed(holder: string): void;
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
    /** The host API's frame: what the host collected since the last one, as JSON; returns how many calls wait. */
    apiFrame(batch: string): number;
}

/** Native operations and services the Rust shell exposes to this generation. */

interface Host {
    /** Checks a document patch (JSON) against the document in force: the validator's problems, or undefined. */
    config: { check(patch: string, autostart?: string): string | undefined };
    kind(node: NodeId): string | undefined;
    get(node: NodeId, key: string): HostValue | undefined;
    published(node: NodeId, key: string): HostValue | undefined;
    label(node: NodeId): string | undefined;
    set(node: NodeId, key: string, value: HostValue): string | undefined;
    /** What `set` would answer, without queuing the write; `timed` also refuses a property the tree cannot animate. */
    check(node: NodeId, key: string, value: HostValue, timed: boolean): string | undefined;
    /**
     * The same write, carried out over `seconds` along `samples` (progress from 0 to 1), after `delay` seconds when
     * given, and driven by the `tween` node when one is named (0 is none).
     */
    animate(
        node: NodeId,
        key: string,
        value: HostValue,
        seconds: number,
        samples: number[],
        delay?: number,
        tween?: number,
    ): string | undefined;
    /** The `tween` nodes a transition's tracks join (`data/js/gnoblin/transition.ts`). */
    tween: {
        create(): NodeId | string;
        destroy(node: NodeId): string | undefined;
    };
    call(node: NodeId, method: string): string | undefined;
    /** Queues the moves that put a window on top of the others. */
    raise(node: NodeId): string | undefined;
    /** Queues the moves that put a window under the others. */
    lower(node: NodeId): string | undefined;
    windows(): NodeId[];
    screens(): NodeId[];
    /** The screen the main seat's pointer is on, or undefined while the seat names none. */
    pointerScreen(): NodeId | undefined;
    /** The host API (`gnoblin/session`): JSON in, JSON out. */
    api: {
        spawn(argv: string, options: string): string;
        kill(id: number, signal: string): string | undefined;
        begin(verb: string): string;
        status(): string | undefined;
        version(): string;
        signals(): string;
    };
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
    lock: LockBackend;
    /** `Output.apply`: the request text goes to the compositor, whose answer comes back under the token. */
    outputs: {
        apply(node: NodeId, text: string): number | undefined;
        takeAnswers(): { token: number; error: string }[];
    };
    entry?: Entry;
}

// ---- Host bridge staging ----
// Owns the host reference and routes writes through the transition staging layer.

/** The shell-installed bridge; missing host functions make this module unusable. */

const host = (globalThis as { __gnoblinHost?: Host }).__gnoblinHost;
if (!host) {
    throw new Error("the gnoblin module needs the shell host and cannot run without it");
}

// ---- Staged writes ----
//
// `gnoblin.requestTransition` runs its callback with the host's writes staged (`data/js/gnoblin/transition.ts`). The
// host functions are replaced here, once, by the staging layer, so every module that reads or writes a node goes through
// it and none has to know. Outside a callback each call goes straight to the native function.

/** Replaces host writes with the transition staging layer for this generation. */

const staging = new Staging(
    { get: host.get, set: host.set, animate: host.animate, check: host.check },
    (code, target, message) => new GnoblinError(code, target, message),
);
/** Original host operations retained behind staging guards. */

const nativeCall = host.call;
/** Native call function preserved before the transition guard replaces it. */

const nativeRaise = host.raise;
/** Native raise function preserved before the transition guard replaces it. */

const nativeLower = host.lower;
/** Native lower function preserved before the transition guard replaces it. */

const nativeApi = { spawn: host.api.spawn, kill: host.api.kill, begin: host.api.begin };
/** Process operations preserved before the transition guard replaces them. */

/** Names a host node for errors raised when a staged operation is forbidden. */

function nodeName(node: NodeId): string {
    return `${host.kind(node) === "screen" ? "Output" : "Client"}#${host.label(node)}`;
}

host.get = (node, key) => staging.get(node, key) as HostValue | undefined;
host.set = (node, key, value) => staging.set(node, key, value);
host.animate = (node, key, value, seconds, samples) => staging.animate(node, key, value, seconds, samples);
host.call = (node, method) => {
    staging.forbid(`${nodeName(node)}.${method}()`);
    return nativeCall(node, method);
};
host.raise = (node) => {
    staging.forbid(`${nodeName(node)}.raise()`);
    return nativeRaise(node);
};
host.lower = (node) => {
    staging.forbid(`${nodeName(node)}.lower()`);
    return nativeLower(node);
};
host.api.spawn = (argv, options) => {
    staging.forbid("gnoblin.spawn()");
    return nativeApi.spawn(argv, options);
};
host.api.kill = (id, signal) => {
    staging.forbid("Process.kill()");
    return nativeApi.kill(id, signal);
};
host.api.begin = (verb) => {
    staging.forbid(`gnoblin.session.${verb}()`);
    return nativeApi.begin(verb);
};

// ---- Errors and listeners ----
// Owns public errors, listener interfaces, and the queued callback machinery.

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
    /** The compositor changed who draws the window's frame: `client`, `server` or `none` (see `Client.decoration`). */
    onDecorationChanged?(client: Client, decoration: string): void;
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
    /**
     * The client asked for a change of its window (`request.kind`): maximize, fullscreen, minimize, activate, move and
     * so on. Call `request.handle()` to answer it yourself; the first listener to do so wins and the built-in answer
     * (`windowState`) does not run. The listeners of the client itself are asked before those of the collections.
     */
    onRequest?(client: Client, request: ClientRequest<Client>): void;
}

/** Observes membership: an object joining or leaving a collection. */
export class CollectionListener {
    onAdded?(member: Client | Workspace | Output, collection: Collection<Client | Workspace | Output>): void;
    onRemoved?(member: Client | Workspace | Output, collection: Collection<Client | Workspace | Output>): void;
}

/**
 * Observes the outputs: attach it to one output, or to `gnoblin.outputs` for all of them. A listener on the
 * collection hears `onConnected` for an output that is plugged in while the shell runs, not for the ones that were
 * there when it started.
 */
export class OutputListener {
    /** An `apply` asked of this output was answered: `error` is null when it took effect, else why it did not. */
    onApplied?(output: Output, error: string | null): void;
    /** The output appeared: it was plugged in, or made by the compositor. */
    onConnected?(output: Output): void;
    /** The output is gone. Its last values stay readable; `connected` is false. */
    onDisconnected?(output: Output): void;
    /** The output's mode (`WxH@Hz`) changed. */
    onModeChanged?(output: Output, mode: string): void;
    onScaleChanged?(output: Output, scale: number): void;
    /** The output's place or size in the compositor's coordinates changed. */
    onGeometryChanged?(output: Output, geometry: Rect): void;
}

/** A listener source and its attached handlers. */

interface Source {
    listeners: object[];
}

/** A call the module itself queues (an input hook), run beside the listeners' under the same budget. */
interface Task {
    site: string;
    task(): void;
}

/** One deferred listener invocation, including its source and call arguments. */

interface Invocation {
    source: Source;
    listener: object;
    method: string;
    args: unknown[];
}

/** Listener calls and internal tasks waiting for the shell pump. */

const queue: (Invocation | Task)[] = [];
/** Failures collected for the shell to take after processing callbacks. */

let reports: ErrorReport[] = [];

/** Reads the current handler property from a listener. */

function handlerOf(listener: object, method: string): unknown {
    return Reflect.get(listener, method);
}

/** Names a listener constructor for error reports, falling back to `listener`. */

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

/** Queues each listener currently implementing `method` on a source. */

function queueFor(source: Source, method: string, args: unknown[]): void {
    for (const listener of source.listeners) {
        if (typeof handlerOf(listener, method) === "function") {
            queue.push({ source, listener, method, args });
        }
    }
}

/** Checks listener input before it is attached. Throws for non-object values. */

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

/** The host properties frozen when a client leaves, so the object keeps its final readable state. */

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
    "parent_window",
    "from_launcher",
    "decoration",
    "decoration_wanted",
];

/** What listeners were last told about a client, to raise an event only for a real change. */
interface Seen {
    title: string;
    appId: string;
    decoration: string;
    focused: boolean;
    x: number;
    y: number;
    width: number;
    height: number;
    /** The node of the window this one is a dialog of, 0 for none. */
    parent: number;
}

/** State shared by native-backed object instances: identity, liveness, events, and last published values. */

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

/** Per-object state store; a WeakMap preserves object lifetime without retaining abandoned wrappers. */

const states = new WeakMap<object, NodeState>();

/** Returns native state for a managed object. Throws when the value is not a shell-created object. */

function stateOf(object: object): NodeState {
    const state = states.get(object);
    if (!state) {
        throw new TypeError("not an object the compositor made");
    }
    return state;
}

/** Reads a scalar property from the frozen snapshot or the current host mirror. */

function read<T extends Scalar>(object: object, key: string): T {
    const state = stateOf(object);
    return (state.frozen ? state.frozen[key] : host.get(state.node, key)) as T;
}

/** Freezes the requested properties and marks an object dead before removal events are delivered. */

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
        parent: host.published(node, "parent_window") as number,
        decoration: host.published(node, "decoration") as string,
    };
}

/** The queued write of one number property, or the error that names the member. */
/** Writes a finite number property. Throws for dead clients, invalid values, or host refusal. */

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
/** Writes a boolean property. Throws for dead clients, invalid values, or host refusal. */

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

/** A three-number vector used for rotation and scale. */

type Vector3 = readonly [number, number, number];

/** The queued write of a property of three numbers (`rotation`, `scale`), or the error that names the member. */
/** Writes a three-number vector property. Throws for dead clients, invalid values, or host refusal. */

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

export type { ClientEasing } from "gnoblin/transition";

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

/** Duration and easing options for a client animation; duration is measured in seconds. */

export interface AnimateOptions {
    /** Seconds. Zero is an instant write. */
    duration: number;
    /** Default `"out"`, as the retired shell's. */
    easing?: ClientEasing;
}

/** Calls a native client method. Throws for a dead client or host refusal. */

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

/** The stable wrapper for each live host window node. */

const clientsByNode = new Map<NodeId, Client>();
/** The workspace each client is on. A workspace is the script's, so the compositor does not know. */
const membership = new WeakMap<Client, Workspace>();
/** The clients that were there when the shell booted: a layout does not place them again. */
const preexisting = new WeakSet<Client>();
/** The clients a layout was offered as arrivals: a dialog never is, so a layout is never told one left. */
const offered = new WeakSet<Client>();
/** The dialogs already centred in their parent. */
const centred = new WeakSet<Client>();

// ---- Client object ----
// Owns the public window API; membership and host-backed values are shared with frame processing.

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
     * Who draws the window's frame, as the compositor negotiated it: `server` when the shell's titlebar does (the client
     * asked for server-side and this shell declared decorations, or the window is an X11 one), `client` when the client
     * draws its own title bar (a GTK, Qt or foot window), `none` for a window nobody frames. The compositor writes it.
     * `frameInset` is reserved only for a `server` window.
     */
    get decoration(): string {
        return read<string>(this, "decoration");
    }

    /** What the client asked of `xdg-decoration`: `client`, `server`, or `unset` (it asked nothing, or is an X11 window). */
    get decorationWanted(): string {
        return read<string>(this, "decoration_wanted");
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

    /**
     * The window this one is a dialog of (`xdg_toplevel.set_parent`), or null. A dialog is centred in its parent when
     * it opens, moves with it, stays on its workspace and output, and is left out of the layout's tiling.
     */
    get parentWindow(): Client | null {
        const node = read<number>(this, "parent_window");
        return node ? (clientsByNode.get(node) ?? null) : null;
    }

    /**
     * Whether the window opened on an activation token the session's launcher minted, so `focus.newWindows` treats it as
     * launched. The compositor sets it when the window redeems such a token; a program started with `gnoblin.spawn` is
     * not marked, because the shell's spawner mints no token.
     */
    get fromLauncher(): boolean {
        return read<boolean>(this, "from_launcher");
    }

    /** The output that shows the client's workspace, or null while no output shows it (or the output is gone). */
    get output(): Output | null {
        const desk = deskOfClient(this);
        return desk?.parked === false ? desk.output : null;
    }

    /**
     * Moves the window to `workspace` and fits it to that workspace's work area: a maximised window fills it, a snapped
     * one re-snaps, a floating one is shrunk to fit and moved inside it. A window that leaves the shown workspace hands
     * its focus to a window that stays. Its dialogs go with it. Refused (false, and nothing moves) for a fullscreen
     * window, a dialog (it moves only with its parent), a dead window, a workspace that is not in
     * `gnoblin.workspaces`, or the workspace it is on already.
     */
    moveToWorkspace(workspace: Workspace): boolean {
        return moveClientTo(this, workspace);
    }

    /** `moveToWorkspace` to the workspace `output` shows. False when the output is gone or shows none. */
    moveToOutput(output: Output): boolean {
        if (!(output instanceof Output)) {
            throw new TypeError(`${this}.moveToOutput: needs an Output`);
        }
        const shown = output.alive ? output.workspace : null;
        return shown !== null && moveClientTo(this, shown);
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
     * Starts moving the window with the arrow keys, 16 pixels a press, inside the work area: Return keeps the place,
     * Escape puts the window back. Refused (false) for a window that is fullscreen, maximised, tiled or not showing; a
     * snapped window is unsnapped first. See `gnoblin/reposition`.
     */
    moveWithKeys(): boolean {
        return repositioning.begin(this as unknown as RepositionClient, "move");
    }

    /** Starts resizing the window with the arrow keys: Right and Down grow it, Left and Up shrink it. */
    resizeWithKeys(): boolean {
        return repositioning.begin(this as unknown as RepositionClient, "resize");
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
        const samples = curveSamples(curve);
        // Reduced motion lands the value at once, as the compositor's own safety net would.
        const seconds = motion(duration);
        for (const [key, value] of entries) {
            const sent = Array.isArray(value) ? [value[0], value[1], value[2]] : (value as number);
            const code =
                seconds === 0 ? host.set(state.node, key, sent) : host.animate(state.node, key, sent, seconds, samples);
            if (code != null) {
                refuse(code, target, "client");
            }
        }
    }

    /**
     * Assigns `props` as one `requestTransition`: the same as `gnoblin.requestTransition(() => { client.opacity = ... })`
     * for this client. Without `options` the change is atomic and immediate. `options.duration` is in milliseconds.
     */
    set(props: Record<string, unknown>, options?: TransitionOptions): Transition {
        return setProperties(transitions, this, `${this}`, props, options);
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
        // A fullscreen sibling keeps the top: the the retired shell `Window:raise` put each back over the window it raised.
        if (!this.fullscreen) {
            const own = membership.get(this) ?? null;
            for (const node of host.windows()) {
                const other = clientsByNode.get(node);
                if (
                    other !== undefined &&
                    other !== this &&
                    (membership.get(other) ?? null) === own &&
                    other.fullscreen
                ) {
                    host.raise(node);
                }
            }
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

// ---- Workspace and output objects ----
// Owns script-created workspaces and compositor-created output wrappers.

/** Layout and membership state held privately for each script workspace. */

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
    /** The fullscreen and maximised clients the layout last computed for: a change is a reason to compute again. */
    coverSeen: string;
}

/** Per-workspace state; a WeakMap avoids retaining removed workspaces. */

const wsStates = new WeakMap<object, WorkspaceState>();

/** Returns workspace state. Throws when the value is not a Workspace. */

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
                swapped: (a, b) => swapClients(this, a as Client, b as Client),
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
            coverSeen: "",
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

    /** Whether an output shows this workspace. With several outputs, several workspaces are active. */
    get active(): boolean {
        return isShown(this);
    }

    /** The output that shows this workspace, or null while none does. */
    get output(): Output | null {
        const desk = deskShowing(this);
        return desk !== null && !desk.parked ? desk.output : null;
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

    get clients(): MutableCollection<Client> {
        return wsState(this).clients;
    }

    /**
     * Assigns `props` (`layout`, `gaps`, `name`) as one `requestTransition`: the windows tween from where they are
     * drawn to where the new layout puts them. Without `options` the change is atomic and immediate.
     */
    set(props: Record<string, unknown>, options?: TransitionOptions): Transition {
        return setProperties(transitions, this, `${this}`, props, options);
    }

    /** Shows this workspace on the output under the pointer (the primary output when no pointer says). */
    show(options?: ShowOptions): void {
        showWorkspace(this, options);
    }

    /**
     * Moves the keyboard focus `step` windows along the workspace's windows, in the order they are listed (the retired shell's
     * `Workspace:walk`): `1` to the next, `-1` to the previous. Nothing moves when no window of the workspace has the focus,
     * and a step past either end leaves the focus where it is.
     */
    walk(step: number): void {
        if (typeof step !== "number" || !Number.isInteger(step)) {
            throw new TypeError(`${this}.walk: step must be a whole number, got ${String(step)}`);
        }
        const list = wsState(this).clientsState.list as Client[];
        const at = list.findIndex((client) => hasFocus(client));
        const next = at < 0 ? undefined : list[at + step];
        if (next !== undefined && next.alive) {
            next.focus();
        }
    }

    toString(): string {
        return `Workspace#${wsState(this).id}`;
    }
}

// ---- Output properties and collections ----
// Owns output snapshots and live object collections, including query views and listeners.

/** The native output properties retained after an output disconnects. */

const OUTPUT_KEYS = [
    "name",
    "connector",
    "identity",
    "mirror_of",
    "make",
    "model",
    "x",
    "y",
    "width",
    "height",
    "output_scale",
    "transform",
    "mode",
    "primary",
    "enabled",
];

/** What listeners were last told about an output, to raise an event only for a real change. */
interface OutputSeen {
    /** Whether the output has no place of its own: it mirrors another or is off. It has no desk then. */
    detached: boolean;
    mode: string;
    scale: number;
    x: number;
    y: number;
    width: number;
    height: number;
}

/** Per-output event baseline, retained while the wrapper exists. */

const outputSeen = new WeakMap<object, OutputSeen>();

/** Reads the compositor-published output properties used for connection and change events. */

function publishedOutput(node: NodeId): OutputSeen {
    return {
        detached: host.published(node, "mirror_of") !== "" || host.published(node, "enabled") === false,
        mode: host.published(node, "mode") as string,
        scale: host.published(node, "output_scale") as number,
        x: host.published(node, "x") as number,
        y: host.published(node, "y") as number,
        width: host.published(node, "width") as number,
        height: host.published(node, "height") as number,
    };
}

/**
 * A connected output, the compositor's `screen` node: a physical or virtual display with its place in the layout. It
 * shows one workspace at a time (`workspace`, `show`), and the work area the layouts tile in is its rectangle less the
 * bars on it. It is made and destroyed by the compositor as displays come and go; a destroyed one keeps its last values
 * and reads `connected === false`.
 */
export class Output {
    constructor(token: symbol, node: NodeId) {
        if (token !== CREATE) {
            throw new TypeError("an Output is made by the compositor, not by a script");
        }
        states.set(this, {
            node,
            alive: true,
            frozen: null,
            listeners: [],
            readied: false,
            seen: null,
            floating: false,
        });
        outputSeen.set(this, publishedOutput(node));
    }

    get id(): string {
        return host.label(stateOf(this).node) ?? "?";
    }

    get alive(): boolean {
        return stateOf(this).alive;
    }

    /** Whether the display is still connected: false once the compositor destroyed the output. */
    get connected(): boolean {
        return stateOf(this).alive;
    }

    /** The name the compositor gives the output, which a workspace set's entry and `gnoblin.outputs.where` use. */
    get name(): string {
        return read<string>(this, "name");
    }

    get make(): string {
        return read<string>(this, "make");
    }

    get model(): string {
        return read<string>(this, "model");
    }

    /** The connector the display is plugged into (`DP-1`): what `defineConfig({ outputs })` and `mirrorOf` name. */
    get connector(): string {
        return read<string>(this, "connector");
    }

    /** The monitor's EDID identity (`MAN:product:serial`), empty for a display that reports none. */
    get identity(): string {
        return read<string>(this, "identity");
    }

    /** The output this one shows an exact copy of, or null when it shows its own picture. */
    get mirrorOf(): Output | null {
        const source = read<string>(this, "mirror_of");
        return source === "" ? null : (liveOutputs().find((one) => one.connector === source) ?? null);
    }

    /** The mode being driven, `WxH@Hz`. */
    get mode(): string {
        return read<string>(this, "mode");
    }

    /** The factor the output composes at: 2 is a HiDPI panel. */
    get scale(): number {
        return read<number>(this, "output_scale");
    }

    get transform(): string {
        return read<string>(this, "transform");
    }

    /** Whether this is the primary output, the one the stock shell's bars and `gnoblin.workspaces.area` are on. */
    get primary(): boolean {
        return read<boolean>(this, "primary");
    }

    get enabled(): boolean {
        return read<boolean>(this, "enabled");
    }

    get x(): number {
        return read<number>(this, "x");
    }

    get y(): number {
        return read<number>(this, "y");
    }

    get width(): number {
        return read<number>(this, "width");
    }

    get height(): number {
        return read<number>(this, "height");
    }

    get position(): { x: number; y: number } {
        return { x: this.x, y: this.y };
    }

    /**
     * Configures the display through the compositor's transactional output engine and keeps the result as this
     * monitor's profile, so the same monitor on the same port gets it back after an unplug. A key left out keeps its
     * value. The whole arrangement is checked before hardware is touched; if the backend then refuses a mode, the
     * earlier changes are put back and the last good arrangement stays. A bad value throws here, naming the key.
     * The compositor's answer arrives a frame later on the returned `OutputApplication` and `onApplied`.
     */
    apply(options: OutputConfig & { mirrorOf?: Output | string | null }): OutputApplication {
        return applyOutput(this, options, `${this}.apply`);
    }

    /** Shows an exact copy of `source` (scaled to fit, letterboxed) on this display, or stops mirroring with null. */
    mirror(source: Output | null): OutputApplication {
        return applyOutput(this, { mirrorOf: source }, `${this}.mirror`);
    }

    /** The whole output in the compositor's coordinates. */
    get bounds(): Rect {
        return { x: this.x, y: this.y, width: this.width, height: this.height };
    }

    /** What a layout tiles in on this output: the output less the bars on it. Null for a gone output or one with no size. */
    get area(): Rect | null {
        return stateOf(this).alive ? areaOf(this) : null;
    }

    /** The workspace the output shows, or null when it shows none or is gone. */
    get workspace(): Workspace | null {
        const desk = deskByOutput.get(this);
        return desk !== undefined && stateOf(this).alive ? desk.shown : null;
    }

    /**
     * Shows `workspace` (itself or its id) on this output. A workspace another output shows moves here and that output
     * takes the workspace this one showed, so no workspace is on two outputs. `options.duration` slides the windows.
     */
    show(workspace: Workspace | string, options: ShowOptions = {}): void {
        showWorkspace(workspace, { ...options, output: this });
    }

    addListener(listener: OutputListener): void {
        assertListener(listener, `${this}.addListener`);
        const { listeners } = stateOf(this);
        if (!listeners.includes(listener)) {
            listeners.push(listener);
        }
    }

    removeListener(listener: OutputListener): void {
        const { listeners } = stateOf(this);
        const at = listeners.indexOf(listener);
        if (at >= 0) {
            listeners.splice(at, 1);
        }
    }

    toString(): string {
        return `Output#${host.label(stateOf(this).node)}`;
    }
}

/** Any object kind supported by a host collection. */

type Member = Client | Workspace | Output;

/** Operations that mutate a script-owned collection. */

interface Mutation {
    add(member: Member): void;
    /** False when the collection refuses to let the member go; anything else is taken as done. */
    remove(member: Member): boolean | void;
    clear(): void;
    /** Told after two members traded places in the order. */
    swapped?(a: Member, b: Member): void;
}

/** Private view state for root collections and their filtered selections. */

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

/** Collection backing state; the weak keys follow the lifetime of collection objects. */

const collections = new WeakMap<object, CollectionState>();
/** The views with listeners: the only ones whose membership the plan keeps up to date. */
const tracked: CollectionState[] = [];

/** Returns the private state of a collection. Throws for non-collection values. */

function collectionState(collection: object): CollectionState {
    const state = collections.get(collection);
    if (!state) {
        throw new TypeError("not a gnoblin collection");
    }
    return state;
}

/** Fields supported by `where` for each kind of collection. */

const QUERY_KEYS: Record<string, readonly string[]> = {
    Client: ["id", "appId", "title", "focused", "fullscreen", "x", "y", "width", "height"],
    Workspace: ["id", "name", "index", "active"],
    Output: ["id", "name", "make", "model", "mode", "scale", "primary", "enabled", "x", "y", "width", "height"],
};

/** Tests a member against the data-only property query. */

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

/** Returns the current members in collection order. */

function itemsOf(state: CollectionState): Member[] {
    if (state.parent === null) {
        return state.list.slice();
    }
    const query = state.query as Query;
    return itemsOf(state.parent).filter((member) => matches(query, member));
}

/** Finds the root collection that owns a selection's members. */

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

    /**
     * Removes a member, given as itself or by id. A member that is not in the collection is ignored. True when a member was
     * removed; false when there was none to remove or the collection refused (`gnoblin.workspaces` keeps its last one).
     */
    remove(member: T | string): boolean {
        const target = typeof member === "string" ? this.get(member) : member;
        if (target === undefined || !collectionState(this).list.includes(target)) {
            return false;
        }
        return this.mutation("remove").remove(target) !== false;
    }

    clear(): void {
        this.mutation("clear").clear();
    }

    /**
     * Trades the places of two members in the collection's order. A workspace's `clients` is the order its layout
     * places by, so two clients of a tiled workspace trade tiles at the next frame. Both must be members.
     */
    swap(a: T, b: T): void {
        const mutation = this.mutation("swap");
        const list = collectionState(this).list;
        const first = list.indexOf(a);
        const second = list.indexOf(b);
        if (first < 0 || second < 0) {
            throw new GnoblinError("not-a-member", "Collection.swap", "both members must be in the collection");
        }
        if (first === second) {
            return;
        }
        list[first] = b;
        list[second] = a;
        mutation.swapped?.(a, b);
    }
}

/** The workspaces of the script, which also say which of them each output shows. */
class WorkspaceCollection extends MutableCollection<Workspace> {
    /** Named groups of workspaces to show together, each entry on an output. */
    readonly sets = new WorkspaceSets({
        declared: () => new Set((workspaceRoot.list as Workspace[]).map((workspace) => workspace.id)),
        outputs: () => ({
            names: deskOutputs().map((output) => output.name),
            primary: primaryOutput()?.name ?? "",
        }),
        show: (id, output) =>
            showWorkspace(id, {
                duration: SLIDE_SECONDS,
                ...(output === undefined ? {} : { output: deskOutputs().find((one) => one.name === output) }),
            }),
        report: (message) => {
            reports.push({ site: "gnoblin.workspaces.sets", error: new Error(message) });
        },
    });

    /** The area windows are laid out in on the primary output; null while the tree has none. */
    get area(): Rect | null {
        return screenArea();
    }

    /**
     * Makes the set `name` of `defineConfig({ workspaceSets })` (or of `sets.define`) live: each entry shows its
     * workspace on its output, and one whose output is not connected shows on the primary output, with a report.
     * Activating the live set again changes nothing. Refused (false, with a report) for an unknown set or a workspace
     * that has since gone, as the the retired shell `activate_set` did; `sets.activate` throws instead.
     */
    activateSet(name: string): boolean {
        try {
            this.sets.activate(name);
            return true;
        } catch (error) {
            if (!(error instanceof WorkspaceSetError)) {
                throw error;
            }
            reports.push({
                site: "gnoblin.workspaces.activateSet",
                error: new Error(`workspace set refused: ${error.message}`),
            });
            return false;
        }
    }

    /**
     * Shows a workspace, given as itself or by id, on `options.output`, or on the output under the pointer (the primary
     * output when no pointer says). With `options.duration` the windows slide from one workspace to the other; without
     * it the switch is at once.
     */
    show(workspace: Workspace | string, options?: ShowOptions): void {
        showWorkspace(workspace, options);
    }
}

/** Creates a root collection state for objects of `kind`. */

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

// ---- Root collections ----
// Owns the compositor's live objects and the script's workspace list.

/** All native clients in compositor order. */

const clientRoot = collectionState(new Collection<Client>(CREATE, rootState("Client")));
/** Connected outputs in compositor creation order. */

const outputRoot = collectionState(new Collection<Output>(CREATE, rootState("Output")));
/** Script workspaces and their named sets; membership is script-owned. */

const workspaceRoot = collectionState(
    new WorkspaceCollection(CREATE, {
        ...rootState("Workspace"),
        mutation: {
            add: (workspace) => addWorkspace(workspace as Workspace),
            // The last workspace stays: a shell with nowhere to put a window is unusable (Before: `remove_workspace` refuses).
            remove: (workspace) => {
                if (workspaceRoot.list.length < 2) {
                    return false;
                }
                removeWorkspace(workspace as Workspace);
                return true;
            },
            clear: () => {
                for (const workspace of [...workspaceRoot.list]) {
                    removeWorkspace(workspace as Workspace);
                }
            },
        },
    }),
);

// ---- Collection events and workspace membership ----
// Owns collection notifications, client transfers, and the output-to-workspace desk model.

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

/** Queues a member event for its own listeners and every collection that currently holds it. */

function emitMember(client: Client | Output, held: CollectionState[], method: string, args: unknown[]): void {
    queueFor(stateOf(client), method, args);
    for (const state of held) {
        queueFor(state, method, args);
    }
}

/** Returns private workspace state for a client, or null when it has no workspace. */

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

/**
 * What one output shows: the workspace on it, and what becomes of its windows when the output is unplugged (the the retired shell
 * shell's `Desk`). A desk outlives its output only while no other output is left: it waits `parked`, hidden, for the
 * next output, whose desk it becomes. A tree with no output at all has one desk with none that is not parked, so a
 * script still has a workspace on show. Workspaces are the script's, not an output's: one workspace is on one desk.
 */

interface Desk {
    output: Output | null;
    shown: Workspace | null;
    parked: boolean;
}

/** All live desks, with reverse lookup by output for active workspaces. */

const desks: Desk[] = [];
/** Reverse index from an output to the desk it displays. */

const deskByOutput = new WeakMap<Output, Desk>();
/** Maps compositor screen-node handles to their stable output wrappers. */

const outputsByNode = new Map<NodeId, Output>();

/** The outputs that are connected, in the order the compositor made them. */
function liveOutputs(): Output[] {
    return outputRoot.list as Output[];
}

/** The outputs that have a desk: every connected output but a mirror and one that is off. */
function deskOutputs(): Output[] {
    return liveOutputs().filter((output) => deskByOutput.has(output));
}

/** The output flagged primary, or the first one; null while the tree has none. */
function primaryOutput(): Output | null {
    const list = deskOutputs();
    return list.find((output) => output.primary) ?? list[0] ?? null;
}

/** The desk of the primary output; with no output, the desk a tree without one keeps (or a parked one). */
function primaryDesk(): Desk | null {
    const output = primaryOutput();
    return output === null ? (desks.find((desk) => desk.output === null) ?? null) : (deskByOutput.get(output) ?? null);
}

/** The desk a new window, a key and a plain `show` act on: the one under the pointer, else the primary one. */
function currentDesk(): Desk | null {
    const node = host.pointerScreen();
    const under = node === undefined ? undefined : outputsByNode.get(node);
    return (under === undefined ? undefined : deskByOutput.get(under)) ?? primaryDesk();
}

/** Finds the desk that currently shows a workspace, if any. */

function deskShowing(workspace: Workspace): Desk | null {
    return desks.find((desk) => desk.shown === workspace) ?? null;
}

/** Whether an output (or the tree's one output-less desk) shows `workspace`. A parked desk shows nothing. */
function isShown(workspace: Workspace): boolean {
    const desk = deskShowing(workspace);
    return desk !== null && !desk.parked;
}

/** The desk the workspace `client` is on is shown on, or null. */
function deskOfClient(client: Client): Desk | null {
    const workspace = membership.get(client);
    return workspace === undefined ? null : deskShowing(workspace);
}

/** A desk for a tree that has no output, so the first workspace is on show whatever the compositor has. */
function ensureDesk(): void {
    if (desks.length === 0) {
        desks.push({ output: null, shown: null, parked: false });
    }
}

/** Gives each desk that shows nothing the first workspace no desk shows. */
function fillDesks(): void {
    for (const desk of desks) {
        if (desk.shown !== null) {
            continue;
        }
        const free = (workspaceRoot.list as Workspace[]).find((workspace) => deskShowing(workspace) === null);
        if (free !== undefined) {
            setShown(desk, free);
        }
    }
}
/**
 * False until the first frame: the entry module is still adding workspaces, and the windows that were
 * there at boot join one only when it is done, or a reload could not put them back where they were.
 */
let opened = false;

/** Runs a host callback while collecting its error under `site` instead of aborting the frame. */

function guard(site: string, body: () => void): void {
    try {
        body();
    } catch (error) {
        reports.push({ site, error });
    }
}

/** A workspace's clients traded places: its layout computes again, and one with a record of its own trades them there. */
function swapClients(workspace: Workspace, a: Client, b: Client): void {
    const state = wsState(workspace);
    state.dirty = true;
    guard(`${workspace}.layout.onClientsSwapped`, () => state.layout.onClientsSwapped?.(a, b));
}

/** Puts `client` in `workspace`, which it is not in. */
function link(workspace: Workspace, client: Client, arrived: boolean): void {
    const state = wsState(workspace);
    membership.set(client, workspace);
    state.clientsState.list.push(client);
    state.dirty = true;
    if (arrived) {
        state.arrived.push(client);
        offered.add(client);
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
    } else if (offered.has(client) || preexisting.has(client)) {
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
    // A dialog stays with its parent: it goes wherever the parent goes.
    for (const dialog of dialogsOf(client)) {
        moveClient(dialog, target, false);
    }
}

/** The live clients that are dialogs of `parent`. */
function dialogsOf(parent: Client): Client[] {
    return (clientRoot.list as Client[]).filter((client) => client.alive && client.parentWindow === parent);
}

/**
 * Puts a dialog where its parent is, and centres it in the parent the first time. Returns whether it did: a dialog
 * whose parent is on no workspace (or is gone) is placed as any other window.
 */
function placeDialog(client: Client): boolean {
    const parent = client.parentWindow;
    const workspace = parent === null ? undefined : membership.get(parent);
    if (parent === null || workspace === undefined) {
        return false;
    }
    if (membership.get(client) !== workspace) {
        moveClient(client, workspace, false);
    }
    if (!centred.has(client)) {
        centred.add(client);
        client.x = parent.x + Math.floor((parent.width - client.width) / 2);
        client.y = parent.y + Math.floor((parent.height - client.height) / 2);
    }
    return true;
}

/** A rectangle that moved to another output: shrunk to the area when larger, then moved inside it. */
function fitInto(rect: Rect, area: Rect): Rect {
    const width = Math.min(rect.width, area.width);
    const height = Math.min(rect.height, area.height);
    return {
        x: Math.min(Math.max(rect.x, area.x), area.x + area.width - width),
        y: Math.min(Math.max(rect.y, area.y), area.y + area.height - height),
        width,
        height,
    };
}

/**
 * The bar a window shows above its content, whether or not fullscreen hides it at the moment. `frameInset` is 0 for a
 * fullscreen window, but the room a window keeps for its bar is not: leaving fullscreen brings the bar back.
 */
function barHeight(client: Client): number {
    return decorations?.titlebarOf(client)?.style.height ?? 0;
}

/**
 * The content rectangle `rect` as a frame inside `area`: the content with `bar` on top, shrunk to fit and moved the least
 * distance that puts all of it inside. A bar is never left above the area's top edge.
 */
function fitContent(rect: Rect, bar: number, area: Rect): Rect {
    const frame = fitInto({ x: rect.x, y: rect.y - bar, width: rect.width, height: rect.height + bar }, area);
    return { x: frame.x, y: frame.y + bar, width: frame.width, height: Math.max(0, frame.height - bar) };
}

/** `area` less the room a bar takes along its top: where the content of a window that fills a frame in `area` goes. */
function belowBar(client: Client, area: Rect): Rect {
    const bar = barHeight(client);
    return { x: area.x, y: area.y + bar, width: area.width, height: Math.max(1, area.height - bar) };
}

/** The part of a client the rectangle it floats at is kept in. */
interface Floater {
    floatingBounds?: Rect | null;
}

/**
 * Fits a window to the work area it is now in: a maximised window fills it, a snapped one tiles to its zone again, a
 * floating one shrinks to fit and moves inside it (the frame, bar included, stays inside), and the rectangle it floats
 * at follows. A fullscreen window covers its output.
 */
function refitClient(client: Client, area: Rect): void {
    if (client.fullscreen) {
        const output = outputRectOf(client);
        if (output !== null) {
            client.bounds = output;
        }
        return;
    }
    const zone = snappedZone(client);
    const bar = barHeight(client);
    if (client.maximised) {
        client.bounds = framed(client, area);
    } else if (zone !== null) {
        snapClient(client as unknown as SnapClient, zone, { edgeTiling: true, area: belowBar(client, area) });
    } else {
        client.bounds = fitContent(client.bounds, bar, area);
    }
    const floater = client as unknown as Floater;
    if (floater.floatingBounds) {
        floater.floatingBounds = fitContent(floater.floatingBounds, bar, area);
    }
}

/** What `barHeight` was when a window was last fitted, so the next pass knows a bar came or went. */
const fittedBar = new WeakMap<Client, number>();

/**
 * A titlebar came or went (the compositor changed who draws the window's frame, or a reload toggled decorations): the
 * window's content does not move by itself, so a bar that mounts above a window placed at the top of the area would be
 * drawn off the screen. The rule is that the FRAME keeps its place: the content moves down by the bar that came, up by the
 * bar that went, and the result is clamped so the frame stays inside the work area. A maximised window fills the area
 * again, a snapped one tiles to its zone again, and a window a tiling layout places is placed again by the layout (the
 * decoration change marks its workspace dirty). The rectangles a window returns to (leaving maximised, unsnapping, the
 * one it floats at) move the same way, so they do not bring the old offset back. A fullscreen window is left alone and
 * fitted when it leaves: the rectangle it returns to is the one the bar was fitted to before.
 */
function followBar(client: Client, area: Rect): void {
    if (client.fullscreen) {
        return;
    }
    const bar = barHeight(client);
    const was = fittedBar.get(client);
    fittedBar.set(client, bar);
    if (was === undefined || was === bar) {
        return;
    }
    const delta = bar - was;
    const moved = (rect: Rect): Rect => fitContent({ ...rect, y: rect.y + delta }, bar, area);
    const kept = windowState.kept(client).floating;
    if (kept !== null) {
        windowState.keep(client, { floating: moved(kept) });
    }
    const record = snapRecord(client);
    if (record !== null) {
        restoreSnap(client, { zone: record.zone, floating: moved(record.floating) });
    }
    const floater = client as unknown as Floater;
    if (floater.floatingBounds) {
        floater.floatingBounds = moved(floater.floatingBounds);
    }
    const zone = snappedZone(client);
    if (client.maximised) {
        client.bounds = framed(client, area);
    } else if (zone !== null) {
        snapClient(client as unknown as SnapClient, zone, { edgeTiling: true, area: belowBar(client, area) });
    } else if (!client.tiled || stateOf(client).floating) {
        client.bounds = moved(client.bounds);
    }
}

/** Hands the keyboard to the first window still showing on `workspace`, or to nobody. A tree with no seat has none. */
function passFocusFrom(workspace: Workspace | null, leaving: Client): void {
    if (host.input.seats().length === 0) {
        return;
    }
    const next =
        workspace === null
            ? undefined
            : (wsState(workspace).clientsState.list as Client[]).find(
                  (one) => one !== leaving && one.alive && !one.minimised,
              );
    if (next === undefined) {
        input.primary.clearFocus();
    } else {
        next.focus();
    }
}

/** `client.moveToWorkspace(workspace)`: the retired shell's `Window:move_to`. */
function moveClientTo(client: Client, workspace: Workspace): boolean {
    if (!(workspace instanceof Workspace)) {
        throw new TypeError(`${client}.moveToWorkspace: needs a Workspace`);
    }
    const from = membership.get(client) ?? null;
    if (
        !client.alive ||
        !wsState(workspace).attached ||
        client.fullscreen ||
        client.parentWindow !== null ||
        from === workspace
    ) {
        return false;
    }
    const wasFocused = hasFocus(client);
    moveClient(client, workspace, true);
    // The window left the screen, so the focus passes to a window that stays.
    if (wasFocused && !isShown(workspace)) {
        passFocusFrom(from, client);
    }
    const area = workAreaOfWorkspace(workspace);
    if (area !== null) {
        refitClient(client, area);
    }
    return true;
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

/** Removes every client from a workspace without closing the windows. */

function clearClients(workspace: Workspace): void {
    for (const client of [...wsState(workspace).clientsState.list] as Client[]) {
        detachClient(workspace, client);
    }
}

/** Makes `next` the workspace `desk` shows. */
function setShown(desk: Desk, next: Workspace | null): void {
    const before = desk.shown;
    if (before === next) {
        return;
    }
    desk.shown = next;
    if (next) {
        wsState(next).dirty = true;
    }
    for (const changed of [before, next]) {
        if (changed && wsState(changed).attached) {
            recompute(workspaceRoot, changed);
        }
    }
}

/** Seconds a workspace slide takes, as the retired shell's `Desk:show`. */
export const SLIDE_SECONDS = 0.25;

/** How `show` moves to a workspace. */
export interface ShowOptions {
    /** Seconds the slide between the two workspaces takes. Zero or absent switches at once. */
    duration?: number;
    /** How much of the slide a touchpad gesture has made already, 0 to 1: the slide carries on from there. */
    progress?: number;
    /** The output to show it on. Left out, the one under the pointer, else the primary one. */
    output?: Output;
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

/** Slides waiting to start after the next successful layout pass. */

const slides: Slide[] = [];
/** The workspaces that are not shown but keep their windows drawn until a slide away from them ends. */
const holding = new Map<Workspace, number>();

/**
 * The duration the shell gives an animation: `seconds`, or 0 while the seat asks for reduced motion
 * (`appearance.reduced-motion`, the seat's `reduced_motion`). A 0 makes a slide, a fade or `Client.animate` an instant write.
 * The compositor lands a timed write at once under reduced motion too, so a shell that forgets this still obeys.
 */
function motion(seconds: number): number {
    const seat = mainSeat();
    return seat !== undefined && seatHost.settings(seat)?.reducedMotion === true ? 0 : seconds;
}

/** `gnoblin.workspaces.show(workspace, options)`. */
function showWorkspace(workspace: Workspace | string, options: ShowOptions = {}): void {
    const target = typeof workspace === "string" ? workspaceRoot.list.find((w) => w.id === workspace) : workspace;
    if (target === undefined) {
        throw new GnoblinError("unknown-workspace", "gnoblin.workspaces.show", `there is no workspace "${workspace}"`);
    }
    if (!wsState(target).attached) {
        throw new GnoblinError("detached", `${target}.show`, "add the workspace to gnoblin.workspaces first");
    }
    const asked = options.duration ?? 0;
    const progress = options.progress ?? 0;
    if (typeof asked !== "number" || !Number.isFinite(asked) || asked < 0) {
        throw new TypeError(`${target}.show: duration must be a finite number of seconds of at least 0`);
    }
    if (typeof progress !== "number" || !(progress >= 0 && progress <= 1)) {
        throw new TypeError(`${target}.show: progress must be a number from 0 to 1`);
    }
    if (options.output !== undefined && !(options.output instanceof Output)) {
        throw new TypeError(`${target}.show: output must be an Output`);
    }
    if (options.output !== undefined && !options.output.alive) {
        throw new GnoblinError("destroyed", `${target}.show`, `${options.output} is gone`);
    }
    const duration = motion(asked);
    ensureDesk();
    const desk = options.output === undefined ? currentDesk() : (deskByOutput.get(options.output) ?? null);
    if (desk === null) {
        throw new GnoblinError("no-output", `${target}.show`, "there is no output to show a workspace on");
    }
    const from = desk.shown;
    // A workspace is on one desk: one an other desk shows changes places with the one this desk showed.
    const other = desks.find((candidate) => candidate !== desk && candidate.shown === target);
    setShown(desk, target as Workspace);
    if (other !== undefined) {
        setShown(other, from);
    }
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

/** Work that waits for a time: the end of a slide, a script's `gnoblin.after`. Queued at the first frame past the time. */
function after(seconds: number, run: () => void): Timer {
    if (typeof seconds !== "number" || !Number.isFinite(seconds) || seconds < 0) {
        throw new TypeError("gnoblin.after: seconds must be a finite number of at least 0");
    }
    if (typeof run !== "function") {
        throw new TypeError("gnoblin.after: needs a function to run");
    }
    return hostApi.timers.after(seconds * 1000, run);
}

/** The workspace a window that has none joins: the one the output under the pointer shows, else the initial rule's. */
function newWindowWorkspace(): Workspace | null {
    return currentDesk()?.shown ?? initialWorkspace(workspaceRoot.list as Workspace[]);
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
        if (placeDialog(client)) {
            continue;
        }
        const target = newWindowWorkspace();
        if (target === null) {
            return;
        }
        moveClient(client, target, !preexisting.has(client));
    }
}

/** Attaches a workspace to the public root and adopts any clients that are waiting for one. */

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
    ensureDesk();
    fillDesks();
    adoptOrphans();
}

/**
 * Takes the workspace out of `gnoblin.workspaces`. Its windows move to the neighbour before it (the one after it, for the
 * first), and that neighbour is shown where the removed one was; none is closed.
 */
function removeWorkspace(workspace: Workspace): void {
    const state = wsState(workspace);
    if (!state.attached) {
        return;
    }
    const held = holders(workspaceRoot, workspace);
    state.attached = false;
    const at = workspaceRoot.list.indexOf(workspace);
    const neighbour = (workspaceRoot.list[at - 1] ?? workspaceRoot.list[at + 1]) as Workspace | undefined;
    workspaceRoot.list.splice(at, 1);
    for (const view of held) {
        view.members?.delete(workspace);
        queueFor(view, "onRemoved", [workspace, view.self]);
    }
    const target = neighbour ?? null;
    for (const desk of desks) {
        if (desk.shown === workspace) {
            desk.shown = null;
            // The neighbour takes its place, unless another output shows it already: then the first one no output shows.
            const free =
                neighbour !== undefined && deskShowing(neighbour) === null
                    ? neighbour
                    : ((workspaceRoot.list as Workspace[]).find((one) => deskShowing(one) === null) ?? null);
            setShown(desk, free);
        }
    }
    for (const client of [...state.clientsState.list] as Client[]) {
        moveClient(client, target, true);
    }
}

/**
 * A script that declared no workspace gets one floating `desktop`, so there is always somewhere for a window to go. A
 * script that never calls `defineConfig` is a program with no keys to switch workspaces, so one is all it needs; the
 * stock config's four workspaces come with `defineConfig` (`defaultConfig`).
 */
function ensureDefaultWorkspace(): void {
    if (workspaceRoot.list.length > 0) {
        return;
    }
    addWorkspace(new Workspace({ id: "desktop" }));
}

/** An output's rectangle in the compositor's coordinates. Null for a gone output or one with no size yet. */
function rectOf(output: Output): Rect | null {
    const state = stateOf(output);
    if (!state.alive) {
        return null;
    }
    const width = host.get(state.node, "width");
    const height = host.get(state.node, "height");
    if (typeof width !== "number" || typeof height !== "number" || !(width > 0) || !(height > 0)) {
        return null;
    }
    const x = host.get(state.node, "x");
    const y = host.get(state.node, "y");
    return { x: typeof x === "number" ? x : 0, y: typeof y === "number" ? y : 0, width, height };
}

/** What a layout tiles in on `output`: its rectangle less the bars on it. */
function areaOf(output: Output): Rect | null {
    const rect = rectOf(output);
    if (rect === null) {
        return null;
    }
    const insets = barInsets(output);
    return {
        x: rect.x,
        y: rect.y + insets.top,
        width: rect.width,
        height: Math.max(1, rect.height - insets.top - insets.bottom),
    };
}

/** The area of the output a desk shows on: null for a desk with no output, a parked one, or an output with no size. */
function areaOfDesk(desk: Desk): Rect | null {
    return desk.output === null || desk.parked ? null : areaOf(desk.output);
}

/** The primary output as a rectangle: the whole output. Null when the tree has no output yet. */
function screenRect(): Rect | null {
    const output = primaryOutput();
    return output === null ? null : rectOf(output);
}

/** The area windows are laid out in on the primary output. Null when the tree has no output yet. */
function screenArea(): Rect | null {
    const output = primaryOutput();
    return output === null ? null : areaOf(output);
}

/** The output rectangle `client` is on, or the primary output's when its workspace is on none. */
function outputRectOf(client: Client | null): Rect | null {
    const desk = client === null ? null : deskOfClient(client);
    return desk?.output && !desk.parked ? rectOf(desk.output) : screenRect();
}

/** The work area of the output `client` is on, or the primary output's when its workspace is on none. */
function workAreaOf(client: Client | null): Rect | null {
    const desk = client === null ? null : deskOfClient(client);
    return desk?.output && !desk.parked ? areaOf(desk.output) : screenArea();
}

/** The work area of the output that shows `workspace`, or the primary output's when none does. */
function workAreaOfWorkspace(workspace: Workspace): Rect | null {
    const desk = deskShowing(workspace);
    return desk?.output && !desk.parked ? areaOf(desk.output) : screenArea();
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

// ---- Layout visibility and workspace slides ----
// Owns layout placements, visibility writes, and the transition of windows between shown workspaces.

/** Compares all three components of two rotation or scale vectors. */

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
        const shown = isShown(workspace) || holding.has(workspace);
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
function startSlides(): void {
    for (const slide of slides.splice(0)) {
        const { from, to, direction, duration, progress } = slide;
        // The slide is as long as the output the incoming workspace is on is wide.
        const area = workAreaOfWorkspace(to);
        if (area === null) {
            continue;
        }
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
                if (client.alive && membership.get(client) === from && !isShown(from)) {
                    writeShown(client, false);
                    client.x = rest;
                }
            }
        });
    }
}

// ---- Client styles ----
// Owns style scopes, inheritance resolution, and writes of resolved style values to the host.

/** The style a collection of clients carries, and where it ranks among the selections. */
interface ScopeStyle {
    style: ClientStyle;
    /** The order the scope's style was first asked for in: the tie-break between selections of equal priority. */
    order: number;
}

/** Styles attached to scopes, keyed by the collection state they affect. */

const scopeStyles = new WeakMap<CollectionState, ScopeStyle>();
/** The `where()` selections that have a style, in the order they got it. */
const styledSelections: CollectionState[] = [];
/** Stable tie-break order for style selections created at equal priority. */

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
    /** The corner radius last written, or the node's own when nothing was written yet. */
    cornerRadius: number | null;
}

/** Per-client style resolution and last-applied host values. */

const clientStyles = new WeakMap<Client, ClientStyleState>();
/** The texts `takeNotices` has not handed out yet, and every text already said this generation. */
const notices: string[] = [];
/** Notice texts already emitted, to avoid repeating the same style warning. */

const noticed = new Set<string>();

/** Deduplicates a notice so one unsupported style is reported only once per generation. */

function say(text: string): void {
    if (!noticed.has(text)) {
        noticed.add(text);
        notices.push(text);
    }
}

/** Lazily allocates the per-client style bookkeeping used by the frame pass. */

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
            cornerRadius: null,
        };
        clientStyles.set(client, state);
    }
    return state;
}

/** Finds which workspace client collection owns a style scope. */

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

/** Creates and binds the client-owned style override layer. */

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

/** Resolves the effective style, retaining the last result after the client is gone. */

function computedStyleOf(client: Client): ComputedStyle {
    if (!stateOf(client).alive) {
        return clientStyles.get(client)?.computed ?? resolveStyle([]);
    }
    return resolveStyle(layersOf(client));
}

/** Reports style properties the renderer skipped for a client. */

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

/**
 * The most effects the compositor's tree keeps on one node (`MAX_EFFECTS` in crates/gnoblin-render/src/transaction.rs).
 * A style lists at most `MAX_STYLE_LIST` shadows and as many borders, and one backdrop blur makes the seventeenth.
 */
/** Maximum number of effects the compositor retains on one node, from transaction.rs MAX_EFFECTS. */

const MAX_NODE_EFFECTS = 16;

/**
 * Writes what a client's resolved style becomes on its node: the effects list, the opacity and the corner radius,
 * each when it changed.
 */
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
    if (mapping.effects.length > MAX_NODE_EFFECTS) {
        const error = new RangeError(
            `${label}: a window carries at most ${MAX_NODE_EFFECTS} effects, the style makes ${mapping.effects.length}`,
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
    const radiusBefore = state.cornerRadius ?? (host.published(node, "corner_radius") as number);
    if (mapping.cornerRadius !== radiusBefore) {
        refused("corner radius", host.set(node, "corner_radius", mapping.cornerRadius));
    }
    state.cornerRadius = mapping.cornerRadius;
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

// ---- Layout driver ----
// Owns per-frame layout invalidation, layout hooks, placement checks, and style application.

/**
 * The layout driver, run once at the end of each frame after every listener. For each workspace it first
 * notices what changed: a layout's own enumerable fields (a snapshot diff, because a field is a plain
 * property the host never sees written), the workspace's gaps, and the area. Then the layout hears of the
 * windows that came and went, and the shown workspace, when it is dirty, computes once and its placements
 * are written. A workspace that is not shown stays dirty until it is.
 */

function layout(): number {
    laying = true;
    try {
        return layoutWorkspaces();
    } finally {
        laying = false;
    }
}

/** Whether the layout pass is running, which a transition cannot start inside: its writes are the pass's own. */
/** Blocks transitions while layout-owned writes and callbacks are being applied. */

let laying = false;

/** Runs one guarded layout pass over workspaces and returns the number recomputed. */

function layoutWorkspaces(): number {
    fillDesks();
    // A workspace no output shows hears its hooks for the primary output's area; one an output shows, for that output's.
    const fallback = screenArea();
    let laidOut = 0;
    for (const workspace of workspaceRoot.list as Workspace[]) {
        const state = wsState(workspace);
        const desk = deskShowing(workspace);
        const area = desk === null ? fallback : areaOfDesk(desk);
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
            const covering = children
                .filter((client) => client.fullscreen || client.maximised)
                .map((client) => stateOf(client).node)
                .join(",");
            if (covering !== state.coverSeen) {
                state.coverSeen = covering;
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
            // A bar that came or went since a window was last fitted moves its content, before the layout places the
            // windows it owns; a window that has just arrived is placed with the bar it has, so it only records it.
            guard(`${workspace}.followBar`, () => {
                for (const client of children) {
                    if (client.alive) {
                        followBar(client, area);
                    }
                }
            });
            if (desk !== null && (state.dirty || !rectsEqual(state.area, area))) {
                const before = state.area;
                state.dirty = false;
                state.area = { ...area };
                if (before !== null && !rectsEqual(before, area)) {
                    // The work area moved under the windows (a bar came, went or grew): a maximised window fills it
                    // again, a snapped one tiles to its zone again and a floating one is brought back inside, as the the retired shell
                    // `Desk:arrange` did. The layout places what it owns after this.
                    guard(`${workspace}.refit`, () => {
                        for (const client of children) {
                            if (client.alive) {
                                refitClient(client, area);
                            }
                        }
                    });
                    guard(`${workspace}.layout.onAreaChanged`, () => target.onAreaChanged?.(before, { ...area }));
                }
                guard(`${workspace}.layout.compute`, () => {
                    const site = `${workspace}.layout.compute`;
                    // A client the script floats (a window rule, `client.floating`) keeps its own place, and so does one
                    // that covers the output (fullscreen) or fills the work area (maximised). A dialog stays with its
                    // parent, centred in it, and is never tiled.
                    const tiled = children.filter(
                        (client) =>
                            !stateOf(client).floating &&
                            !client.fullscreen &&
                            !client.maximised &&
                            client.parentWindow === null,
                    );
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
    lastFocus = focusedClient();
    startSlides();
    guard("gnoblin.workspaces.visibility", syncVisibility);
    guard("gnoblin.style", applyStyles);
    return laidOut;
}

// ---- Generation snapshots ----
// Owns workspace placement and window-state persistence across generation reloads.

/** Serializable state needed to reconcile workspaces and retained window geometry after reload. */

interface WorkspaceSnapshot {
    /** The workspace the primary output showed. */
    active: string | null;
    workspaces: { id: string; clients: number[] }[];
    /** The workspace each output showed, by the output's name. */
    outputs: { output: string; workspace: string }[];
    /** The rectangles windows go back to, which live in the generation and would be lost with it. */
    kept: KeptWindow[];
}

/** What one window keeps beyond where it is, each rectangle as `[x, y, width, height]`. */
interface KeptWindow {
    node: number;
    /** The rectangle a maximised window had before it was maximised. */
    floating: number[] | null;
    /** The rectangle a fullscreen window had before it was. */
    windowed: number[] | null;
    /** Where the user left a floating window. */
    leftAt: number[] | null;
    /** The zone a snapped window sits in, and the rectangle unsnapping returns it to. */
    zone: string | null;
    snapFloating: number[] | null;
    /** The titlebar height the window was last fitted to, so a reload that toggles decorations moves its content. */
    bar: number | null;
}

/** Encodes a rectangle as x, y, width, height for a generation snapshot. */

const asArray = (rect: Rect | null | undefined): number[] | null =>
    rect ? [rect.x, rect.y, rect.width, rect.height] : null;
/** Restores a four-number rectangle from a generation snapshot. */

const asRect = (numbers: number[] | null | undefined): Rect | null =>
    numbers && numbers.length === 4 ? { x: numbers[0], y: numbers[1], width: numbers[2], height: numbers[3] } : null;

/** The rectangles each window with one keeps, and the bar it was fitted to, for the next generation. */
function keptWindows(): KeptWindow[] {
    const kept: KeptWindow[] = [];
    for (const client of clientRoot.list as Client[]) {
        const rects = windowState.kept(client);
        const snap = snapRecord(client);
        const leftAt = (client as unknown as { floatingBounds?: Rect | null }).floatingBounds ?? null;
        // A window that had no bar is the first sight of the next generation, which fits it to the bar it has then.
        const fitted = fittedBar.get(client) ?? 0;
        const bar = fitted > 0 ? fitted : null;
        if (rects.floating === null && rects.windowed === null && snap === null && leftAt === null && bar === null) {
            continue;
        }
        kept.push({
            node: stateOf(client).node,
            floating: asArray(rects.floating),
            windowed: asArray(rects.windowed),
            leftAt: asArray(leftAt),
            zone: snap?.zone ?? null,
            snapFloating: asArray(snap?.floating),
            bar,
        });
    }
    return kept;
}

/** Gives the windows of the last generation the rectangles they were keeping. */
function restoreKept(kept: readonly KeptWindow[]): void {
    for (const entry of kept) {
        const client = clientsByNode.get(entry.node);
        if (client === undefined || !client.alive) {
            continue;
        }
        windowState.keep(client, { floating: asRect(entry.floating), windowed: asRect(entry.windowed) });
        if (typeof entry.bar === "number") {
            fittedBar.set(client, entry.bar);
        }
        const leftAt = asRect(entry.leftAt);
        if (leftAt !== null) {
            (client as unknown as { floatingBounds: Rect }).floatingBounds = leftAt;
        }
        const floating = asRect(entry.snapFloating);
        if (entry.zone !== null && floating !== null) {
            restoreSnap(client, { zone: entry.zone as SnapZone, floating });
        }
    }
}

/** Which window is on which workspace, for the next generation to reconcile against. */
function snapshotWorkspaces(): WorkspaceSnapshot {
    return {
        active: primaryDesk()?.shown?.id ?? null,
        workspaces: (workspaceRoot.list as Workspace[]).map((workspace) => ({
            id: workspace.id,
            clients: (wsState(workspace).clientsState.list as Client[]).map((client) => stateOf(client).node),
        })),
        outputs: deskOutputs().flatMap((output) => {
            const shown = deskByOutput.get(output)?.shown;
            return shown === null || shown === undefined ? [] : [{ output: output.name, workspace: shown.id }];
        }),
        kept: keptWindows(),
    };
}

/**
 * Puts the windows back on the workspaces of the same id. A window whose workspace this generation does not
 * declare moves to the first workspace; one that stays is not offered to the layout as an arrival, so a
 * floating window keeps the place the user gave it.
 */
function reconcileWorkspaces(previous: WorkspaceSnapshot): void {
    restoreKept(previous.kept ?? []);
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
    // Each output shows what it showed; the primary output's workspace is the one the snapshot calls active.
    const wanted: [Desk, Workspace][] = [];
    for (const entry of previous.outputs ?? []) {
        const output = deskOutputs().find((one) => one.name === entry.output);
        const workspace = (workspaceRoot.list as Workspace[]).find((one) => one.id === entry.workspace);
        const desk = output === undefined ? undefined : deskByOutput.get(output);
        if (desk !== undefined && workspace !== undefined) {
            wanted.push([desk, workspace]);
        }
    }
    const primary = primaryDesk();
    const shown = (workspaceRoot.list as Workspace[]).find((workspace) => workspace.id === previous.active);
    if (primary !== null && shown !== undefined && !wanted.some(([desk]) => desk === primary)) {
        wanted.push([primary, shown]);
    }
    for (const [desk, workspace] of wanted) {
        const other = desks.find((candidate) => candidate !== desk && candidate.shown === workspace);
        if (other !== undefined) {
            setShown(other, desk.shown);
        }
        setShown(desk, workspace);
    }
}

// ---- Native object lifecycle ----
// Owns node adoption, connection changes, and cleanup of disconnected objects.

/**
 * Wraps a node the first time it is seen. A node that appears while the shell runs is announced: its
 * collections are told it was added and its listeners that it is ready. A node that was there at boot is
 * not an event, because the config attaches after it exists.
 */
function adopt(node: NodeId, announce: boolean): void {
    const kind = host.kind(node);
    if (kind === "screen" && !outputsByNode.has(node)) {
        adoptScreen(node, announce);
        return;
    }
    if (kind === "window" && !clientsByNode.has(node)) {
        const client = new Client(CREATE, node);
        clientsByNode.set(node, client);
        clientRoot.list.push(client);
        if (announce) {
            queueFor(clientRoot, "onAdded", [client, clientRoot.self]);
            recompute(clientRoot, client);
            stateOf(client).readied = true;
            emitMember(client, holders(clientRoot, client), "onReady", [client]);
            // A window that appears joins the workspace the output under the pointer shows; a dialog joins its parent's
            // and is centred in it.
            if (opened && !placeDialog(client)) {
                const target = newWindowWorkspace();
                if (target !== null) {
                    moveClient(client, target, true);
                }
            }
        } else {
            preexisting.add(client);
        }
    }
}

/**
 * Wraps a screen node the first time it is seen and gives it a desk: the output-less desk a tree kept (or a parked one
 * that waited for it), else a new one that shows the first workspace no desk shows. A screen that appears while the
 * shell runs is announced; one that was there at boot is not, because the config attaches after it exists.
 */
function adoptScreen(node: NodeId, announce: boolean): void {
    const output = new Output(CREATE, node);
    outputsByNode.set(node, output);
    outputRoot.list.push(output);
    // A mirror and an output that is off have no place in the desktop, so they get no desk: nothing is laid out there.
    const wasParked = (outputSeen.get(output) as OutputSeen).detached ? false : giveDesk(output);
    if (!announce) {
        configureOutput(output);
        return;
    }
    queueFor(outputRoot, "onAdded", [output, outputRoot.self]);
    recompute(outputRoot, output);
    emitMember(output, holders(outputRoot, output), "onConnected", [output]);
    const desk = deskByOutput.get(output);
    // The windows that waited hidden come back, and one takes the keyboard when nobody holds it.
    if (
        wasParked &&
        desk !== undefined &&
        desk.shown !== null &&
        host.input.seats().length > 0 &&
        focusedClient() === null
    ) {
        const first = (wsState(desk.shown).clientsState.list as Client[]).find((one) => one.alive && !one.minimised);
        first?.focus();
    }
    configureOutput(output);
}

/**
 * Gives an output a desk: the output-less desk a tree kept (or a parked one that waited for it), else a new one that
 * shows the first workspace no desk shows. Returns whether the desk was parked.
 */
function giveDesk(output: Output): boolean {
    let desk = desks.find((candidate) => candidate.output === null);
    if (desk === undefined) {
        desk = { output: null, shown: null, parked: false };
        desks.push(desk);
    }
    const wasParked = desk.parked;
    desk.output = output;
    desk.parked = false;
    deskByOutput.set(output, desk);
    fillDesks();
    if (desk.shown !== null) {
        wsState(desk.shown).dirty = true;
    }
    return wasParked;
}

/**
 * A screen went away. Its windows move to the workspace another output shows (the retired shell's `Screen:on_destroyed`),
 * fitted to that output; with no other output they wait hidden on the parked desk for the next one, and the focus is
 * dropped, because a window with no output cannot hold it.
 */
function releaseScreen(output: Output): void {
    const held = leave(outputRoot, output, outputsByNode, OUTPUT_KEYS);
    takeDesk(output);
    emitMember(output, held, "onDisconnected", [output]);
    for (const state of held) {
        queueFor(state, "onRemoved", [output, state.self]);
    }
}

/** Takes an output's desk away: its windows move to the workspace another output shows, or the desk parks. */
function takeDesk(output: Output): void {
    const desk = deskByOutput.get(output);
    if (desk !== undefined) {
        desk.output = null;
        const survivor = desks.find((candidate) => candidate !== desk && candidate.output !== null);
        const shown = desk.shown;
        if (survivor !== undefined) {
            desks.splice(desks.indexOf(desk), 1);
            const target = survivor.shown ?? (workspaceRoot.list[0] as Workspace | undefined) ?? null;
            if (shown !== null && target !== null && shown !== target) {
                const area = areaOfDesk(survivor);
                for (const client of [...wsState(shown).clientsState.list] as Client[]) {
                    if (client.parentWindow === null) {
                        moveClient(client, target, true);
                        if (area !== null) {
                            refitClient(client, area);
                        }
                    }
                }
            }
            if (shown !== null) {
                wsState(shown).dirty = true;
            }
        } else {
            desk.parked = true;
            const holder = host.input.seats().length > 0 ? focusedClient() : null;
            if (holder !== null && shown !== null && membership.get(holder) === shown) {
                input.primary.clearFocus();
            }
        }
    }
    deskByOutput.delete(output);
}

// ---- Output configuration and events ----
// Owns asynchronous output applications and output-property event delivery.

/** The compositor's answer to an `Output.apply`, filled in a frame after the call. */
export interface OutputApplication {
    /** Whether the compositor has answered. */
    readonly done: boolean;
    /** Null while waiting and on success; else why the arrangement was left as it was, naming the key. */
    readonly error: string | null;
}

/** An output request awaiting its compositor reply and optional config notice path. */

interface PendingApplication {
    output: Output;
    result: { done: boolean; error: string | null };
    /** The config path (`outputs.DP-1`) to name in a notice when the compositor refuses; null for a script's own call. */
    path: string | null;
}

/** Pending applications indexed by the compositor's request token. */

const applications = new Map<number, PendingApplication>();

/** Sends one request for `output` to the compositor and tracks its answer. */
function applyOutput(
    output: Output,
    options: OutputConfig & { mirrorOf?: Output | string | null },
    target: string,
    path: string | null = null,
): OutputApplication {
    if (!stateOf(output).alive) {
        throw new GnoblinError("destroyed", target, "the output is gone");
    }
    const { mirrorOf, ...rest } = options;
    const source = mirrorOf instanceof Output ? mirrorOf.connector : mirrorOf;
    const request = outputRequest(mirrorOf === undefined ? rest : { ...rest, mirrorOf: source }, target);
    const token = host.outputs.apply(stateOf(output).node, request);
    if (token === undefined) {
        throw new GnoblinError("destroyed", target, "the output is gone");
    }
    const result = { done: false, error: null as string | null };
    applications.set(token, { output, result, path });
    return {
        get done() {
            return result.done;
        },
        get error() {
            return result.error;
        },
    };
}

/** Hands each answer to its application, tells the output's listeners, and reports a refused config entry. */
function settleApplications(): void {
    for (const answer of host.outputs.takeAnswers()) {
        const pending = applications.get(answer.token);
        if (pending === undefined) {
            continue;
        }
        applications.delete(answer.token);
        pending.result.done = true;
        pending.result.error = answer.error === "" ? null : answer.error;
        if (pending.result.error !== null && pending.path !== null) {
            // `mode: ...` is a key's own refusal, so it extends the path; anything else is about the entry as a whole.
            const keyed = /^[a-z_]+: /.test(pending.result.error);
            say(`${pending.path}${keyed ? "." : ": "}${pending.result.error}`);
        }
        if (stateOf(pending.output).alive) {
            emitMember(pending.output, holders(outputRoot, pending.output), "onApplied", [
                pending.output,
                pending.result.error,
            ]);
        }
    }
}

/** The `outputs` table of the running config, applied to each output as it appears and to the ones already here. */
let configuredOutputs: Readonly<Record<string, OutputConfig>> = {};

/** Applies the config entry for this output's connector, if the config has one. */
function configureOutput(output: Output): void {
    const setting = configuredOutputs[output.connector];
    if (setting === undefined) {
        return;
    }
    const path = `outputs.${output.connector}`;
    try {
        applyOutput(output, setting, `config.${path}`, path);
    } catch (error) {
        say(`${path}: ${error instanceof Error ? error.message : String(error)}`);
    }
}

/** Raises the events one output's changed properties imply, to the listeners that held it. */
function outputChanged(output: Output): void {
    const seen = outputSeen.get(output) as OutputSeen;
    const now = publishedOutput(stateOf(output).node);
    const held = holders(outputRoot, output);
    if (now.detached !== seen.detached) {
        seen.detached = now.detached;
        if (now.detached) {
            takeDesk(output);
        } else {
            giveDesk(output);
        }
    }
    if (now.mode !== seen.mode) {
        seen.mode = now.mode;
        emitMember(output, held, "onModeChanged", [output, now.mode]);
    }
    if (now.scale !== seen.scale) {
        seen.scale = now.scale;
        emitMember(output, held, "onScaleChanged", [output, now.scale]);
    }
    if (now.x !== seen.x || now.y !== seen.y || now.width !== seen.width || now.height !== seen.height) {
        const { x, y, width, height } = now;
        Object.assign(seen, { x, y, width, height });
        emitMember(output, held, "onGeometryChanged", [output, { x, y, width, height }]);
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

// ---- Frame events and listener queue ----
// Owns updates from the host, deferred callbacks, and generation readiness.

/** The window that had the first seat's keyboard focus when the last frame ended: what a closing window is checked against. */
let lastFocus: Client | null = null;

/**
 * Hands the keyboard to the neighbour of a focused window that closed, as the retired strip did for its columns and every
 * layout now does: the window after it in the workspace, else the one before; a dialog hands it back to its parent. Nothing
 * moves while the workspace is not shown, a minimised window is skipped, and a window the compositor has already focused
 * is left alone. A layout that has its own `onClientRemoved` (a strip's columns are not in window order) does it itself.
 */
function handOffFocus(workspace: Workspace, index: number, parent: Client | null): void {
    if (!isShown(workspace) || host.input.seats().length === 0 || focusedClient() !== null) {
        return;
    }
    const list = wsState(workspace).clientsState.list as Client[];
    const usable = (client: Client | undefined): client is Client =>
        client !== undefined && client.alive && !client.minimised;
    const next = [parent ?? undefined, list[index], list[index - 1]].find(usable);
    next?.focus();
}

/** Releases a destroyed host node and notifies the listeners that held its wrapper. */

function release(node: NodeId): void {
    const gone = outputsByNode.get(node);
    if (gone !== undefined) {
        releaseScreen(gone);
        return;
    }
    const client = clientsByNode.get(node);
    if (!client) {
        return;
    }
    const own = wsOf(client);
    const ownHeld = own ? holders(own.clientsState, client) : [];
    const wasFocused = lastFocus === client;
    const parent = client.parentWindow;
    const held = leave(clientRoot, client, clientsByNode, CLIENT_KEYS);
    input.clientGone(client);
    if (own) {
        const workspace = membership.get(client) as Workspace;
        membership.delete(client);
        const index = own.clientsState.list.indexOf(client);
        own.clientsState.list.splice(index, 1);
        own.dirty = true;
        if (wasFocused && typeof own.layout.onClientRemoved !== "function") {
            handOffFocus(workspace, index, parent);
        }
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
    if (now.decoration !== seen.decoration) {
        seen.decoration = now.decoration;
        emitMember(client, held, "onDecorationChanged", [client, now.decoration]);
        // The bar's room above the window came or went: the workspace's layout places it again.
        const own = wsOf(client);
        if (own) {
            own.dirty = true;
        }
    }
    if (now.focused !== seen.focused) {
        seen.focused = now.focused;
        emitMember(client, held, "onFocusChanged", [client, now.focused]);
    }
    if (now.x !== seen.x || now.y !== seen.y || now.width !== seen.width || now.height !== seen.height) {
        const { x, y, width, height } = now;
        const [dx, dy] = [x - seen.x, y - seen.y];
        Object.assign(seen, { x, y, width, height });
        emitMember(client, held, "onGeometryChanged", [client, { x, y, width, height }]);
        // A dialog moves with its parent.
        if (dx !== 0 || dy !== 0) {
            for (const dialog of dialogsOf(client)) {
                dialog.x += dx;
                dialog.y += dy;
            }
        }
    }
    if (now.parent !== seen.parent) {
        seen.parent = now.parent;
        if (opened && now.parent !== 0) {
            placeDialog(client);
        }
    }
}

/** Processes one batch of host tree changes and returns the queued callback count. */

function plan(created: NodeId[], touched: NodeId[], destroyed: NodeId[]): number {
    launches.frame();
    settleApplications();
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
        const output = outputsByNode.get(node);
        if (output) {
            outputChanged(output);
            recompute(outputRoot, output);
        }
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
/** Method names used to route compositor pointer edges to client listeners. */

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
        windowState.pointerEdge(event.kind);
        const client = clientsByNode.get(event.node);
        const method = POINTER_METHODS[event.kind];
        if (client && method && stateOf(client).alive) {
            const { x, y, button } = event;
            emitMember(client, clientHolders(client), method, [client, { x, y, button }]);
        }
    }
    return queue.length;
}

/** Number of deferred callbacks still waiting for the shell pump. */

function pending(): number {
    return queue.length;
}

/** Runs one deferred callback and returns the remaining queue length. */

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
    // The declaration is written here, after the generation that ended took its own back: a reload stages the new
    // generation before it retires the old one, so a write made while staging would be undone by that retire.
    if (decorations !== null) {
        declareDecorations(true);
    }
    for (const node of nodes) {
        const client = clientsByNode.get(node);
        if (client && !stateOf(client).readied) {
            stateOf(client).readied = true;
            emitMember(client, clientHolders(client), "onReady", [client]);
        }
    }
    return queue.length;
}

/** Transfers collected callback failures to the shell and starts an empty error batch. */

function takeErrors(): ErrorReport[] {
    const taken = reports;
    reports = [];
    return taken;
}

// ---- Input and focus integration ----
// Owns the shell's input adapters, focus lookup, stock requests, and keyboard actions.

/** Config values that override the compositor's edge tiling and drag constraints. */

const seatOverrides: { edgeTiling: boolean | null; dragConstraint: string | null } = {
    edgeTiling: null,
    dragConstraint: null,
};

/** Host input operations with config overrides layered over the published seat settings. */

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

/** Seat input manager wired to host focus, window stack, layouts, and the deferred callback queue. */

const input = new InputManager(seatHost, {
    clientOf: (node) => clientsByNode.get(node),
    stack: () => host.windows().flatMap((node) => clientsByNode.get(node) ?? []),
    workArea: (client) => workAreaOf((client as Client | undefined) ?? null),
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

// ---- Window policy and input actions ----
// Connects compositor focus and window requests to stock rules, gestures, and keyboard operations.

/** Whether a fullscreen window is showing on the workspace `output` shows: what makes its bars give way. */
function fullscreenShowing(output: { alive: boolean } | null): boolean {
    const desk = output === null ? primaryDesk() : (deskByOutput.get(output as Output) ?? null);
    const shown = desk === null || desk.parked ? null : desk.shown;
    if (shown === null) {
        return false;
    }
    return (wsState(shown).clientsState.list as Client[]).some((client) => client.fullscreen && !client.minimised);
}

barsGiveWayTo(fullscreenShowing);
barsOnOutputs(primaryOutput);

/** Calls the `onRequest` of the client's own listeners, then of the collections that hold it, until one handles it. */
function dispatchRequest(client: Client, request: ClientRequest<Client>): void {
    const sources: Source[] = [stateOf(client), ...clientHolders(client)];
    for (const source of sources) {
        for (const listener of [...source.listeners]) {
            const handler = handlerOf(listener, "onRequest");
            if (typeof handler !== "function") {
                continue;
            }
            try {
                handler.call(listener, client, request);
            } catch (error) {
                reports.push({ site: `${describeListener(listener)}.onRequest`, error });
            }
            if (request.handled) {
                return;
            }
        }
    }
}

/** Reads the first seat's focused window, or null when it has no managed client. */

/** The window the first seat's keyboard focus is on, if it is one of the compositor's. */
function focusedClient(): Client | null {
    const seat = host.input.seats()[0];
    return seat === undefined ? null : (clientsByNode.get(host.input.focus(seat)) ?? null);
}

/** Shows the workspace `step` places from the one the output under the pointer shows; nothing past either end. */
function stepWorkspace(step: number): void {
    const shown = currentDesk()?.shown ?? null;
    if (shown === null) {
        return;
    }
    const target = workspaceRoot.list[workspaceRoot.list.indexOf(shown) + step];
    if (target !== undefined) {
        showWorkspace(target as Workspace, { duration: SLIDE_SECONDS });
    }
}

/** The five actions a touchpad gesture runs: the retired shell's `Gestures.actions`. */
/** Built-in actions used by the gesture helpers. */

const gestureActions: GestureActions = {
    workspaceNext: () => stepWorkspace(1),
    workspacePrevious: () => stepWorkspace(-1),
    windowClose: () => focusedClient()?.close(),
    windowMinimise: () => {
        const client = focusedClient();
        if (client !== null) {
            windowState.minimise(client);
        }
    },
    windowToggleMaximise: () => {
        const client = focusedClient();
        if (client === null) {
            return;
        }
        if (client.maximised) {
            windowState.unmaximise(client);
        } else {
            windowState.maximise(client);
        }
    },
};

/** `gnoblin.gestures`: the five actions and the helpers that bind them to a swipe or a pinch. */
/** Public gesture bindings with the stock action set. */

const gestureTools: GestureTools = {
    actions: gestureActions,
    direction: gestureDirection,
    onSwipe: (map, threshold) => onSwipe(map, threshold, { actions: gestureActions }),
    onPinch: (map, threshold) => onPinch(map, threshold, { actions: gestureActions }),
};

/** The stock answers to a client's requests of its window, started with the module so no script has to ask. */
/** Installs the default client request handling before config-specific listeners are attached. */

windowState.start<Client>({
    clientOf: (node) => clientsByNode.get(node),
    siblings: (client) => {
        const own = membership.get(client) ?? null;
        return host
            .windows()
            .flatMap((node) => clientsByNode.get(node) ?? [])
            .filter((other) => (membership.get(other) ?? null) === own);
    },
    raise: (client) => client.raise(),
    lower: (client) => client.lower(),
    close: (client) => client.close(),
    focused: focusedClient,
    focus: (client) => {
        // A tree with no seat has nowhere to put the focus.
        if (host.input.seats().length === 0) {
            return;
        }
        if (client === null) {
            input.primary.clearFocus();
        } else {
            client.focus();
        }
    },
    onActiveWorkspace: (client) => {
        const own = membership.get(client);
        return own !== undefined && isShown(own);
    },
    output: (client) => outputRectOf(client ?? null),
    workArea: (client) => workAreaOf(client ?? null),
    framed,
    snapped: (client) => snappedZone(client) !== null,
    unsnap: (client) => {
        unsnapClient(client as unknown as SnapClient);
    },
    dispatch: dispatchRequest,
    report: (site, error) => {
        reports.push({ site, error });
    },
    now: () => host.input.now(),
    dragConstraint: () => {
        const seat = host.input.seats()[0];
        return seat === undefined ? undefined : seatHost.dragConstraint(seat);
    },
    edgeTiling: () => {
        const seat = host.input.seats()[0];
        return seat !== undefined && seatHost.settings(seat)?.edgeTiling === true;
    },
});

/** The keyboard move and resize mode: its keys, its hint and its end are the stock library's; the area is the shell's. */
repositioning.start({
    workArea: (client) => workAreaOf(client as unknown as Client),
    dragConstraint: () => {
        const seat = mainSeat();
        return seat === undefined ? undefined : seatHost.dragConstraint(seat);
    },
    focus: (client) => (client as unknown as Client).focus(),
});

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
    setFullscreen: (client, on) => {
        if (on) {
            windowState.enterFullscreen(client);
        } else {
            windowState.leaveFullscreen(client);
        }
    },
    setMaximised: (client, on) => {
        if (on) {
            windowState.maximise(client);
        } else {
            windowState.unmaximise(client);
        }
    },
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

/** Config overrides applied before seat focus defaults are resolved. */

let focusOverrides: FocusConfig = {};

/** Resolves effective focus policy from config overrides and the first seat's settings. */

function currentFocusSettings(): FocusSettings {
    const seat = host.input.seats()[0];
    const published = seat === undefined ? undefined : seatHost.settings(seat);
    const base = published === undefined ? FOCUS_DEFAULTS : seatFocusSettings(published);
    return focusSettings(focusOverrides, "config.focus", base);
}

/** The stock window rules and focus policy, once a config is registered. */
let stock: { rules: RuleEngine<Client>; focus: FocusPolicy<Client> } | null = null;

/** Replaces the config's stock rules and focus policy while preserving their listener order. */

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
        fromLauncher: (client) => client.fromLauncher,
        skipsFocus: (client) => rules.skipsFocus(client),
        alive: (client) => client.alive,
    });
    stock = { rules, focus };
    // The rules hear a new window first: `skipFocus` is what the focus policy reads when it hears the same one.
    clientRoot.self.addListener(rules);
    clientRoot.self.addListener(focus);
}

// ---- Decorations and session services ----
// Owns optional shell integrations and the always-on session services.

// The stock lock screen is part of the runtime, not of a config: a locked session must always have one to type into.
attachLock(host.lock);

/** Optional decoration manager, initialized when the stock titlebars are enabled. */

let decorations: Decorations | null = null;

/** Creates stock decorations once and routes titlebar actions through host state. */

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
                maximise: (client, on) => {
                    if (on) {
                        windowState.maximise(client as Client);
                    } else {
                        windowState.unmaximise(client as Client);
                    }
                },
                minimise: (client) => {
                    windowState.minimise(client as Client);
                },
                motion,
                // A press on the bar is a press on its window: the focus policy hears it as it hears one on the window.
                press: (client, event) => {
                    if (stateOf(client as Client).alive) {
                        emitMember(client as Client, clientHolders(client as Client), "onPointerPress", [
                            client,
                            event,
                        ]);
                    }
                },
                release: (client, event) => {
                    if (stateOf(client as Client).alive) {
                        emitMember(client as Client, clientHolders(client as Client), "onPointerRelease", [
                            client,
                            event,
                        ]);
                    }
                },
            },
        );
        clientRoot.self.addListener(decorations);
    }
}

/** Tells the compositor whether this shell draws window decorations, so a client that asks for server-side is told so. */
function declareDecorations(available: boolean): void {
    const seat = host.input.seats()[0];
    if (seat === undefined) {
        return;
    }
    const code = host.input.declareDecorations(seat, available);
    if (code != null) {
        reports.push({
            site: "Decorations.declare",
            error: new GnoblinError("refused", "Seat.decorations_available", `the host refused the write (${code})`),
        });
    }
}

/** Dialog decisions accumulated for the shell to consume. */

let verdicts: Verdict[] = [];

/** Dialog request manager backed by the shell callback queue. */

const dialogs = new DialogManager({
    schedule: (site, task) => {
        queue.push({ site, task });
    },
    verdict: (request, deny, reason) => {
        verdicts.push({ request, deny, reason });
    },
});

/** Idle watches and holds scheduled alongside regular listeners. */

const idleMonitor = new IdleMonitor({
    schedule: (site, task) => {
        queue.push({ site, task });
    },
});

/** Authentication manager connected to compositor-owned dialog observations. */

const authentication = new AuthenticationManager(dialogs);

/** Launch feedback manager resolved by windows associated with each launch. */

const launches = new LaunchFeedback({
    now: () => host.input.now(),
    schedule: (site, task) => {
        queue.push({ site, task });
    },
});

/**
 * Tells `launches` of the windows that appear, take focus or name their application. The listeners are attached
 * before any launch can begin, so the windows that exist now answer none.
 */
function trackLaunches(): void {
    const arrivals = new CollectionListener();
    arrivals.onAdded = (member) => {
        if (member instanceof Client) {
            launches.answered(member.appId);
        }
    };
    clientRoot.self.addListener(arrivals);
    const changes = new ClientListener();
    changes.onFocusChanged = (client, focused) => {
        if (focused) {
            launches.answered(client.appId);
        }
    };
    changes.onAppIdChanged = (client, appId) => {
        launches.answered(appId);
    };
    clientRoot.self.addListener(changes);
}

// ---- Config callbacks and integrations ----
// Owns the registrations and bridges used by config, menus, alerts, XR, and session APIs.

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

/**
 * Refuses a config whose document settings the validator would refuse, with the key path in the message. The
 * `autostart` commands go along: a host that owns the document commits them as its `autostart` section, so one that
 * would start a second keyring is refused here, while the running generation is still the old one.
 */
function checkDocument(patch: ConfigPlan["document"], autostart: ConfigPlan["autostart"]): void {
    if (Object.keys(patch).length === 0 && autostart.length === 0) {
        return;
    }
    const problems = host.config.check(JSON.stringify(patch), JSON.stringify(autostart));
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
        maximise: (client, on) => {
            if (on) {
                windowState.maximise(client as Client);
            } else {
                windowState.unmaximise(client as Client);
            }
        },
        minimise: (client) => {
            windowState.minimise(client as Client);
        },
        leaveFullscreen: (client) => {
            windowState.leaveFullscreen(client as Client);
        },
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
// ---- XR and session API bridges ----
// Adapts compositor-owned services to the stock XR, scene, and session modules.

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

/** XR view into host nodes, seats, clients, and input contexts. */

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

/** Runtime for the optional XR extension, attached to the scene and host adapters. */

const xrRuntime = new XRRuntime(xrWorld);

/** Monotonically increasing ids for operations exposed through the session API. */

let operationIds = 0;

/** Session-facing process, runtime, timer, bus, and focus APIs. */

const hostApi = new HostApi(
    {
        schedule: (site, task) => {
            queue.push({ site, task });
        },
        now: () => host.input.now(),
        spawn: (argv, options) => host.api.spawn(argv, options),
        kill: (id, signal) => host.api.kill(id, signal),
        begin: (verb) => host.api.begin(verb),
        status: () => host.api.status(),
        version: () => host.api.version(),
        signals: () => host.api.signals(),
        operationId: () => ++operationIds,
        error: (code, target, message) => new GnoblinError(code, target, message),
        // The entry module runs before the first `ready`: a reload asked for from there would ask again after it.
        loading: () => !opened,
    },
    {
        windows: () => [...clientRoot.list],
        workspaces: () => [...workspaceRoot.list],
        // `gnoblin.outputs` is the outputs module's; a shell without it has no `Output` kind to search.
        outputs: () => {
            if (!("outputs" in gnoblin) || typeof gnoblin.outputs !== "object" || gnoblin.outputs === null) {
                return null;
            }
            const outputs = gnoblin.outputs as Iterable<object>;
            return [...outputs];
        },
        focusedWindow: () => (clientRoot.self as Collection<Client>).focused,
    },
);

// ---- Transition rollback and animation ----
// Owns snapshots and staged frame passes used to commit or restore transition callbacks.

/**
 * What a transition callback can change that is not a tree write, put back when the transition fails: each workspace's
 * layout, gaps, name and dirty flag, the own fields of the layout it had, and every style. A layout the callback
 * assigned (and so was not the workspace's layout when the transition began) keeps the field changes made to it.
 */
function snapshotScriptState(): () => void {
    if (laying) {
        throw new GnoblinError("busy", "gnoblin.requestTransition", "cannot start inside a layout pass");
    }
    const restores: (() => void)[] = [];
    for (const workspace of workspaceRoot.list as Workspace[]) {
        const state = wsState(workspace);
        const { layout: held, gaps, name, dirty, fields, gapsSeen } = state;
        const own = layoutFields(held);
        restores.push(() => {
            Object.assign(state, { layout: held, gaps, name, dirty, fields, gapsSeen });
            for (const key of Reflect.ownKeys(held)) {
                if (Object.getOwnPropertyDescriptor(held, key)?.enumerable && !own.has(key)) {
                    Reflect.deleteProperty(held, key);
                }
            }
            for (const [key, value] of own) {
                Reflect.set(held, key, value);
            }
        });
    }
    restores.push(snapshotStyles(everyStyle));
    return () => {
        for (const restore of restores) {
            restore();
        }
    };
}

/**
 * Runs the layout and style passes once against the staged state, so their writes are staged with the callback's. A
 * placement a layout got wrong is a report the pass would log; here it fails the transition, which is what a
 * transition with an invalid target does.
 */
function settleStaged(): void {
    const before = reports.length;
    layout();
    const failed = reports.splice(before);
    if (failed.length > 0) {
        throw failed[0].error;
    }
}

/** Transition engine that stages writes, validates placements, and drives animation tracks. */

const transitions = new TransitionEngine({
    staging,
    motion,
    animated: (key) => Object.hasOwn(ANIMATED_KEYS, key),
    placed: (node) =>
        (host.get(node, "width") as number) > 0 &&
        (host.get(node, "height") as number) > 0 &&
        host.get(node, "visible") !== false,
    label: nodeName,
    makeTween: () => {
        const made = host.tween.create();
        if (typeof made !== "number") {
            throw new GnoblinError(
                String(made),
                "gnoblin.requestTransition",
                "the shell could not make the node that holds the transition's play state",
            );
        }
        return made;
    },
    dropTween: (tween) => {
        host.tween.destroy(tween);
    },
    schedule: (site, task) => {
        queue.push({ site, task });
    },
    snapshot: snapshotScriptState,
    settle: settleStaged,
    error: (code, target, message) => new GnoblinError(code, target, message),
});
hostApi.onTrackEnded = (event) => transitions.trackEnded(event);

// ---- Public facade and console inspection ----
// Exposes the host object model, registers its console representations, and connects the shell frame callbacks.

/** The public `gnoblin` object exposed to configuration code and the console. */

class Gnoblin {
    /** The optional extensions: `gnoblin.extensions.xr` is null until the XR program has a session (docs/xr.md). */
    readonly extensions = new Extensions(xrRuntime);
    readonly clients = clientRoot.self as Collection<Client>;
    readonly workspaces = workspaceRoot.self as WorkspaceCollection;
    /** The outputs, live across hotplug: an output is added when it is plugged in and removed when it goes. */
    readonly outputs = outputRoot.self as Collection<Output>;
    /** The seats of the compositor as input contexts, with the bindings that run on their chords and drags. */
    readonly input = input;
    /** Policy over the compositor's own consent, passphrase and authentication dialogs: listeners that may refuse a request. */
    readonly dialogs = dialogs;
    /** Watches for the session going idle, and holds that keep it awake. */
    readonly idle = idleMonitor;
    /** The session's polkit authentications: listeners that watch them and may refuse one. Nothing here can allow one. */
    readonly authentication = authentication;
    /** Launch feedback: the busy state between asking for an application and its window. */
    readonly launches = launches;

    /** The programs this shell started: `spawn`, `list`, `byId` and the listeners told when one ends. */
    readonly process = hostApi.process;
    /** Calls that run later, on the frame loop: `after`, and the monotonic clock `now`. */
    readonly timers = hostApi.timers;
    /** The session: its state, and the verbs that log out, suspend, power off or lock it. */
    readonly session = hostApi.session;
    /** What a script asks of the process that runs it: `reloadConfig`. */
    readonly runtime = hostApi.runtime;
    /** The windows in the order they were last focused. */
    readonly focus = hostApi.focus;
    /** The touchpad gesture actions (`workspace.next` and four more) and `onSwipe`, `onPinch` and `direction`. */
    readonly gestures = gestureTools;

    /** The duration the shell gives an animation: `seconds`, or 0 while the seat asks for reduced motion. */
    motion(seconds: number): number {
        return motion(seconds);
    }

    /**
     * Runs `callback` once, now, with every property write staged, and draws the change as one animation: each
     * written property moves from where it is drawn to the staged value over `options.duration` milliseconds. The
     * callback also holds layout changes (`workspace.layout = ...`), which move every window from its place. A
     * failure in the callback leaves the previous state. Nothing animates under reduced motion.
     */
    requestTransition(callback: () => void, options?: TransitionOptions): Transition {
        return transitions.request(callback, options);
    }

    /**
     * Starts a program and returns its `Process`. A string is one command line for `/bin/sh`; an array is the
     * program and its arguments. The same as `gnoblin.process.spawn`.
     */
    spawn(command: string | string[], options?: SpawnOptions): Process {
        return hostApi.process.spawn(command, options);
    }

    /** Shows a workspace, given as itself or by id, on the output under the pointer. See `WorkspaceCollection.show`. */
    show(workspace: Workspace | string, options?: ShowOptions): void {
        showWorkspace(workspace, options);
    }

    /** Runs `run` once, `seconds` from now: at the first frame after that time. `gnoblin.timers.after` takes ms. */
    after(seconds: number, run: () => void): Timer {
        return after(seconds, run);
    }

    /** The build identity: `gnoblin`, `gnome`, `mutter`, `js`, `api`, `gitRemote`, `gitSha` and `buildId`. */
    version(): Version {
        return hostApi.version();
    }

    /**
     * The live objects of one kind (`Window`, `Workspace`, and `Output` where the shell has outputs) that match a
     * table of field values or a predicate, in list order. An unknown kind or field throws and names what works.
     */
    find(kind: string, filter?: Record<string, string | number | boolean> | ((object: never) => unknown)): object[] {
        return hostApi.find(kind, filter);
    }

    /**
     * Calls `fn` for every `name` the compositor raises, `namespace.object.signal` (hyphens are accepted for
     * underscores). An unknown name throws and lists the near ones.
     */
    on(name: string, fn: (event: SignalEvent) => void): Subscription {
        return hostApi.bus.on(name, fn);
    }

    /** Like `on`, for the first signal only. */
    once(name: string, fn: (event: SignalEvent) => void): Subscription {
        return hostApi.bus.once(name, fn);
    }

    /** Detaches a subscription, or every handler `fn` of the event `name`. */
    off(target: Subscription | string, fn?: (event: SignalEvent) => void): void {
        hostApi.bus.off(target, fn);
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
        checkDocument(plan.document, plan.autostart);
        effectiveConfig = merged;
        for (const planned of plan.workspaces) {
            addWorkspace(
                new Workspace({ id: planned.id, name: planned.name, layout: planned.layout, gaps: planned.gaps }),
            );
        }
        if (plan.workspaceSets.length > 0) {
            (workspaceRoot.self as WorkspaceCollection).sets.define([...plan.workspaceSets]);
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
        configuredOutputs = plan.outputs;
        for (const output of liveOutputs()) {
            configureOutput(output);
        }
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
                const shown = currentDesk()?.shown ?? null;
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

/** The object model installed as `globalThis.gnoblin` for this generation. */

export const gnoblin = new Gnoblin();
trackLaunches();

// ---- Console and shell entry ----
// Registers inspectable objects, adopts the initial tree, and exposes frame callbacks to the shell.

// What the console shows of the host's own objects. A client is the short block of what a person asks about a window;
// nested inside another value (a collection's member) it is only its id, app and title.
/** Concise client fields shown by the console inspector. */

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
inspect.register(Output, (output) => ({
    name: "Output",
    id: output.id,
    fields: {
        name: output.name,
        mode: output.mode,
        scale: output.scale,
        primary: output.primary,
        x: output.x,
        y: output.y,
        width: output.width,
        height: output.height,
        workspace: output.workspace?.id ?? null,
        ...(output.connected ? {} : { connected: false }),
    },
    brief: { name: output.name, mode: output.mode },
}));
inspect.register(Collection, (collection) => ({
    name: "Collection",
    fields: {},
    items: [...collection],
}));

for (const node of host.screens()) {
    adopt(node, false);
}
for (const node of host.windows()) {
    adopt(node, false);
}

Object.defineProperty(globalThis, "gnoblin", { value: gnoblin, enumerable: false });
// The console's helpers: `inspect.clients()` and the rest are typed at a console prompt without an import. Unlike
// `gnoblin` the name is a convenience and not part of the object model, so a script may replace it.
Object.defineProperty(globalThis, "inspect", { value: inspect, enumerable: false, writable: true, configurable: true });
host.entry = {
    render: (value, maxBytes) => inspect.render(value, { maxBytes }),
    ui: () =>
        runLockFrame(
            (site, error) => reports.push({ site, error }),
            () => {
                try {
                    xrRuntime.frame();
                } catch (error) {
                    reports.push({ site: "XRRuntime.frame", error });
                }
            },
        ),
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
    wants: () => (idleMonitor.wanted ? 1 : 0),
    idleReading: (reading) => {
        idleMonitor.deliver(reading);
        return queue.length;
    },
    idleLost: () => {
        idleMonitor.lost();
        return queue.length;
    },
    takeIdleCommands: () => idleMonitor.takeCommands(),
    idleHolders: () => idleMonitor.holders(),
    idleFailed: (holder) => idleMonitor.failed(holder),
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
    apiFrame: (batch) => {
        // Only what this frame queued: calls an earlier stage left over wait for the frame's own budget.
        const before = queue.length;
        hostApi.frame(batch);
        return queue.length - before;
    },
    cancelAll: () => {
        input.cancelAll();
        xrRuntime.release();
        sceneReleaseAll();
        transitions.release();
        lockRelease();
        uiReleaseAll();
        hostApi.retire();
        if (decorations !== null) {
            declareDecorations(false);
        }
        return queue.length;
    },
};
