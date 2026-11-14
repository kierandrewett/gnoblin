// Operations and timers: what an asynchronous call of the host returns, and the clock that runs its deadlines.
//
// An `Operation` is the result of a call that finishes later: `gnoblin.session.poweroff()`, `process.kill()`,
// `gnoblin.runtime.reloadConfig()`. It is settled once, with a value or with a typed error, and the callbacks
// added to it run as queued calls under the same wall-clock budget as every listener: one that throws is reported
// and the next callback still runs, and a callback added after the operation settled runs once, after the current
// call returns.
//
//     gnoblin.session.suspend().onComplete((value, error) => {
//         if (error) console.log(error.code, error.message);
//     });
//
// Both classes are plain TypeScript over a small host (`OperationHost`), so they run without a compositor.

/** What an operation can fail with. The codes are the ones `docs/shell-api/errors.md` lists, plus the reload's own. */
export type OperationErrorCode =
    | "invalid_argument"
    | "not_found"
    | "unsupported"
    | "denied"
    | "busy"
    | "cancelled"
    | "timed_out"
    | "unavailable"
    | "internal"
    | "config_rejected";

/** The failure of an operation: a stable `code` to branch on and a `message` for a person. */
export interface OperationError {
    readonly code: OperationErrorCode;
    readonly message: string;
}

export type OperationStatus = "pending" | "succeeded" | "failed";

/** Stops a callback or a handler before it runs. Calling it twice is harmless. */
export interface Subscription {
    unsubscribe(): void;
}

/** What timers need of the shell. */
export interface ClockHost {
    /** Queues `task` after the calls already queued, under the listener budget. `site` names it in a report. */
    schedule(site: string, task: () => void): void;
    /** The monotonic clock, in milliseconds. */
    now(): number;
}

/** What operations need of the shell: the queue, and a timer for a deadline. */
export interface OperationHost extends ClockHost {
    after(ms: number, fn: () => void): { cancel(): boolean };
}

/** The ceiling on pending timers, as the Lua host's. A config past it is looping. */
export const MAX_TIMERS = 1024;

/** The longest delay a timer takes, in milliseconds: a day. */
export const MAX_TIMER_MS = 86_400_000;

type Callback<T> = (value: T | undefined, error: OperationError | undefined) => void;

/**
 * The result of a call that finishes later. `status` is `pending` until the operation settles; then `value` holds the
 * result of a success and `error` the failure of the other outcome, never both.
 */
export class Operation<T> {
    /** Counts the operations of a generation, so a log line can name one. */
    readonly id: number;
    /** The call that made it, such as `session.poweroff`. */
    readonly method: string;
    #host: OperationHost;
    #status: OperationStatus = "pending";
    #value: T | undefined;
    #error: OperationError | undefined;
    #callbacks: { run: Callback<T>; live: boolean }[] = [];
    #cancel: (() => void) | undefined;

    /** Only the host makes operations: a script receives them from the call that started the work. */
    constructor(host: OperationHost, id: number, method: string, cancel?: () => void) {
        this.#host = host;
        this.id = id;
        this.method = method;
        this.#cancel = cancel;
    }

    get status(): OperationStatus {
        return this.#status;
    }

    get value(): T | undefined {
        return this.#value;
    }

    get error(): OperationError | undefined {
        return this.#error;
    }

    /**
     * Calls `fn(value, error)` once when the operation settles: `error` is set on a failure and `value` on a
     * success. The returned subscription stops the callback before it runs.
     */
    onComplete(fn: (value: T | undefined, error: OperationError | undefined) => void): Subscription {
        if (typeof fn !== "function") {
            throw new TypeError(`${this.method}.onComplete: expects a function`);
        }
        const entry = { run: fn, live: true };
        const subscription: Subscription = {
            unsubscribe: () => {
                entry.live = false;
            },
        };
        if (this.#status === "pending") {
            this.#callbacks.push(entry);
        } else {
            this.#deliver(entry);
        }
        return subscription;
    }

