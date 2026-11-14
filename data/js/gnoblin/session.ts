// The host API that is not about windows: `gnoblin.session`, `gnoblin.runtime`, `gnoblin.process`, `gnoblin.timers`,
// `gnoblin.on`, `gnoblin.find`, `gnoblin.focus` and `gnoblin.version`.
// `HostApi` is plain TypeScript over a few functions of the shell (`ApiHost`), so it runs without a compositor. The
// shell calls `frame` once a frame with what happened outside the script: a call the compositor finished, a signal it
// raised, a process that ended. Every listener and callback runs as a queued call under the usual wall-clock budget.
// Errors are typed. A call that cannot be made at all throws (`TypeError` for a bad argument). A call that is made and
// refused returns an `Operation` that fails with an `OperationError`; its `code` is one of `docs/shell-api/errors.md`.

import { Operation, Timers } from "gnoblin/operation";
import type { ClockHost, OperationErrorCode, OperationHost, Subscription } from "gnoblin/operation";
import { ProcessTable } from "gnoblin/process";
import type { ExitEvent } from "gnoblin/process";
import type { TrackEndedEvent } from "gnoblin/transition";

/** The lock states the compositor reports. */
export type LockState = "unlocked" | "covering" | "locked" | "failsafe";

/**
 * The session as the compositor last published it. `state` is `running` while the shell has a connection to the
 * compositor and `unavailable` when it has none. `lockState` is present only while `lockAvailable` is true:
 * unavailable does not mean unlocked.
 */
export interface SessionStatus {
    readonly state: "running" | "unavailable";
    readonly lockAvailable: boolean;
    /** How many lock state changes the session has published. */
    readonly revision: number;
    readonly lockState?: LockState;
}

/** What `logout`, `poweroff`, `reboot`, `suspend` and `hibernate` succeed with: the compositor took the request. */
export interface Accepted {
    readonly accepted: true;
}

/** What `lock` succeeds with: a shell client received the request. Read `status()` to see the session lock. */
export interface LockRequest {
    readonly dispatched: boolean;
}

/** What `gnoblin.runtime.reloadConfig()` succeeds with once the compositor accepted the reloaded document. */
export interface ReloadResult {
    readonly ok: true;
    readonly action: "config reload";
    /** The revision of the settings the compositor now holds. */
    readonly settingsRevision: number;
    /** The sections the commit dropped, one sentence each. */
    readonly warnings: readonly string[];
    /** The settings accepted but not applied (read only at a process's start, or by nothing), one sentence each. */
    readonly notApplied: readonly string[];
}

/** The build identity. A field the build does not know is `"unknown"`; remote credentials are removed. */
export interface Version {
    readonly gnoblin: string;
    readonly gnome: string;
    readonly mutter: string;
    /** The script engine. */
    readonly js: string;
    /** The control API version, `major.minor.patch`. */
    readonly api: string;
    readonly gitRemote: string;
    readonly gitSha: string;
    readonly buildId: string;
}

/** A schema signal as a handler receives it: its name, the object that raised it, and its fields in camelCase. */
export interface SignalEvent {
    /** `namespace.object.signal`, with underscores. */
    readonly name: string;
    /** The identity of the object that raised it. */
    readonly object: number;
    readonly [field: string]: unknown;
}

/** What the shell hands the module: the host's functions, the queue and the clock. */
export interface ApiHost extends ClockHost {
    /** The native spawn: argv and options as JSON; answers `{id, pid}` or `{error}` as JSON. */
    spawn(argv: string, options: string): string;
    /** The native kill: an error as JSON, or undefined when the signal was sent. */
    kill(id: number, signal: string): string | undefined;
    /** A fresh operation id. */
    operationId(): number;
    /** The error a failed call throws: `code` says why, `target` names the call. */
    error(code: string, target: string, message: string): Error;
    /** Starts a session verb and answers `{ticket}` or `{error}` as JSON. */
    begin(verb: string): string;
    /** The session's published state as JSON, or undefined when the shell has no session connection. */
    status(): string | undefined;
    /** The build identity as JSON. */
    version(): string;
    /** The schema's signals as JSON: `[{name, fields}]`. */
    signals(): string;
    /** True while the user's entry module is being evaluated. */
    loading(): boolean;
}

