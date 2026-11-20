/**
 * The most launches the registry keeps. The oldest finished one is dropped for a new one. With all pending, `begin`
 * refuses.
 */
export declare const MAX_LAUNCHES = 64;
/** The deadline of a launch that gave none, in milliseconds. */
export declare const DEFAULT_LAUNCH_TIMEOUT_MS = 3000;
/** The shortest deadline a launch may have, in milliseconds. */
export declare const MIN_LAUNCH_TIMEOUT_MS = 100;
/** The longest deadline a launch may have, in milliseconds. */
export declare const MAX_LAUNCH_TIMEOUT_MS = 10000;
/** Where a launch is. It begins `pending` and leaves it once, for one of the other three. */
export type LaunchState = "pending" | "started" | "timed_out" | "ended";
/**
 * Observes one launch: pass it to `gnoblin.launches.begin` as `listener`. Every method is optional, and each receives
 * the launch first.
 */
export declare class LaunchListener {
    /** A field changed; `fields` names them. */
    onChanged?(launch: Launch, fields: readonly string[]): void;
    /** A window of the application appeared or took focus. */
    onStarted?(launch: Launch): void;
    /** The deadline passed first. */
    onTimedOut?(launch: Launch): void;
    /**
     * The launch left `pending`, for any reason; it runs after the other methods. `reason` is the new state,
     * `replaced` or a refusal.
     */
    onClosed?(launch: Launch, reason: string): void;
}
/** What `gnoblin.launches.begin` takes beside the token and the application. */
export interface LaunchOptions {
    /** The deadline of the launch in milliseconds, 100 to 10000. Default 3000. */
    timeoutMs?: number;
    /** Told of the launch's changes. */
    listener?: LaunchListener;
}
/** What the registry needs of the shell: a clock and the call queue. */
export interface LaunchHost {
    /** A monotonic clock in milliseconds. */
    now(): number;
    /** Queues a handler call. `site` identifies it in reports. */
    schedule(site: string, task: () => void): void;
}
/** The form two application IDs are compared in: lowercase, no `.desktop`. */
export declare function normalizeApplication(name: string): string;
/** One launch. It is the same object for as long as it is in the registry. */
export declare class Launch {
    #private;
    /** The caller's unique key for matching this launch. */
    readonly token: string;
    /** Application identifier used to match a window, such as `org.example.App.desktop`. */
    readonly application: string;
    /** The deadline asked for, in milliseconds. */
    readonly timeoutMs: number;
    /** Unix time in milliseconds when `begin` was called. */
    readonly startedAt: number;
    /** Made by `gnoblin.launches.begin`; a script has no use for the constructor. */
    constructor(registry: LaunchFeedback, token: string, application: string, timeoutMs: number, deadline: number, handlers: LaunchListener);
    /** The current state, from `pending` to one terminal state. */
    get state(): LaunchState;
    /** True while the launch is `pending`. */
    get alive(): boolean;
    /** Counts the changes of the launch; 0 until the first. */
    get revision(): number;
    /** Ends a pending launch: the program was cancelled or has started. A launch that is not pending is unchanged. */
    finish(): void;
    /** The registry's side: leaves `pending` for `state` and tells the handlers. Not for scripts. */
    static settle(launch: Launch, host: LaunchHost, state: Exclude<LaunchState, "pending">, reason: string): boolean;
    /** Returns the launch's deadline in monotonic milliseconds. */
    static deadline(launch: Launch): number;
}
/** `gnoblin.launches`: the launches the shell tracks, and the windows that answer them. */
export declare class LaunchFeedback {
    #private;
    constructor(host: LaunchHost);
    /** The launches the registry keeps, oldest first, pending and finished. */
    list(): Launch[];
    /**
     * Tracks a launch of `application` under `token`. It is returned `pending`, with revision 0. A token that is
     * already in use ends the earlier launch (`onClosed` with the reason `replaced`). A launch the registry cannot
     * take, because it holds 64 pending ones, comes back `ended` with that reason in its `onClosed`.
     */
    begin(token: string, application: string, options?: LaunchOptions): Launch;
    /** Ends a pending launch as `ended`. */
    leave(launch: Launch, state: "ended"): void;
    /** A window of `appId` appeared or took focus: it starts the oldest pending launch that matches. */
    answered(appId: string): void;
    /** Whether a pending launch still waits for a window or its deadline. */
    get pending(): boolean;
    /** Times out the launches whose deadline has passed. The shell calls this once a frame. */
    frame(): void;
}
