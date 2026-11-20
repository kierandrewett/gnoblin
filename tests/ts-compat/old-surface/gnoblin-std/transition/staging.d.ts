import { Transition as Native } from "gnoblin-std/emitted/classes";
import type { TransitionOptions as NativeOptions } from "gnoblin-std/emitted/records";
import type { Init } from "gnoblin-std/emitted/native";
import type { ErrorCode } from "gnoblin-core/errors";
/** A value as it goes to the tree: a number, a flag, a text, a list of numbers or effect objects. */
export type Value = unknown;
/** The host operations the engine's own mirror answers: a read, a write, and what a write would answer. */
export interface StageBackend {
    /** Reads a property value. */
    get(node: number, key: string): Value;
    /** Writes a property and returns a refusal word if the write is refused. */
    set(node: number, key: string, value: Value): string | undefined;
    /** What `set` would answer, without queuing the write. */
    check(node: number, key: string, value: Value, timed: boolean): string | undefined;
}
/**
 * Makes the error a failed call throws: `code` says why, `target` names the member or call, `message` the reason.
 */
export type MakeError = (code: ErrorCode, target: string, message: string) => Error;
/** A copy of a staged value, so a later write does not change what was read. */
export declare const copy: (value: Value) => Value;
/**
 * The reads and writes of the engine around a transition callback. Outside a callback every call goes to the engine's own host; a
 * write of a property a running transition may own is made through the library, which tells that transition it lost it.
 */
export declare class Staging {
    #private;
    constructor(backend: StageBackend, error: MakeError);
    /** Whether property reads and writes are currently staged. */
    get active(): boolean;
    /** A transition began, or ended: the count a plain write consults. */
    track(delta: 1 | -1): void;
    /** Starts staging in the library. A second start fails, naming the call. */
    begin(options: Init<NativeOptions>): void;
    /**
     * Ends staging: the library runs its passes against the staged state and sends what was staged as timed writes, and the engine
     * sends what only it could hold. Returns the transition the library made.
     */
    commit(): Native;
    /** Stops staging and drops what was staged. */
    discard(): void;
    /** Reads a staged value first, otherwise reads from the engine. */
    get(node: number, key: string): Value;
    /** Writes immediately or stages the value until the callback completes. */
    set(node: number, key: string, value: Value): string | undefined;
    /** Throws for a call that has a side effect, while a callback is staging. Otherwise does nothing. */
    forbid(target: string): void;
}
