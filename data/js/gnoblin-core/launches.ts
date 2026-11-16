// Launch feedback tracks the time between starting an application and its window appearing. The shell records pending
// launches, marks one started when a matching window appears or takes focus, and times it out at its deadline.
// A window starts only the oldest matching pending launch. This registry uses a small host interface and runs without
// a compositor.

import { NEVER, onWake } from "gnoblin-core/wake";

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

/**
 * Observes one launch: pass it to `gnoblin.launches.begin` as `listener`. Every method is optional, and each receives
 * the launch first.
 */
export class LaunchListener {
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

const NAME_MAX = 128;
const APPLICATION_MAX = 512;

/** The form two application IDs are compared in: lowercase, no `.desktop`. */
export function normalizeApplication(name: string): string {
    const lower = name.toLowerCase();
    return lower.endsWith(".desktop") ? lower.slice(0, -".desktop".length) : lower;
}

/** One launch. It is the same object for as long as it is in the registry. */
export class Launch {
    /** The caller's unique key for matching this launch. */
    readonly token: string;
    /** Application identifier used to match a window, such as `org.example.App.desktop`. */
    readonly application: string;
    /** The deadline asked for, in milliseconds. */
    readonly timeoutMs: number;
    /** Unix time in milliseconds when `begin` was called. */
    readonly startedAt: number;
    #state: LaunchState = "pending";
    #revision = 0;
    readonly #deadline: number;
    readonly #handlers: LaunchListener;
    readonly #registry: LaunchFeedback;

    /** Made by `gnoblin.launches.begin`; a script has no use for the constructor. */
    constructor(
        registry: LaunchFeedback,
        token: string,
        application: string,
        timeoutMs: number,
        deadline: number,
        handlers: LaunchListener,
    ) {
        this.#registry = registry;
        this.token = token;
        this.application = application;
        this.timeoutMs = timeoutMs;
        this.startedAt = Date.now();
        this.#deadline = deadline;
        this.#handlers = handlers;
    }

    /** The current state, from `pending` to one terminal state. */
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

    /** Returns the launch's deadline in monotonic milliseconds. */
    static deadline(launch: Launch): number {
        return launch.#deadline;
    }
}

function validate(token: unknown, application: unknown, options: unknown): LaunchOptions {
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
    const known = ["timeoutMs", "listener"];
    for (const key of Object.keys(given)) {
        if (!known.includes(key)) {
            throw new TypeError(`gnoblin.launches.begin: unknown option "${key}" (known: ${known.join(", ")})`);
        }
    }
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
    const listener = given.listener;
    if (listener !== undefined) {
        if (listener === null || typeof listener !== "object") {
            throw new TypeError("gnoblin.launches.begin: the listener must be an object");
        }
        for (const name of ["onChanged", "onStarted", "onTimedOut", "onClosed"]) {
            const member = (listener as Record<string, unknown>)[name];
            if (member !== undefined && typeof member !== "function") {
                throw new TypeError(`gnoblin.launches.begin: ${name} must be a function`);
            }
        }
    }
    return given as LaunchOptions;
}

/** `gnoblin.launches`: the launches the shell tracks, and the windows that answer them. */
export class LaunchFeedback {
    readonly #host: LaunchHost;
    #launches: Launch[] = [];

    constructor(host: LaunchHost) {
        this.#host = host;
        // A pending launch times out at its deadline.
        onWake("launch deadlines", () =>
            this.#launches.reduce(
                (earliest, launch) => (launch.alive ? Math.min(earliest, Launch.deadline(launch)) : earliest),
                NEVER,
            ),
        );
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
    begin(token: string, application: string, options?: LaunchOptions): Launch {
        const given = validate(token, application, options);
        const earlier = this.#launches.find((launch) => launch.token === token);
        if (earlier !== undefined) {
            Launch.settle(earlier, this.#host, "ended", "replaced");
            this.#launches.splice(this.#launches.indexOf(earlier), 1);
        }
        const timeoutMs = given.timeoutMs ?? DEFAULT_LAUNCH_TIMEOUT_MS;
        const launch = new Launch(
            this,
            token,
            application,
            timeoutMs,
            this.#host.now() + timeoutMs,
            given.listener ?? {},
        );
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

    /** Ends a pending launch as `ended`. */
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
