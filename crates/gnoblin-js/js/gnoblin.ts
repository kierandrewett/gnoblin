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
    takeErrors(): ErrorReport[];
}

interface Host {
    kind(node: NodeId): string | undefined;
    get(node: NodeId, key: string): Scalar | undefined;
    published(node: NodeId, key: string): Scalar | undefined;
    set(node: NodeId, key: string, value: Scalar): string | undefined;
    call(node: NodeId, method: string): string | undefined;
    windows(): NodeId[];
    workspaces(): NodeId[];
    spawn(command: string | string[]): string | undefined;
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

interface Invocation {
    source: Source;
    listener: object;
    method: string;
    args: unknown[];
}

const queue: Invocation[] = [];
let reports: ErrorReport[] = [];

function handlerOf(listener: object, method: string): unknown {
    return Reflect.get(listener, method);
}

function describeListener(listener: object): string {
    const name = Reflect.get(listener, "constructor")?.name;
    return typeof name === "string" && name ? name : "listener";
}

/** Runs one queued call if its listener is still attached; a throw is reported, never propagated. */
function run(call: Invocation): void {
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

const CLIENT_KEYS = ["title", "app_id", "focused", "fullscreen", "x", "y", "width", "height", "workspace"];
const WORKSPACE_KEYS = ["index", "active"];

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
    const target = `Client#${state.node}.${key}`;
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

function callMethod(client: Client, method: string): void {
    const state = stateOf(client);
    const target = `Client#${state.node}.${method}()`;
    if (!state.alive) {
        throw new GnoblinError("destroyed", target, "the client is gone");
    }
    const code = host.call(state.node, method);
    if (code != null) {
        refuse(code, target, "client");
    }
}

const clientsByNode = new Map<NodeId, Client>();
const workspacesByNode = new Map<NodeId, Workspace>();

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
        return String(stateOf(this).node);
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
        const target = `Client#${stateOf(this).node}.bounds`;
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

    /** The workspace the client is on, or null. */
    get workspace(): Workspace | null {
        return workspacesByNode.get(read<number>(this, "workspace")) ?? null;
    }

    addListener(listener: ClientListener): void {
        assertListener(listener, `Client#${stateOf(this).node}.addListener`);
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
        return `Client#${stateOf(this).node}`;
    }
}

/** A workspace the compositor reports. Read-only for now. */
export class Workspace {
    constructor(token: symbol, node: NodeId) {
        if (token !== CREATE) {
            throw new TypeError("a Workspace is made by the compositor, not by a script");
        }
        states.set(this, { node, alive: true, frozen: null, listeners: [], readied: false, seen: null });
    }

    get id(): string {
        return String(stateOf(this).node);
    }

    get alive(): boolean {
        return stateOf(this).alive;
    }

    get index(): number {
        return read<number>(this, "index");
    }

    get active(): boolean {
        return read<boolean>(this, "active");
    }

    toString(): string {
        return `Workspace#${stateOf(this).node}`;
    }
}

type Member = Client | Workspace;

interface CollectionState extends Source {
    self: Collection<Member>;
    /** The collection a `where` narrows; null for a root collection, which holds `list`. */
    parent: CollectionState | null;
    query: Query | null;
    list: Member[];
    /** The members a view last told its listeners of, kept only while it has listeners. */
    members: Set<Member> | null;
    keys: readonly string[];
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
    Workspace: ["id", "index", "active"],
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

function root<T extends Member>(kind: string): CollectionState {
    const collection = new Collection<T>(CREATE, {
        parent: null,
        query: null,
        list: [],
        members: null,
        keys: QUERY_KEYS[kind],
        listeners: [],
    });
    return collectionState(collection);
}

const clientRoot = root<Client>("Client");
const workspaceRoot = root<Workspace>("Workspace");

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
        }
    } else if (kind === "workspace" && !workspacesByNode.has(node)) {
        const workspace = new Workspace(CREATE, node);
        workspacesByNode.set(node, workspace);
        workspaceRoot.list.push(workspace);
        if (announce) {
            queueFor(workspaceRoot, "onAdded", [workspace, workspaceRoot.self]);
            recompute(workspaceRoot, workspace);
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
    if (client) {
        const held = leave(clientRoot, client, clientsByNode, CLIENT_KEYS);
        emitMember(client, held, "onClosed", [client]);
        for (const state of held) {
            queueFor(state, "onRemoved", [client, state.self]);
        }
        return;
    }
    const workspace = workspacesByNode.get(node);
    if (workspace) {
        for (const state of leave(workspaceRoot, workspace, workspacesByNode, WORKSPACE_KEYS)) {
            queueFor(state, "onRemoved", [workspace, state.self]);
        }
    }
}

/** Raises the events one client's changed properties imply, to the listeners that held it before the change. */
function changed(client: Client): void {
    const state = stateOf(client);
    const seen = state.seen as Seen;
    const now = publishedValues(state.node);
    const held = holders(clientRoot, client);
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
            continue;
        }
        const workspace = workspacesByNode.get(node);
        if (workspace) {
            recompute(workspaceRoot, workspace);
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
    for (const node of nodes) {
        const client = clientsByNode.get(node);
        if (client && !stateOf(client).readied) {
            stateOf(client).readied = true;
            emitMember(client, holders(clientRoot, client), "onReady", [client]);
        }
    }
    return queue.length;
}

function takeErrors(): ErrorReport[] {
    const taken = reports;
    reports = [];
    return taken;
}

class Gnoblin {
    readonly clients = clientRoot.self as Collection<Client>;
    readonly workspaces = workspaceRoot.self as Collection<Workspace>;

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
}

export const gnoblin = new Gnoblin();

for (const node of [...host.windows(), ...host.workspaces()]) {
    adopt(node, false);
}

Object.defineProperty(globalThis, "gnoblin", { value: gnoblin, enumerable: false });
host.entry = { plan, next, pending, ready, takeErrors };
