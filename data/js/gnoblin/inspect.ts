// Read-only inspection helpers: this module owns plain snapshots and value rendering, not shell state.
// The host registers its native object types; scripts can use the same helpers without importing the host.
// Rendering limits and truncation are part of the API; see the types below.
// Read this module first for plain-data and console-rendering behavior.

/** What a registered kind reports of one instance. */
export interface InspectView {
    /** The kind's name, as it prints: `Client`, `Workspace`. */
    readonly name: string;
    /** The instance's id; it prints as `Name#id` and is the first member of the data form. */
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
    /** `undefined`, `null`, `boolean`, `number`, `string`, `array`, `function`, `object`, or a kind's or class's name. */
    readonly type: string;
    /** The value as plain JSON data; absent for `undefined`. */
    readonly value?: unknown;
    /** The value as a person reads it in a terminal. */
    readonly text: string;
    /** Whether the size cap cut something off. */
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
    readonly clients: readonly string[];
}

/** One binding registered on an input context, and where it ranks among the ones with the same trigger. */
export interface BindingInfo {
    /** The seat's context. */
    readonly context: string;
    readonly id: string;
    /** The trigger as written. */
    readonly trigger: string;
    /** `key`, `pointer`, `wheel` or `gesture`. */
    readonly kind: string;
    /** Who bound it: the context itself, a layout attached to it, or one device of it. */
    readonly owner: "context" | "layout" | "device";
    /** The layout's class name for a layout's binding. */
    readonly layout: string | null;
    /** The device's name for a device's binding. */
    readonly device: string | null;
    readonly priority: number;
    readonly enabled: boolean;
    /** Who runs first for this trigger: 1 is the binding a press reaches first, 2 the one it falls to when 1 declines. */
    readonly rank: number;
}

const DEFAULT_MAX_BYTES = 65536;
const DEFAULT_MAX_DEPTH = 6;
/** A string longer than this is cut: a console is not a file viewer. */
const MAX_STRING = 4096;

type Describer = (value: never) => InspectView;

/** The registered kinds, in the order they are tried. */
const kinds: { ctor: abstract new (...args: never[]) => unknown; describe: Describer }[] = [];

/** Teaches the renderer what an instance of `ctor` shows. A later registration for the same class replaces it. */
function register<T>(ctor: abstract new (...args: never[]) => T, describe: (value: T) => InspectView): void {
    const at = kinds.findIndex((kind) => kind.ctor === ctor);
    const entry = { ctor, describe: describe as Describer };
    if (at >= 0) {
        kinds[at] = entry;
    } else {
        kinds.push(entry);
    }
}

interface Walk {
    left: number;
    truncated: boolean;
    readonly maxDepth: number;
    readonly ancestors: Set<object>;
}

interface Shown {
    data: unknown;
    text: string;
}

function spend(state: Walk, text: string): void {
    state.left -= text.length;
}

function leaf(state: Walk, data: unknown, text: string): Shown {
    spend(state, text);
    return { data, text };
}

function cut(state: Walk): Shown {
    state.truncated = true;
    return { data: "[truncated]", text: "[truncated]" };
}

function nameOf(value: object): string {
    const ctor: unknown = Reflect.get(value, "constructor");
    const name = typeof ctor === "function" ? ctor.name : "";
    return name === "" ? "Object" : name;
}

function failure(error: unknown): string {
    return `[Error: ${error instanceof Error ? error.message : String(error)}]`;
}

function describeFunction(value: Function): string {
    const source = Function.prototype.toString.call(value);
    const name = value.name === "" ? "(anonymous)" : value.name;
    return source.startsWith("class") ? `[class ${name}]` : `[Function ${name}]`;
}

function walk(value: unknown, state: Walk, depth: number): Shown {
    if (state.left <= 0) {
        return cut(state);
    }
    switch (typeof value) {
        case "undefined":
            return leaf(state, null, "undefined");
        case "boolean":
            return leaf(state, value, String(value));
        case "number": {
            // JSON has no NaN, Infinity or negative zero: they travel as their names.
            const text = Object.is(value, -0) ? "-0" : String(value);
            return leaf(state, Number.isFinite(value) && text !== "-0" ? value : text, text);
        }
        case "bigint":
            return leaf(state, `${value}n`, `${value}n`);
        case "symbol":
            return leaf(state, value.toString(), value.toString());
        case "string": {
            let text = value;
            if (text.length > MAX_STRING) {
                text = text.slice(0, MAX_STRING);
                state.truncated = true;
            }
            return leaf(state, text, JSON.stringify(text));
        }
        case "function": {
            const text = describeFunction(value);
            return leaf(state, text, text);
        }
        default:
            break;
    }
    if (value === null) {
        return leaf(state, null, "null");
    }
    if (typeof value === "object") {
        return walkObject(value, state, depth);
    }
    return leaf(state, String(value), String(value));
}

