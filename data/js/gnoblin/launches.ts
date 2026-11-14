// `gnoblin.launches`: launch feedback, the "starting" state between asking for an application and its window.
// A shell calls `begin(token, application)` when it starts a program and shows a spinner while the launch is `pending`.
// The launch is `started` when a window of that application appears or takes focus, `timed_out` when the deadline
// passes first, and `ended` when the shell calls `finish()`. The compositor keeps no launch records: these are the
// shell's own bookkeeping over the windows it sees, and a window is not linked to the launch it answered.
// Matching compares the window's `appId` and the launch's `application` lowercased, with a trailing `.desktop`
// removed. A window starts only the oldest pending launch it matches. The registry is plain TypeScript over three
// functions of the host (`LaunchHost`), so it runs without a compositor.

/**
 * The most launches the registry keeps. The oldest finished one is dropped for a new one. With all pending, `begin`
 * refuses.
 */
export const MAX_LAUNCHES = 64;

/** The deadline of a launch that gave none, in milliseconds. */
export const DEFAULT_LAUNCH_TIMEOUT_MS = 3000;

/** The shortest deadline a launch may have, in milliseconds. */
export const MIN_LAUNCH_TIMEOUT_MS = 100;
/** The longest deadline a launch may have, in milliseconds. */
export const MAX_LAUNCH_TIMEOUT_MS = 10_000;

/** Where a launch is. It begins `pending` and leaves it once, for one of the other three. */
export type LaunchState = "pending" | "started" | "timed_out" | "ended";

/** What a launch tells its owner. Every member is optional. */
export interface LaunchHandlers {
    /** The deadline of the launch in milliseconds, 100 to 10000. Default 3000. */
    timeoutMs?: number;
    /** A field changed; `fields` names them. */
    onChanged?(launch: Launch, fields: readonly string[]): void;
    /** A window of the application appeared or took focus. */
    onStarted?(launch: Launch): void;
    /** The deadline passed first. */
    onTimedOut?(launch: Launch): void;
    /**
     * The launch left `pending`, for any reason; it runs after the other handlers. `reason` is the new state,
     * `replaced` or a refusal.
     */
    onClosed?(launch: Launch, reason: string): void;
}

/** What the registry needs of the shell: a clock and the call queue. */
export interface LaunchHost {
    /** A monotonic clock in milliseconds. */
    now(): number;
    schedule(site: string, task: () => void): void;
}

const NAME_MAX = 128;
const APPLICATION_MAX = 512;

/** The form two application IDs are compared in: lowercase, no `.desktop`. */
export function normalizeApplication(name: string): string {
    const lower = name.toLowerCase();
    return lower.endsWith(".desktop") ? lower.slice(0, -".desktop".length) : lower;
}

/** One launch. It is the same object for as long as it is in the registry. */
export class Launch {
    readonly token: string;
    readonly application: string;
    /** The deadline asked for, in milliseconds. */
    readonly timeoutMs: number;
    /** Unix time in milliseconds when `begin` was called. */
    readonly startedAt: number;
    #state: LaunchState = "pending";
    #revision = 0;
    readonly #deadline: number;
    readonly #handlers: LaunchHandlers;
    readonly #registry: LaunchFeedback;

    /** Made by `gnoblin.launches.begin`; a script has no use for the constructor. */
    constructor(
        registry: LaunchFeedback,
        token: string,
        application: string,
        timeoutMs: number,
        deadline: number,
        handlers: LaunchHandlers,
    ) {
        this.#registry = registry;
        this.token = token;
        this.application = application;
        this.timeoutMs = timeoutMs;
        this.startedAt = Date.now();
        this.#deadline = deadline;
        this.#handlers = handlers;
    }

    get state(): LaunchState {
        return this.#state;
    }

    /** True while the launch is `pending`. */
    get alive(): boolean {
        return this.#state === "pending";
    }

    /** Counts the changes of the launch; 0 until the first. */
    get revision(): number {
        return this.#revision;
    }

    /** Ends a pending launch: the program was cancelled or has started. A launch that is not pending is unchanged. */
    finish(): void {
        this.#registry.leave(this, "ended");
    }

    /** The registry's side: leaves `pending` for `state` and tells the handlers. Not for scripts. */
    static settle(launch: Launch, host: LaunchHost, state: Exclude<LaunchState, "pending">, reason: string): boolean {
        if (launch.#state !== "pending") {
            return false;
        }
        launch.#state = state;
        launch.#revision += 1;
        const handlers = launch.#handlers;
        const fields = ["state", "alive", "revision"];
        const queue = (site: string, call: (() => void) | undefined): void => {
            if (call !== undefined) {
                host.schedule(`Launch.${site}`, call);
            }
        };
        if (handlers.onChanged !== undefined) {
            const changed = handlers.onChanged;
            queue("onChanged", () => changed.call(handlers, launch, fields));
        }
        if (state === "started" && handlers.onStarted !== undefined) {
            const started = handlers.onStarted;
            queue("onStarted", () => started.call(handlers, launch));
        }
        if (state === "timed_out" && handlers.onTimedOut !== undefined) {
            const timedOut = handlers.onTimedOut;
            queue("onTimedOut", () => timedOut.call(handlers, launch));
        }
        if (handlers.onClosed !== undefined) {
            const closed = handlers.onClosed;
            queue("onClosed", () => closed.call(handlers, launch, reason));
        }
        return true;
    }

