import type { ErrorCode } from "gnoblin-core/errors";
/** The failure of an operation: a stable `code` to branch on and a `message` for a person. */
export interface OperationError {
    /** Stable reason code for branching on the failure. */
    readonly code: ErrorCode;
    /** Human-readable failure detail. */
    readonly message: string;
}
/** Where an operation is. It starts `pending` and ends as `succeeded` or `failed`; it never goes back. */
export type OperationStatus = "pending" | "succeeded" | "failed";
/** Stops a callback or a handler before it runs. Calling it twice is harmless. */
export interface Subscription {
    /** Prevents the subscribed callback from running. Repeated calls have no effect. */
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
    /** Schedules `fn` after `ms` milliseconds and returns a timer with a `cancel()` method. */
    after(ms: number, fn: () => void): {
        cancel(): boolean;
    };
}
/** The ceiling on pending timers, as the retired Lua host's. A config past it is looping. */
export declare const MAX_TIMERS = 1024;
/** The longest delay a timer takes, in milliseconds: a day. */
export declare const MAX_TIMER_MS = 86400000;
/**
 * The result of a call that finishes later. `status` is `pending` until the operation settles; then `value` holds the
 * result of a success and `error` the failure of the other outcome, never both.
 */
export declare class Operation<T> {
    #private;
    /** Counts the operations of a generation, so a log line can name one. */
    readonly id: number;
    /** The call that made it, such as `session.poweroff`. */
    readonly method: string;
    /** Only the host makes operations: a script receives them from the call that started the work. */
    constructor(host: OperationHost, id: number, method: string, cancel?: () => void);
    /** Current settlement state. */
    get status(): OperationStatus;
    /** Result value after success; otherwise `undefined`. */
    get value(): T | undefined;
    /** Failure detail after failure; otherwise `undefined`. */
    get error(): OperationError | undefined;
    /**
     * Calls `fn(value, error)` once when the operation settles: `error` is set on a failure and `value` on a
     * success. The returned subscription stops the callback before it runs.
     */
    onComplete(fn: (value: T | undefined, error: OperationError | undefined) => void): Subscription;
    /** A new operation that succeeds with `fn(value)`, or fails with the same error. A throw in `fn` is `internal`. */
    map<U>(fn: (value: T) => U): Operation<U>;
    /**
     * A new operation that fails `timed_out` if this one has not settled in `ms` milliseconds, and settles as this
     * one does if it is first. It does not cancel this operation.
     */
    timeout(ms: number): Operation<T>;
    /**
     * Stops waiting: the operation fails `cancelled` and its callbacks run. Returns false when it had already
     * settled. Work already sent to the compositor still happens; its result is ignored.
     */
    cancel(): boolean;
    /** Settles the operation with `value`. Host-only: a script has no use for it. Returns false if it was settled. */
    succeed(value: T): boolean;
    /** Settles the operation with a failure. Host-only. Returns false if it was settled. */
    fail(code: ErrorCode, message: string): boolean;
}
/** A pending call of a function at a time. */
export declare class Timer {
    #private;
    /** Unique timer id within its timer collection. */
    readonly id: number;
    /** The time it runs at, in milliseconds on the monotonic clock. */
    readonly at: number;
    constructor(owner: Timers, id: number, at: number, fn: () => void);
    /** True until the timer has run or was cancelled. */
    get pending(): boolean;
    /** Stops the timer. Returns false when it had already run or was cancelled. */
    cancel(): boolean;
    /** Marks the timer spent and returns what it runs. Host-only: `Timers.fire` is its one caller. */
    take(): () => void;
}
/**
 * `gnoblin.timers`: calls that run later, on the shell's frame loop.
 *
 * A timer runs at the first frame at or after its time, as a queued call under the listener budget, so one that
 * throws is reported and never stops another. The clock is monotonic and measured in milliseconds; only the
 * difference between two readings is meaningful. A generation's pending timers end with it: `cancelAll` runs when
 * a reload or a shutdown retires the generation.
 */
export declare class Timers {
    #private;
    constructor(host: ClockHost);
    /** The monotonic clock in milliseconds. */
    now(): number;
    /** How many timers are waiting. */
    get size(): number;
    /** Runs `fn` once, `ms` milliseconds from now (0 to a day). The returned timer can be cancelled. */
    after(ms: number, fn: () => void): Timer;
    /** Drops a timer from the pending list. Called by `Timer.cancel`. */
    forget(timer: Timer): void;
    /** Queues every timer that is due, earliest first. The shell calls it once a frame. */
    fire(): void;
    /** Cancels every pending timer. */
    cancelAll(): void;
}