type Host = ApiHost;

interface Failure {
    code: string;
    message: string;
}

interface OperationEvent {
    kind: "op";
    ticket: number;
    ok: boolean;
    value?: unknown;
    error?: Failure;
}

interface SignalMessage {
    kind: "signal";
    name: string;
    object: number;
    fields: Record<string, unknown>;
}

interface LostMessage {
    kind: "lost";
    message: string;
}

/** The compositor's `track_ended` signal, as the host collects it (`data/js/gnoblin/transition.ts`). */
interface TrackEndedMessage extends TrackEndedEvent {
    kind: "trackEnded";
}

type HostEvent = OperationEvent | SignalMessage | LostMessage | ExitEvent | TrackEndedMessage;

// ---- the event bus -----------------------------------------------------------------------------------------------

/** The most handlers the bus holds, as the retired Lua host's. */
export const MAX_HANDLERS = 1024;

function camel(name: string): string {
    return name.replace(/_([a-z0-9])/g, (_, letter: string) => letter.toUpperCase());
}

/** The edit distance of two names, for the near matches an unknown event name lists. */
function distance(a: string, b: string): number {
    let previous = Array.from({ length: b.length + 1 }, (_, index) => index);
    for (let row = 1; row <= a.length; row++) {
        const current = [row];
        for (let column = 1; column <= b.length; column++) {
            const cost = a[row - 1] === b[column - 1] ? 0 : 1;
            current.push(Math.min(previous[column] + 1, current[column - 1] + 1, previous[column - 1] + cost));
        }
        previous = current;
    }
    return previous[b.length];
}

/** How many edits away a name may be and still be offered as a near match. */
const NEAR_DISTANCE = 3;

/** The known names that are near `name`: the same last segment, or a small edit distance. */
export function nearMatches(name: string, known: readonly string[]): string[] {
    const last = name.split(".").pop() ?? name;
    return known
        .map((candidate) => ({ candidate, score: distance(candidate, name) }))
        .filter(({ candidate, score }) => candidate.endsWith(`.${last}`) || score <= NEAR_DISTANCE)
        .sort((a, b) => a.score - b.score)
        .slice(0, 5)
        .map(({ candidate }) => candidate);
}

interface Handler {
    name: string;
    fn: (event: SignalEvent) => void;
    once: boolean;
    /** False once the handler was detached by a script, so a call already queued does not run. */
    active: boolean;
}

/** `gnoblin.on`: handlers for the schema's signals, by their `namespace.object.signal` names. */
export class EventBus {
    #host: Host;
    #known: Set<string> | null = null;
    #handlers: Handler[] = [];

    constructor(host: Host) {
        this.#host = host;
    }

    /** Every name `on` accepts, in schema order. */
    get names(): string[] {
        return [...this.#signals()];
    }

    #signals(): Set<string> {
        if (this.#known === null) {
            const table = JSON.parse(this.#host.signals()) as { name: string }[];
            this.#known = new Set(table.map((entry) => entry.name));
        }
        return this.#known;
    }

    /** The schema spelling of a name: hyphens are underscores. An unknown name throws and lists the near ones. */
    canonical(name: string, site: string): string {
        if (typeof name !== "string") {
            throw new TypeError(`${site}: the event name must be a string`);
        }
        const schema = name.replaceAll("-", "_");
        const known = this.#signals();
        if (!known.has(schema)) {
            const near = nearMatches(schema, [...known]);
            const hint = near.length > 0 ? `; near matches: ${near.join(", ")}` : "";
            throw new TypeError(
                `${site}: no schema signal named \`${schema}\`; expected \`namespace.object.signal\`${hint}`,
            );
        }
        return schema;
    }

    #add(name: string, fn: (event: SignalEvent) => void, once: boolean, site: string): Subscription {
        const schema = this.canonical(name, site);
        if (typeof fn !== "function") {
            throw new TypeError(`${site}: needs a function to call`);
        }
        if (this.#handlers.length >= MAX_HANDLERS) {
            throw new RangeError(`${site}: ${MAX_HANDLERS} event handlers are already attached`);
        }
        const handler: Handler = { name: schema, fn, once, active: true };
        this.#handlers.push(handler);
        return { unsubscribe: () => this.#remove(handler) };
    }

    on(name: string, fn: (event: SignalEvent) => void): Subscription {
        return this.#add(name, fn, false, "gnoblin.on");
    }

    once(name: string, fn: (event: SignalEvent) => void): Subscription {
        return this.#add(name, fn, true, "gnoblin.once");
    }

    /** Detaches a subscription, or every handler `fn` of the event `name`. */
    off(target: Subscription | string, fn?: (event: SignalEvent) => void): void {
        if (typeof target === "string") {
            const schema = this.canonical(target, "gnoblin.off");
            for (const handler of this.#handlers.filter(
                (h) => h.name === schema && (fn === undefined || h.fn === fn),
            )) {
                this.#remove(handler);
            }
        } else if (target !== null && typeof target === "object" && typeof target.unsubscribe === "function") {
            target.unsubscribe();
        } else {
            throw new TypeError("gnoblin.off: expects a subscription, or an event name and the handler");
        }
    }