    static deadline(launch: Launch): number {
        return launch.#deadline;
    }
}

function validate(token: unknown, application: unknown, options: unknown): LaunchHandlers {
    if (typeof token !== "string" || token.length === 0 || token.length > NAME_MAX) {
        throw new TypeError(
            `gnoblin.launches.begin: the token must be a non-empty string of at most ${NAME_MAX} characters`,
        );
    }
    if (typeof application !== "string" || application.length === 0 || application.length > APPLICATION_MAX) {
        throw new TypeError(
            `gnoblin.launches.begin: the application must be a non-empty string ` +
                `of at most ${APPLICATION_MAX} characters`,
        );
    }
    if (options === undefined) {
        return {};
    }
    if (options === null || typeof options !== "object") {
        throw new TypeError("gnoblin.launches.begin: the options must be an object");
    }
    const given = options as Record<string, unknown>;
    const timeout = given.timeoutMs;
    if (
        timeout !== undefined &&
        (typeof timeout !== "number" ||
            !Number.isInteger(timeout) ||
            timeout < MIN_LAUNCH_TIMEOUT_MS ||
            timeout > MAX_LAUNCH_TIMEOUT_MS)
    ) {
        throw new RangeError(
            `gnoblin.launches.begin: timeoutMs must be an integer ` +
                `from ${MIN_LAUNCH_TIMEOUT_MS} to ${MAX_LAUNCH_TIMEOUT_MS}`,
        );
    }
    for (const name of ["onChanged", "onStarted", "onTimedOut", "onClosed"]) {
        if (given[name] !== undefined && typeof given[name] !== "function") {
            throw new TypeError(`gnoblin.launches.begin: ${name} must be a function`);
        }
    }
    return given as LaunchHandlers;
}

/** `gnoblin.launches`: the launches the shell tracks, and the windows that answer them. */
export class LaunchFeedback {
    readonly #host: LaunchHost;
    #launches: Launch[] = [];

    constructor(host: LaunchHost) {
        this.#host = host;
    }

    /** The launches the registry keeps, oldest first, pending and finished. */
    list(): Launch[] {
        return [...this.#launches];
    }

    /**
     * Tracks a launch of `application` under `token`. It is returned `pending`, with revision 0. A token that is
     * already in use ends the earlier launch (`onClosed` with the reason `replaced`). A launch the registry cannot
     * take, because it holds 64 pending ones, comes back `ended` with that reason in its `onClosed`.
     */
    begin(token: string, application: string, options?: LaunchHandlers): Launch {
        const handlers = validate(token, application, options);
        const earlier = this.#launches.find((launch) => launch.token === token);
        if (earlier !== undefined) {
            Launch.settle(earlier, this.#host, "ended", "replaced");
            this.#launches.splice(this.#launches.indexOf(earlier), 1);
        }
        const timeoutMs = handlers.timeoutMs ?? DEFAULT_LAUNCH_TIMEOUT_MS;
        const launch = new Launch(this, token, application, timeoutMs, this.#host.now() + timeoutMs, handlers);
        if (this.#launches.length >= MAX_LAUNCHES) {
            const finished = this.#launches.find((held) => !held.alive);
            if (finished === undefined) {
                Launch.settle(launch, this.#host, "ended", `the registry holds ${MAX_LAUNCHES} pending launches`);
                return launch;
            }
            this.#launches.splice(this.#launches.indexOf(finished), 1);
        }
        this.#launches.push(launch);
        return launch;
    }

    leave(launch: Launch, state: "ended"): void {
        Launch.settle(launch, this.#host, state, state);
    }

    /** A window of `appId` appeared or took focus: it starts the oldest pending launch that matches. */
    answered(appId: string): void {
        if (appId === "") {
            return;
        }
        const wanted = normalizeApplication(appId);
        const match = this.#launches.find(
            (launch) => launch.alive && normalizeApplication(launch.application) === wanted,
        );
        if (match !== undefined) {
            Launch.settle(match, this.#host, "started", "started");
        }
    }

    /** Whether a pending launch still waits for a window or its deadline. */
    get pending(): boolean {
        return this.#launches.some((launch) => launch.alive);
    }

    /** Times out the launches whose deadline has passed. The shell calls this once a frame. */
    frame(): void {
        if (!this.pending) {
            return;
        }
        const now = this.#host.now();
        for (const launch of this.#launches) {
            if (launch.alive && Launch.deadline(launch) <= now) {
                Launch.settle(launch, this.#host, "timed_out", "timed_out");
            }
        }
    }
}