    /** A new operation that succeeds with `fn(value)`, or fails with the same error. A throw in `fn` is `internal`. */
    map<U>(fn: (value: T) => U): Operation<U> {
        if (typeof fn !== "function") {
            throw new TypeError(`${this.method}.map: expects a function`);
        }
        const mapped = new Operation<U>(this.#host, this.id, `${this.method}.map`);
        this.onComplete((value, error) => {
            if (error !== undefined) {
                mapped.fail(error.code, error.message);
                return;
            }
            try {
                mapped.succeed(fn(value as T));
            } catch (thrown) {
                mapped.fail("internal", thrown instanceof Error ? thrown.message : String(thrown));
            }
        });
        return mapped;
    }

    /**
     * A new operation that fails `timed_out` if this one has not settled in `seconds`, and settles as this one does
     * if it is first. It does not cancel this operation.
     */
    timeout(seconds: number): Operation<T> {
        const limited = new Operation<T>(this.#host, this.id, `${this.method}.timeout`);
        const timer = this.#host.after(seconds * 1000, () =>
            limited.fail("timed_out", `${this.method} took over ${seconds} s`),
        );
        this.onComplete((value, error) => {
            timer.cancel();
            if (error !== undefined) {
                limited.fail(error.code, error.message);
            } else {
                limited.succeed(value as T);
            }
        });
        return limited;
    }

    /**
     * Stops waiting: the operation fails `cancelled` and its callbacks run. Returns false when it had already
     * settled. Work already sent to the compositor still happens; its result is ignored.
     */
    cancel(): boolean {
        if (this.#status !== "pending") {
            return false;
        }
        this.#cancel?.();
        return this.fail("cancelled", `${this.method} was cancelled`);
    }

    /** Settles the operation with `value`. Host-only: a script has no use for it. Returns false if it was settled. */
    succeed(value: T): boolean {
        if (this.#status !== "pending") {
            return false;
        }
        this.#status = "succeeded";
        this.#value = value;
        this.#flush();
        return true;
    }

    /** Settles the operation with a failure. Host-only. Returns false if it was settled. */
    fail(code: OperationErrorCode, message: string): boolean {
        if (this.#status !== "pending") {
            return false;
        }
        this.#status = "failed";
        this.#error = { code, message };
        this.#flush();
        return true;
    }

    #flush(): void {
        const callbacks = this.#callbacks;
        this.#callbacks = [];
        for (const entry of callbacks) {
            this.#deliver(entry);
        }
    }

    #deliver(entry: { run: Callback<T>; live: boolean }): void {
        this.#host.schedule(`${this.method}.onComplete`, () => {
            if (entry.live) {
                entry.live = false;
                entry.run(this.#value, this.#error);
            }
        });
    }
}

/** A pending call of a function at a time. */
export class Timer {
    readonly id: number;
    /** The time it runs at, in milliseconds on the monotonic clock. */
    readonly at: number;
    #pending = true;
    #fn: () => void;
    #owner: Timers;

    constructor(owner: Timers, id: number, at: number, fn: () => void) {
        this.#owner = owner;
        this.id = id;
        this.at = at;
        this.#fn = fn;
    }

    /** True until the timer has run or was cancelled. */
    get pending(): boolean {
        return this.#pending;
    }

    /** Stops the timer. Returns false when it had already run or was cancelled. */
    cancel(): boolean {
        if (!this.#pending) {
            return false;
        }
        this.#pending = false;
        this.#owner.forget(this);
        return true;
    }

    /** Marks the timer spent and returns what it runs. Host-only: `Timers.fire` is its one caller. */
    take(): () => void {
        this.#pending = false;
        return this.#fn;
    }
}

/**
 * `gnoblin.timers`: calls that run later, on the shell's frame loop.
 *
 * A timer runs at the first frame at or after its time, as a queued call under the listener budget, so one that
 * throws is reported and never stops another. The clock is monotonic and measured in milliseconds; only the
 * difference between two readings is meaningful. A generation's pending timers end with it: `cancelAll` runs when
 * a reload or a shutdown retires the generation.
 */
export class Timers {
    #host: ClockHost;
    #pending: Timer[] = [];
    #next = 1;

    constructor(host: ClockHost) {
        this.#host = host;
    }

    /** The monotonic clock in milliseconds. */
    now(): number {
        return this.#host.now();
    }

    /** How many timers are waiting. */
    get size(): number {
        return this.#pending.length;
    }

    /** Runs `fn` once, `ms` milliseconds from now (0 to a day). The returned timer can be cancelled. */
    after(ms: number, fn: () => void): Timer {
        if (typeof ms !== "number" || !Number.isFinite(ms) || ms < 0 || ms > MAX_TIMER_MS) {
            throw new TypeError(`gnoblin.timers.after: ms must be a finite number from 0 to ${MAX_TIMER_MS}`);
        }
        if (typeof fn !== "function") {
            throw new TypeError("gnoblin.timers.after: needs a function to run");
        }
        if (this.#pending.length >= MAX_TIMERS) {
            throw new RangeError(`gnoblin.timers.after: ${MAX_TIMERS} timers are already pending`);
        }
        const timer = new Timer(this, this.#next++, this.#host.now() + ms, fn);
        this.#pending.push(timer);
        return timer;
    }

    /** Drops a timer from the pending list. Called by `Timer.cancel`. */
    forget(timer: Timer): void {
        const index = this.#pending.indexOf(timer);
        if (index >= 0) {
            this.#pending.splice(index, 1);
        }
    }

    /** Queues every timer that is due, earliest first. The shell calls it once a frame. */
    fire(): void {
        const now = this.#host.now();
        const due = this.#pending.filter((timer) => timer.at <= now).sort((a, b) => a.at - b.at || a.id - b.id);
        for (const timer of due) {
            this.forget(timer);
            this.#host.schedule("gnoblin.timers.after", timer.take());
        }
    }

    /** Cancels every pending timer. */
    cancelAll(): void {
        for (const timer of [...this.#pending]) {
            timer.cancel();
        }
    }
}