    #remove(handler: Handler): void {
        handler.active = false;
        this.#detach(handler);
    }

    #detach(handler: Handler): void {
        const index = this.#handlers.indexOf(handler);
        if (index >= 0) {
            this.#handlers.splice(index, 1);
        }
    }

    /** One signal the compositor raised: each handler of its name is queued. */
    deliver(message: SignalMessage): void {
        const event: Record<string, unknown> = { name: message.name, object: message.object };
        for (const [field, value] of Object.entries(message.fields)) {
            event[camel(field)] = value;
        }
        for (const handler of this.#handlers.filter((h) => h.name === message.name)) {
            if (handler.once) {
                // Out of the list now, so a second signal in the same frame does not queue it again.
                this.#detach(handler);
            }
            this.#host.schedule(`gnoblin.on(${message.name})`, () => {
                if (handler.active) {
                    handler.fn(event as unknown as SignalEvent);
                }
            });
        }
    }

    clear(): void {
        for (const handler of [...this.#handlers]) {
            this.#remove(handler);
        }
    }
}

// ---- the session -------------------------------------------------------------------------------------------------

/** What `Calls` holds of an operation: the ways to settle it, whatever it will carry. */
interface Settler {
    succeed(value: unknown): boolean;
    fail(code: OperationErrorCode, message: string): boolean;
}

/** The calls the shell has made and the compositor has not finished, by ticket. */
class Calls {
    #host: Host;
    #operations: OperationHost;
    #pending = new Map<number, Settler>();

    constructor(host: Host, operations: OperationHost) {
        this.#host = host;
        this.#operations = operations;
    }

    /** Starts `verb`. The returned operation settles when the compositor finishes it, or at once when it refuses. */
    begin<T>(verb: string, method: string): Operation<T> {
        let ticket = 0;
        const operation = new Operation<T>(this.#operations, this.#host.operationId(), method, () => {
            this.#pending.delete(ticket);
        });
        const answer = JSON.parse(this.#host.begin(verb)) as { ticket?: number; error?: Failure };
        if (answer.error !== undefined) {
            operation.fail(answer.error.code as OperationErrorCode, answer.error.message);
            return operation;
        }
        ticket = answer.ticket as number;
        this.#pending.set(ticket, operation as Settler);
        return operation;
    }

    finish(event: OperationEvent): void {
        const call = this.#pending.get(event.ticket);
        if (call === undefined) {
            // Another generation's call: the one that asked is gone, so nobody is waiting for the answer.
            return;
        }
        this.#pending.delete(event.ticket);
        if (event.ok) {
            call.succeed(event.value);
        } else {
            const failure = event.error as Failure;
            call.fail(failure.code as OperationErrorCode, failure.message);
        }
    }

    /** The connection is gone: nothing more arrives, so every call still waiting fails. */
    failAll(message: string): void {
        const waiting = [...this.#pending.values()];
        this.#pending.clear();
        for (const call of waiting) {
            call.fail("unavailable", message);
        }
    }

    clear(): void {
        this.#pending.clear();
    }
}

/** `gnoblin.session`: the session's state, and the verbs that end it, suspend it or lock it. */
export class Session {
    #host: Host;
    #calls: Calls;

