import type { ErrorReport } from "gnoblin-core/host/contract";
/** An object that provides listeners for deferred calls. */
export interface Source {
    /** Listeners attached to this source. */
    listeners: object[];
}
/** An internal call, such as an input hook, run with the same budget as listener calls. */
export interface Task {
    /** Name used to identify errors from this task. */
    site: string;
    /** Callback to run when the task reaches the front of the queue. */
    task(): void;
}
/** A deferred listener call with its source, method and arguments. */
export interface Invocation {
    /** Source that must still contain the listener for the call to run. */
    source: Source;
    /** Listener whose handler is invoked. */
    listener: object;
    /** Name of the handler to read when the call runs. */
    method: string;
    /** Arguments passed to the handler. */
    args: unknown[];
}
/** Reads the current handler property from a listener. */
export declare function handlerOf(listener: object, method: string): unknown;
/** Names a listener constructor for error reports, falling back to `listener`. */
export declare function describeListener(listener: object): string;
/** Adds `listener` to `list` unless it is there already. Returns whether it was added. */
export declare function attach<L extends object>(list: L[], listener: L): boolean;
/** Removes `listener` from `list`. Returns whether it was there. */
export declare function detach<L extends object>(list: L[], listener: L): boolean;
/** Runs one queued call if its listener is still attached; a throw is reported, never propagated. */
export declare function run(call: Invocation | Task): void;
/** Queues each listener currently implementing `method` on a source. */
export declare function queueFor(source: Source, method: string, args: unknown[]): void;
/** Runs a host callback while collecting its error under `site` instead of aborting the frame. */
export declare function guard(site: string, body: () => void): void;
/** Deduplicates a notice so one unsupported style is reported only once per generation. */
export declare function say(text: string): void;
/** Number of deferred callbacks still waiting for the shell pump. */
export declare function pending(): number;
/** Runs one deferred callback and returns the remaining queue length. */
export declare function next(): number;
/** Transfers collected callback failures to the shell and starts an empty error batch. */
export declare function takeErrors(): ErrorReport[];
