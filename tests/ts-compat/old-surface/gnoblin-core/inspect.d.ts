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
    /** `undefined`, `null`, `boolean`, `number`, `string`, `array`, `function`, `object`, or a kind or class name. */
    readonly type: string;
    /** The value as plain JSON data; absent for `undefined`. */
    readonly value?: unknown;
    /** The value as a person reads it in a terminal. */
    readonly text: string;
    /** Whether the size cap cut something off. */
    readonly truncated: boolean;
}
/** Limits for a rendered value. */
export interface RenderOptions {
    /** The most text the rendering may hold before it stops adding to it. Default 65536. */
    readonly maxBytes?: number;
    /** How deep objects and arrays are walked. Default 6. */
    readonly maxDepth?: number;
}
/** A client as plain data. */
export interface ClientInfo {
    /** Client identifier. */
    readonly id: string;
    /** Application id, or an empty string when unavailable. */
    readonly appId: string;
    /** Window title, or an empty string when unavailable. */
    readonly title: string;
    /** Whether this client has keyboard focus. */
    readonly focused: boolean;
    /** Whether this client is fullscreen. */
    readonly fullscreen: boolean;
    /** Whether this client is floating rather than tiled. */
    readonly floating: boolean;
    /** Horizontal position in logical pixels. */
    readonly x: number;
    /** Vertical position in logical pixels. */
    readonly y: number;
    /** Width in logical pixels. */
    readonly width: number;
    /** Height in logical pixels. */
    readonly height: number;
    /** The id of the workspace the client is on, or null. */
    readonly workspace: string | null;
}
/** A workspace as plain data. */
export interface WorkspaceInfo {
    /** Workspace identifier. */
    readonly id: string;
    /** Workspace name. */
    readonly name: string;
    /** Position in the workspace collection. */
    readonly index: number;
    /** Whether this workspace is active. */
    readonly active: boolean;
    /** The layout's class name. */
    readonly layout: string;
    /** Client identifiers in this workspace. */
    readonly clients: readonly string[];
}
/** One binding registered on an input context, and where it ranks among the ones with the same trigger. */
export interface BindingInfo {
    /** The seat's context. */
    readonly context: string;
    /** Binding identifier. */
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
    /** Binding priority among bindings with the same trigger. */
    readonly priority: number;
    /** Whether the binding is enabled. */
    readonly enabled: boolean;
    /** Who runs first for this trigger: 1 is the binding a press reaches first, 2 the one it falls to if 1 declines. */
    readonly rank: number;
}
/** Teaches the renderer what an instance of `ctor` shows. A later registration for the same class replaces it. */
declare function register<T>(ctor: abstract new (...args: never[]) => T, describe: (value: T) => InspectView): void;
/** The value as plain JSON-able data, for tests and the console: the data half of `render`. */
declare function plain(value: unknown, options?: RenderOptions): unknown;
/**
 * Renders `value` for the console and returns the result as JSON text: `{ type, value?, text, truncated }`. A cycle
 * prints as `[Circular]`, a function as `[Function name]`, a native-backed object by its kind (`Client#3 { ... }`), a
 * collection as an array of summaries; what does not fit in `maxBytes` is left out and `truncated` says so.
 */
declare function render(value: unknown, options?: RenderOptions): string;
/** Every client as plain data. */
declare function clients(): ClientInfo[];
/** Every workspace as plain data, with the ids of the clients on it. */
declare function workspaces(): WorkspaceInfo[];
/**
 * The bindings registered on every input context: the trigger, who owns it (the context, a layout or a device),
 * its priority and its `rank` among the bindings that match the same trigger. A shell with no seat has none.
 */
declare function bindings(): BindingInfo[];
/** The shell inspection API; it returns snapshots rather than live native objects. */
export declare const inspect: {
    readonly clients: typeof clients;
    readonly workspaces: typeof workspaces;
    readonly bindings: typeof bindings;
    readonly render: typeof render;
    readonly plain: typeof plain;
    readonly register: typeof register;
};
export {};