    constructor(host: Host, calls: Calls) {
        this.#host = host;
        this.#calls = calls;
    }

    /** The session as the compositor last published it. Read on each call. */
    status(): SessionStatus {
        const text = this.#host.status();
        if (text === undefined) {
            return { state: "unavailable", lockAvailable: false, revision: 0 };
        }
        return { state: "running", ...(JSON.parse(text) as Omit<SessionStatus, "state">) };
    }

    /**
     * Ends the session. The compositor exits with the logout code, and the session wrapper returns to the login
     * manager.
     */
    logout(): Operation<Accepted> {
        return this.#calls.begin("logout", "session.logout");
    }

    /** Powers the machine off. Succeeds once the init system accepts; this call does not prompt. */
    poweroff(): Operation<Accepted> {
        return this.#calls.begin("poweroff", "session.poweroff");
    }

    reboot(): Operation<Accepted> {
        return this.#calls.begin("reboot", "session.reboot");
    }

    /**
     * Suspends to RAM. The session locks first. Fails `denied`, `unsupported`, `busy`, `unavailable` or `timed_out`.
     */
    suspend(): Operation<Accepted> {
        return this.#calls.begin("suspend", "session.suspend");
    }

    /** Hibernates to disk. Fails `unsupported` when the machine reports it cannot. */
    hibernate(): Operation<Accepted> {
        return this.#calls.begin("hibernate", "session.hibernate");
    }

    /** Asks the compositor to take the session lock. Read `status()` to see it enter a locked state. */
    lock(): Operation<LockRequest> {
        return this.#calls.begin("lock", "session.lock");
    }
}

/** `gnoblin.runtime`: what a script asks of the process that runs it. */
export class Runtime {
    #host: Host;
    #calls: Calls;

    constructor(host: Host, calls: Calls) {
        this.#host = host;
        this.#calls = calls;
    }

