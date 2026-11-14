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

import { desugarConfig, defaultConfig } from "gnoblin/config";
import type { GnoblinConfig } from "gnoblin/config";
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
import type { StockLayoutName, WorkspaceOptions } from "gnoblin/workspace";
import { InputManager } from "gnoblin/input";
import type { InputHost, InputMessage } from "gnoblin/input";
import { attachUi, runUiFrame, uiNodeGone, uiReleaseAll } from "gnoblin/ui";
import type { UiBackend } from "gnoblin/ui";
import { Decorations } from "gnoblin/titlebar";

type NodeId = number;
type Scalar = string | number | boolean;

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

interface Entry {
    plan(created: NodeId[], touched: NodeId[], destroyed: NodeId[]): number;
    next(): number;
    pending(): number;
    ready(nodes: NodeId[]): number;
    /** Hands the frame's claimed chords and gestures to the input matcher; returns how many calls wait. */
    input(messages: InputMessage[]): number;
    /** Cancels what the input matcher has in flight and releases every claim; returns how many calls wait. */
    cancelAll(): number;
    takeErrors(): ErrorReport[];
    /** Lays out the workspaces that changed, once each; returns how many it laid out. */
    layout(): number;
    /** Runs the UI toolkit's frame: measurements and pointer events in, every canvas arranged and painted. */
    ui(): void;
    /** Gives a config that declared no workspace the stock one. */
    finishLoad(): void;
    snapshot(): WorkspaceSnapshot;
    reconcile(previous: WorkspaceSnapshot): void;
}

interface Host {
    kind(node: NodeId): string | undefined;
    get(node: NodeId, key: string): Scalar | undefined;
    published(node: NodeId, key: string): Scalar | undefined;
    label(node: NodeId): string | undefined;
    set(node: NodeId, key: string, value: Scalar): string | undefined;
    call(node: NodeId, method: string): string | undefined;
    windows(): NodeId[];
    screens(): NodeId[];
    spawn(command: string | string[]): string | undefined;
    input: InputHost;
    ui: UiBackend;
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

/** Observes the events of a client: attach it to one client, or to a collection for every member. */
export class ClientListener {
    onTitleChanged?(client: Client, title: string): void;
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

const CLIENT_KEYS = ["title", "app_id", "focused", "fullscreen", "maximised", "minimised", "x", "y", "width", "height"];

/** What listeners were last told about a client, to raise an event only for a real change. */
interface Seen {
    title: string;
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
    frozen: Record<string, Scalar | undefined> | null;
    listeners: object[];
    readied: boolean;
    seen: Seen | null;
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
    const frozen: Record<string, Scalar | undefined> = {};
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

    get fullscreen(): boolean {
        return read<boolean>(this, "fullscreen");
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

    focus(): void {
        callMethod(this, "focus");
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
     * what it must equal, or an array of values of which one must.
     */
    where(query: Query): Collection<T> {
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
        return new Collection<T>(CREATE, {
            parent: state,
            query: { ...query },
            list: [],
            members: null,
            keys: state.keys,
            listeners: [],
        });
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
    /** Shows a workspace on the primary display, given as itself or by id. */
    show(workspace: Workspace | string): void {
        showWorkspace(workspace);
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

/** `gnoblin.workspaces.show(workspace)`. */
function showWorkspace(workspace: Workspace | string): void {
    const target = typeof workspace === "string" ? workspaceRoot.list.find((w) => w.id === workspace) : workspace;
    if (target === undefined) {
        throw new GnoblinError("unknown-workspace", "gnoblin.workspaces.show", `there is no workspace "${workspace}"`);
    }
    if (!wsState(target).attached) {
        throw new GnoblinError("detached", `${target}.show`, "add the workspace to gnoblin.workspaces first");
    }
    setActive(target as Workspace);
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
    return { x: 0, y: 0, width, height };
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
 * The layout driver, run once at the end of each frame after every listener. For each workspace it first
 * notices what changed: a layout's own enumerable fields (a snapshot diff, because a field is a plain
 * property the host never sees written), the workspace's gaps, and the area. Then the layout hears of the
 * windows that came and went, and the shown workspace, when it is dirty, computes once and its placements
 * are written. A workspace that is not shown stays dirty until it is.
 */
function layout(): number {
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
            const context: LayoutContext<Client> = {
                workspace,
                children,
                area: { ...area },
                gap: { ...state.gaps },
                focused: children.find((client) => client.focused) ?? null,
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
                    for (const [client, bounds] of checkPlacements(target.compute(context), children, site)) {
                        writeBounds(client, bounds);
                    }
                });
                laidOut += 1;
            }
            // What the hooks and compute did to the layout's own fields is not a change for the next frame.
            state.fields = layoutFields(state.layout);
        });
    }
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

const input = new InputManager(host.input, {
    clientOf: (node) => clientsByNode.get(node),
    stack: () => host.windows().flatMap((node) => clientsByNode.get(node) ?? []),
    layouts: () => workspaceRoot.list.map((workspace) => wsState(workspace as Workspace).layout),
    schedule: (site, task) => {
        queue.push({ site, task });
    },
    report: (site, error) => {
        reports.push({ site, error });
    },
    error: (code, target, message) => new GnoblinError(code, target, message),
});

attachUi(host.ui, (site, error) => {
    reports.push({ site, error });
});

/** The stock titlebars, once a config asks for them. */
let decorations: Decorations | null = null;

function decorate(): void {
    if (decorations === null) {
        decorations = new Decorations((client) => stateOf(client).node);
        clientRoot.self.addListener(decorations);
    }
}

class Gnoblin {
    readonly clients = clientRoot.self as Collection<Client>;
    readonly workspaces = workspaceRoot.self as WorkspaceCollection;
    /** The seats of the compositor as input contexts, with the bindings that run on their chords and drags. */
    readonly input = input;

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

    /** Shows a workspace, given as itself or by id, on the primary display. */
    show(workspace: Workspace | string): void {
        showWorkspace(workspace);
    }

    /**
     * The hook `defineConfig` calls with the config it was given: the declared workspaces become real
     * `Workspace` objects in `gnoblin.workspaces`.
     */
    __register(config: GnoblinConfig): void {
        const plan = desugarConfig(config);
        for (const planned of plan.workspaces) {
            addWorkspace(new Workspace({ id: planned.id, layout: planned.layout, gaps: planned.gaps }));
        }
        if (plan.decorations) {
            decorate();
        }
    }
}

export const gnoblin = new Gnoblin();

for (const node of host.windows()) {
    adopt(node, false);
}

Object.defineProperty(globalThis, "gnoblin", { value: gnoblin, enumerable: false });
host.entry = {
    ui: runUiFrame,
    plan,
    next,
    pending,
    ready,
    takeErrors,
    layout,
    finishLoad: ensureDefaultWorkspace,
    snapshot: snapshotWorkspaces,
    reconcile: reconcileWorkspaces,
    input: (messages) => {
        input.deliver(messages);
        return queue.length;
    },
    cancelAll: () => {
        input.cancelAll();
        uiReleaseAll();
        return queue.length;
    },
};