function walkObject(value: object, state: Walk, depth: number): Shown {
    if (state.ancestors.has(value)) {
        return leaf(state, "[Circular]", "[Circular]");
    }
    const kind = kinds.find((entry) => value instanceof entry.ctor);
    if (kind === undefined && depth > state.maxDepth) {
        const text = Array.isArray(value) ? "[Array]" : `[${nameOf(value)}]`;
        return leaf(state, text, text);
    }
    state.ancestors.add(value);
    try {
        if (kind !== undefined) {
            let view: InspectView;
            try {
                view = kind.describe(value as never);
            } catch (error) {
                const text = failure(error);
                return leaf(state, text, text);
            }
            return walkView(view, state, depth);
        }
        if (Array.isArray(value)) {
            return walkList(value, state, depth, "");
        }
        return walkPlain(value, state, depth);
    } finally {
        state.ancestors.delete(value);
    }
}

/** The members of a list, each shown as a brief: only the value itself shows everything of its kind. */
function walkList(items: readonly unknown[], state: Walk, depth: number, label: string): Shown {
    const data: unknown[] = [];
    const parts: string[] = [];
    spend(state, "[ ]");
    for (const [index, item] of items.entries()) {
        if (state.left <= 0) {
            state.truncated = true;
            const rest = `... ${items.length - index} more`;
            data.push(rest);
            parts.push(rest);
            break;
        }
        const shown = walk(item, state, depth + 1);
        // The separator counts too, or a list of short numbers would cost a third of what it prints.
        spend(state, ", ");
        data.push(shown.data);
        parts.push(shown.text);
    }
    return { data, text: parts.length === 0 ? `${label}[]` : `${label}[ ${parts.join(", ")} ]` };
}

function entriesOf(
    read: () => [string, () => unknown][],
    state: Walk,
    depth: number,
): { data: Record<string, unknown>; parts: string[] } {
    const data: Record<string, unknown> = {};
    const parts: string[] = [];
    const list = read();
    for (const [index, [key, get]] of list.entries()) {
        if (state.left <= 0) {
            state.truncated = true;
            const rest = `... ${list.length - index} more`;
            data["..."] = rest;
            parts.push(rest);
            break;
        }
        let shown: Shown;
        try {
            shown = walk(get(), state, depth + 1);
        } catch (error) {
            const text = failure(error);
            shown = leaf(state, text, text);
        }
        spend(state, `${key}: , `);
        data[key] = shown.data;
        parts.push(`${/^[A-Za-z_$][\w$]*$/.test(key) ? key : JSON.stringify(key)}: ${shown.text}`);
    }
    return { data, parts };
}

function walkView(view: InspectView, state: Walk, depth: number): Shown {
    if (view.items !== undefined) {
        return walkList(view.items, state, depth, `${view.name}(${view.items.length}) `);
    }
    // The value itself shows everything of its kind; anything nested inside another value shows the brief.
    const fields = depth === 0 ? view.fields : (view.brief ?? view.fields);
    const { data, parts } = entriesOf(() => Object.keys(fields).map((key) => [key, () => fields[key]]), state, depth);
    const head = view.id === undefined ? view.name : `${view.name}#${view.id}`;
    spend(state, head);
    return {
        data: { $class: view.name, ...(view.id === undefined ? {} : { id: view.id }), ...data },
        text: parts.length === 0 ? `${head} {}` : `${head} { ${parts.join(", ")} }`,
    };
}

function walkPlain(value: object, state: Walk, depth: number): Shown {
    if (value instanceof Error) {
        return leaf(state, { $class: value.name, message: value.message }, `${value.name}: ${value.message}`);
    }
    if (value instanceof Date) {
        const text = Number.isNaN(value.getTime()) ? "Invalid Date" : value.toISOString();
        return leaf(state, text, text);
    }
    if (value instanceof RegExp) {
        return leaf(state, String(value), String(value));
    }
    if (value instanceof Promise) {
        return leaf(state, "[Promise]", "[Promise]");
    }
    if (value instanceof Map) {
        const shown = walkList([...value.entries()], state, depth, "");
        return { data: { $class: "Map", entries: shown.data }, text: `Map(${value.size}) ${shown.text}` };
    }
    if (value instanceof Set) {
        const shown = walkList([...value.values()], state, depth, "");
        return { data: { $class: "Set", values: shown.data }, text: `Set(${value.size}) ${shown.text}` };
    }
    if (ArrayBuffer.isView(value) || value instanceof ArrayBuffer) {
        const length = "byteLength" in value ? value.byteLength : 0;
        const text = `[${nameOf(value)} ${length}]`;
        return leaf(state, text, text);
    }
    if (value instanceof WeakMap || value instanceof WeakSet) {
        const text = `[${nameOf(value)}]`;
        return leaf(state, text, text);
    }
    const { data, parts } = entriesOf(
        () => Object.keys(value).map((key) => [key, () => (value as Record<string, unknown>)[key]]),
        state,
        depth,
    );
    const name = nameOf(value);
    const bare = Object.getPrototypeOf(value) === null || name === "Object";
    spend(state, name);
    return {
        data: bare ? data : { $class: name, ...data },
        text: bare
            ? parts.length === 0
                ? "{}"
                : `{ ${parts.join(", ")} }`
            : parts.length === 0
              ? `${name} {}`
              : `${name} { ${parts.join(", ")} }`,
    };
}