    /**
     * Asks for the reload `gnoblinctl config reload` asks for: the runtime evaluates the configuration again and
     * commits it. The operation succeeds once the compositor accepted the revision and fails `config_rejected` with
     * the reason when the document is refused, in which case the running configuration stays and this generation's
     * callbacks run. A successful reload replaces the generation that asked, so its callbacks are gone with it:
     * subscribe to nothing here, read `gnoblin.session` or the `config.reloaded` signal in the new one.
     *
     * Throws while the configuration loads, because every reload would ask for another.
     */
    reloadConfig(): Operation<ReloadResult> {
        if (this.#host.loading()) {
            throw new RangeError(
                "gnoblin.runtime.reloadConfig: the configuration is loading, and each reload would ask for another",
            );
        }
        return this.#calls.begin("reload-config", "runtime.reloadConfig");
    }
}

// ---- find and the focus history ----------------------------------------------------------------------------------

type Scalar = string | number | boolean;

/** The objects `gnoblin.find` can search, as the shell lists them. */
export interface FindSource {
    windows(): object[];
    workspaces(): object[];
    /** The live outputs, or null when this shell has none. */
    outputs(): object[] | null;
}

/** The scalar fields of each kind: what a table filter may name. */
const FIELDS: Record<string, readonly string[]> = {
    Window: [
        "id",
        "appId",
        "title",
        "alive",
        "focused",
        "fullscreen",
        "secure",
        "unresponsive",
        "floating",
        "maximised",
        "minimised",
        "tiled",
        "visible",
        "x",
        "y",
        "width",
        "height",
        "opacity",
        "z",
    ],
    Workspace: ["id", "name", "alive", "index", "active"],
};

/** The kinds `find` accepts. `Output` is listed only by a shell that has outputs. */
export function findKinds(source: FindSource): string[] {
    return source.outputs() === null ? ["Window", "Workspace"] : ["Window", "Workspace", "Output"];
}

/**
 * The live objects of one kind that match a filter, in list order. A table filter keeps the objects whose scalar
 * fields equal every value given (`appId` and `app_id` are one field); a function filter keeps those it returns a
 * truthy value for. An unknown kind or field, or a value that is not a string, number or boolean, throws and names
 * what works.
 */
export function find(source: FindSource, kind: string, filter?: Record<string, Scalar> | ((object: never) => unknown)) {
    const kinds = findKinds(source);
    if (!kinds.includes(kind)) {
        throw new TypeError(`gnoblin.find: no kind named ${JSON.stringify(kind)}; the kinds are ${kinds.join(", ")}`);
    }
    const objects =
        kind === "Window" ? source.windows() : kind === "Workspace" ? source.workspaces() : (source.outputs() ?? []);
    if (filter === undefined) {
        return [...objects];
    }
    if (typeof filter === "function") {
        return objects.filter((object) => filter(object as never));
    }
    if (typeof filter !== "object" || filter === null || Array.isArray(filter)) {
        throw new TypeError("gnoblin.find: the filter is a table of field values or a predicate function");
    }
    const wanted = Object.entries(filter).map(([key, value]) => {
        const field = camel(key);
        const fields = FIELDS[kind] ?? scalarFields(objects[0]);
        if (!fields.includes(field)) {
            throw new TypeError(
                `gnoblin.find: ${kind} has no scalar field ${JSON.stringify(key)}; the fields are ${fields.join(", ")}`,
            );
        }
        if (typeof value !== "string" && typeof value !== "number" && typeof value !== "boolean") {
            throw new TypeError(`gnoblin.find: the value of ${key} must be a string, a number or a boolean`);
        }
        return [field, value] as const;
    });
    return objects.filter((object) => wanted.every(([field, value]) => Reflect.get(object, field) === value));
}

/** The names of an object's scalar properties, for a kind that declares none (the outputs of another module). */
function scalarFields(sample: object | undefined): string[] {
    if (sample === undefined) {
        return [];
    }
    const names = new Set<string>();
    for (let prototype: object | null = sample; prototype !== null && prototype !== Object.prototype;) {
        for (const name of Object.getOwnPropertyNames(prototype)) {
            const value = Object.getOwnPropertyDescriptor(prototype, name)?.get ? Reflect.get(sample, name) : undefined;
            if (["string", "number", "boolean"].includes(typeof value)) {
                names.add(name);
            }
        }
        prototype = Object.getPrototypeOf(prototype) as object | null;
    }
    return [...names];
}

/** A window as the focus history reads it. */
export interface HistoryClient {
    readonly id: string;
    readonly alive: boolean;
    readonly workspace: { readonly id: string } | null;
}

/** What `gnoblin.focus.history` takes. The snake_case spellings are accepted too. */
export interface FocusHistoryFilter {
    /** Only the windows on this workspace. */
    workspaceId?: string;
    /** Only the windows on this output; a window with no `output` property matches none. */
    monitorId?: string;
    /** The most windows to return, 1 to 256; 50 by default. */
    limit?: number;
}

const HISTORY_LIMIT = 256;
const HISTORY_DEFAULT = 50;

/** `gnoblin.focus`: the windows in the order they were last focused. */
export class FocusHistory {
    #windows: () => HistoryClient[];
    #focused: () => HistoryClient | null;
    #order: HistoryClient[] = [];

    constructor(windows: () => HistoryClient[], focused: () => HistoryClient | null) {
        this.#windows = windows;
        this.#focused = focused;
    }

    /** Notes who has the focus now. The shell calls it once a frame. */
    observe(): void {
        const focused = this.#focused();
        this.#order = this.#order.filter((client) => client.alive && client !== focused);
        if (focused !== null && focused.alive) {
            this.#order.unshift(focused);
        }
    }