function typeOf(value: unknown): string {
    if (value === null) {
        return "null";
    }
    if (Array.isArray(value)) {
        return "array";
    }
    if (typeof value === "object") {
        const kind = kinds.find((entry) => value instanceof entry.ctor);
        if (kind !== undefined) {
            try {
                return kind.describe(value as never).name;
            } catch {
                return nameOf(value);
            }
        }
        const name = nameOf(value);
        return name === "Object" ? "object" : name;
    }
    return typeof value;
}

/** The value as plain JSON-able data, for `config()` and for tests: the data half of `render`. */
function plain(value: unknown, options: RenderOptions = {}): unknown {
    const state: Walk = {
        left: options.maxBytes ?? DEFAULT_MAX_BYTES,
        truncated: false,
        maxDepth: options.maxDepth ?? DEFAULT_MAX_DEPTH,
        ancestors: new Set(),
    };
    const data = walk(value, state, 0).data;
    return value === undefined ? undefined : data;
}

/**
 * Renders `value` for the console and returns the result as JSON text: `{ type, value?, text, truncated }`. A cycle
 * prints as `[Circular]`, a function as `[Function name]`, a native-backed object by its kind (`Client#3 { ... }`), a
 * collection as an array of summaries; what does not fit in `maxBytes` is left out and `truncated` says so.
 */
function render(value: unknown, options: RenderOptions = {}): string {
    const state: Walk = {
        left: options.maxBytes ?? DEFAULT_MAX_BYTES,
        truncated: false,
        maxDepth: options.maxDepth ?? DEFAULT_MAX_DEPTH,
        ancestors: new Set(),
    };
    const shown = walk(value, state, 0);
    const result: Rendered = {
        type: typeOf(value),
        ...(value === undefined ? {} : { value: shown.data }),
        text: shown.text,
        truncated: state.truncated,
    };
    return JSON.stringify(result);
}

// -- the helpers a console user types ---------------------------------------------------------------------------

interface LiveClient {
    readonly id: string;
    readonly appId: string;
    readonly title: string;
    readonly fullscreen: boolean;
    readonly floating: boolean;
    readonly x: number;
    readonly y: number;
    readonly width: number;
    readonly height: number;
    readonly workspace: { readonly id: string } | null;
}

interface LiveWorkspace {
    readonly id: string;
    readonly name: string;
    readonly index: number;
    readonly active: boolean;
    readonly layout: object;
    readonly clients: Iterable<{ readonly id: string }>;
}

interface LiveContext {
    readonly name: string;
    describeBindings(): BindingInfo[];
}

interface LiveGnoblin {
    readonly clients: Iterable<LiveClient> & { readonly focused: LiveClient | null };
    readonly workspaces: Iterable<LiveWorkspace>;
    readonly input: { readonly contexts: readonly LiveContext[] };
    /** The merged config the last `defineConfig` registered, or null for a shell that has none. */
    __effectiveConfig?(): unknown;
}

function live(): LiveGnoblin {
    // The host defines the global after this module has loaded, so it is read when a helper runs.
    const found: LiveGnoblin | undefined = Reflect.get(globalThis, "gnoblin");
    if (found === undefined) {
        throw new Error("inspect: there is no `gnoblin` global in this runtime");
    }
    return found;
}

/** Every client as plain data. */
function clients(): ClientInfo[] {
    const gnoblin = live();
    const focused = gnoblin.clients.focused;
    return [...gnoblin.clients].map((client) => ({
        id: client.id,
        appId: client.appId,
        title: client.title,
        focused: client === focused,
        fullscreen: client.fullscreen,
        floating: client.floating,
        x: client.x,
        y: client.y,
        width: client.width,
        height: client.height,
        workspace: client.workspace?.id ?? null,
    }));
}

/** Every workspace as plain data, with the ids of the clients on it. */
function workspaces(): WorkspaceInfo[] {
    return [...live().workspaces].map((workspace) => ({
        id: workspace.id,
        name: workspace.name,
        index: workspace.index,
        active: workspace.active,
        layout: nameOf(workspace.layout),
        clients: [...workspace.clients].map((client) => client.id),
    }));
}

/**
 * The bindings registered on every input context: the trigger, who owns it (the context, a layout or a device),
 * its priority and its `rank` among the bindings that match the same trigger. A shell with no seat has none.
 */
function bindings(): BindingInfo[] {
    return live()
        .input.contexts.flatMap((context) => context.describeBindings())
        .sort((a, b) => a.context.localeCompare(b.context) || a.trigger.localeCompare(b.trigger) || a.rank - b.rank);
}

/**
 * The effective config as plain data: the user's config laid over the stock one, as the shell merged it. A function
 * (`setup`, a key's handler) shows as `[Function name]`. `null` when the entry never called `defineConfig`.
 */
function config(): unknown {
    const merged = live().__effectiveConfig?.();
    return merged === undefined || merged === null ? null : plain(merged, { maxBytes: 262144, maxDepth: 10 });
}

/** The shell inspection API; it returns snapshots rather than live native objects. */
export const inspect = { clients, workspaces, bindings, config, render, plain, register } as const;