    /**
     * The live windows, most recently focused first, then those whose focus was never seen in the order the shell
     * lists them. Minimised windows are included and a window that closed is gone. At startup only the focused window
     * seeds the order.
     */
    history(filter?: FocusHistoryFilter | Record<string, unknown>): HistoryClient[] {
        const given = filter ?? {};
        if (typeof given !== "object" || given === null) {
            throw new TypeError("gnoblin.focus.history: the filter is an object");
        }
        const unknown = Object.keys(given).find(
            (key) => !["workspaceId", "workspace_id", "monitorId", "monitor_id", "limit"].includes(key),
        );
        if (unknown !== undefined) {
            throw new TypeError(
                `gnoblin.focus.history: no filter named ${unknown}; the filters are workspaceId, monitorId, limit`,
            );
        }
        const take = (camelName: string, snake: string): unknown =>
            Reflect.get(given, camelName) ?? Reflect.get(given, snake);
        const limit = take("limit", "limit") ?? HISTORY_DEFAULT;
        if (typeof limit !== "number" || !Number.isInteger(limit) || limit < 1 || limit > HISTORY_LIMIT) {
            throw new TypeError(`gnoblin.focus.history: limit is an integer from 1 to ${HISTORY_LIMIT}`);
        }
        const workspace = take("workspaceId", "workspace_id");
        const monitor = take("monitorId", "monitor_id");
        const live = this.#windows();
        const seen = this.#order.filter((client) => live.includes(client));
        const ordered = [...seen, ...live.filter((client) => !seen.includes(client))];
        return ordered
            .filter((client) => workspace === undefined || client.workspace?.id === workspace)
            .filter(
                (client) =>
                    monitor === undefined ||
                    (Reflect.get(client, "output") as { id?: string } | undefined)?.id === monitor,
            )
            .slice(0, limit);
    }
}

// ---- everything together -----------------------------------------------------------------------------------------

/** What the shell gives the module that is not a host function. */
export interface ApiSources extends FindSource {
    /** The window that holds the keyboard focus, or null. */
    focusedWindow(): HistoryClient | null;
}

/**
 * The host API of one generation: its timers, processes, bus and session calls. A generation's state is its own, so
 * a reload starts these afresh and `retire` ends what the old one had pending.
 */
export class HostApi {
    readonly timers: Timers;
    readonly process: ProcessTable;
    readonly session: Session;
    readonly runtime: Runtime;
    readonly bus: EventBus;
    readonly focus: FocusHistory;
    #host: Host;
    #sources: ApiSources;
    #calls: Calls;
    /** Told of each timed write the compositor says has ended; `gnoblin.requestTransition` is the one that listens. */
    onTrackEnded: (event: TrackEndedEvent) => void = () => {};

    constructor(host: Host, sources: ApiSources) {
        this.#host = host;
        this.#sources = sources;
        this.timers = new Timers(host);
        const operations: OperationHost = {
            schedule: (site, task) => host.schedule(site, task),
            now: () => host.now(),
            after: (ms, fn) => this.timers.after(ms, fn),
        };
        this.process = new ProcessTable({
            ...operations,
            spawn: (argv, options) => host.spawn(argv, options),
            kill: (id, signal) => host.kill(id, signal),
            operationId: () => host.operationId(),
            error: (code, target, message) => host.error(code, target, message),
        });
        this.#calls = new Calls(host, operations);
        this.session = new Session(host, this.#calls);
        this.runtime = new Runtime(host, this.#calls);
        this.bus = new EventBus(host);
        this.focus = new FocusHistory(
            () => sources.windows() as HistoryClient[],
            () => sources.focusedWindow(),
        );
    }

    /** The build identity, read on each call. */
    version(): Version {
        return Object.freeze(JSON.parse(this.#host.version()) as Version);
    }

    find(kind: string, filter?: Record<string, Scalar> | ((object: never) => unknown)): object[] {
        return find(this.#sources, kind, filter);
    }

    /** One frame: what the host collected, then the timers that are due. */
    frame(batch: string): void {
        for (const event of JSON.parse(batch) as HostEvent[]) {
            switch (event.kind) {
                case "op":
                    this.#calls.finish(event);
                    break;
                case "signal":
                    this.bus.deliver(event);
                    break;
                case "lost":
                    this.#calls.failAll(event.message);
                    break;
                case "exit":
                    this.process.deliver(event);
                    break;
                case "trackEnded":
                    this.onTrackEnded(event);
                    break;
            }
        }
        this.focus.observe();
        this.timers.fire();
    }

    /** Ends what the generation had pending: its timers, its handlers and the calls it was waiting on. */
    retire(): void {
        this.timers.cancelAll();
        this.bus.clear();
        this.#calls.clear();
    }
}
