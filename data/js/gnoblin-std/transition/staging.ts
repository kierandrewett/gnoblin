// Holds the writes of a transition callback in the library. While a callback runs, the engine's reads and writes of a node stage in
// the library (`transition_begin`, `transition_set`, `transition_commit`): reads see the staged value, writes reach the tree only
// after the callback and the passes that follow it finish without an error, and the library sends them as timed writes.
//
// The engine and the library keep a mirror of the tree each, and the engine's pending writes are not the library's, so this class is
// the one place that joins them: a write the library can hold goes to it (it checks the write at once, so the setter that made it
// throws, naming its own path), and the engine reads its own staged values back from here. A write the library cannot hold, an
// effect list or a node reference, waits here and goes through the engine when the transition has committed.

import { Transition as Native } from "gnoblin-std/generated/classes";
import { Prop } from "gnoblin-std/generated/records";
import type { TransitionOptions as NativeOptions } from "gnoblin-std/generated/records";
import type { Init } from "gnoblin-std/generated/native";
import { refusalOf, stdFailure, unknownNode, wrongKind } from "gnoblin-std/native_error";
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
export const copy = (value: Value): Value => (Array.isArray(value) ? [...value] : value);

/** The library's form of a value it can hold, or nothing for one it cannot. */
function propOf(value: Value): Init<Prop> | undefined {
    if (typeof value === "number") {
        return Prop.number(value);
    }
    if (typeof value === "boolean") {
        return Prop.bool(value ? 1 : 0);
    }
    if (typeof value === "string") {
        return Prop.text(value);
    }
    if (Array.isArray(value) && value.length === 3 && value.every((part) => typeof part === "number")) {
        return Prop.triple(value[0], value[1], value[2]);
    }
    return undefined;
}

/** A write waiting for the transition to commit, or one the engine reads back. */
interface Held {
    node: number;
    key: string;
    value: Value;
    /** Whether the library holds it (else it goes through the engine after the commit). */
    staged: boolean;
}

/**
 * The reads and writes of the engine around a transition callback. Outside a callback every call goes to the engine's own host; a
 * write of a property a running transition may own is made through the library, which tells that transition it lost it.
 */
export class Staging {
    #backend: StageBackend;
    #error: MakeError;
    #held: Map<string, Held> | null = null;
    /** How many transitions have not ended: while there are none a plain write has nothing to release. */
    #running = 0;
    /**
     * What a commit sent for the engine's own reads: the library's writes reach the engine's mirror when the compositor echoes them,
     * so until the engine's value moves off what it was at the commit, a read answers with the committed value.
     */
    #echo = new Map<string, { value: Value; before: Value }>();

    constructor(backend: StageBackend, error: MakeError) {
        this.#backend = backend;
        this.#error = error;
    }

    /** Whether property reads and writes are currently staged. */
    get active(): boolean {
        return this.#held !== null;
    }

    /** A transition began, or ended: the count a plain write consults. */
    track(delta: 1 | -1): void {
        this.#running += delta;
        if (this.#running === 0) {
            this.#echo.clear();
        }
    }

    /** Starts staging in the library. A second start fails, naming the call. */
    begin(options: Init<NativeOptions>): void {
        try {
            Native.begin(options);
        } catch (error) {
            throw stdFailure(error, "gnoblin.requestTransition");
        }
        this.#held = new Map();
    }

    /**
     * Ends staging: the library runs its passes against the staged state and sends what was staged as timed writes, and the engine
     * sends what only it could hold. Returns the transition the library made.
     */
    commit(): Native {
        const held = this.#held;
        this.#held = null;
        let made: Native | null;
        try {
            made = Native.commit();
        } catch (error) {
            throw stdFailure(error, "gnoblin.requestTransition");
        }
        if (made === null) {
            throw this.#error("internal", "gnoblin.requestTransition", "the library made no transition");
        }
        for (const entry of (held ?? new Map<string, Held>()).values()) {
            if (entry.staged) {
                this.#echo.set(`${entry.node}:${entry.key}`, {
                    value: copy(entry.value),
                    before: copy(this.#backend.get(entry.node, entry.key)),
                });
            } else {
                this.#backend.set(entry.node, entry.key, entry.value);
            }
        }
        return made;
    }

    /** Stops staging and drops what was staged. */
    discard(): void {
        if (this.#held === null) {
            return;
        }
        this.#held = null;
        Native.abort();
    }

    /** Reads a staged value first, otherwise reads from the engine. */
    get(node: number, key: string): Value {
        const id = `${node}:${key}`;
        const staged = this.#held?.get(id);
        if (staged !== undefined) {
            return copy(staged.value);
        }
        const now = this.#backend.get(node, key);
        const sent = this.#echo.get(id);
        if (sent === undefined) {
            return now;
        }
        if (JSON.stringify(now) !== JSON.stringify(sent.before)) {
            this.#echo.delete(id);
            return now;
        }
        return copy(sent.value);
    }

    /** Writes immediately or stages the value until the callback completes. */
    set(node: number, key: string, value: Value): string | undefined {
        const held = this.#held;
        if (held === null) {
            return this.#plain(node, key, value);
        }
        const prop = propOf(value);
        let staged = false;
        if (prop !== undefined) {
            try {
                Native.set(node, key, prop);
                staged = true;
            } catch (error) {
                // A value the library cannot hold for this property (a node reference written as a number), or a node only the engine
                // knows yet (made in this frame, not yet in the tree the library reads), waits for the engine.
                if (!wrongKind(error) && !unknownNode(error)) {
                    return refusalOf(error);
                }
            }
        }
        if (!staged) {
            const refused = this.#backend.check(node, key, value, false);
            if (refused != null) {
                return refused;
            }
        }
        held.set(`${node}:${key}`, { node, key, value: copy(value), staged });
        return undefined;
    }

    /**
     * Stages a write with a curve of its own (a workspace slide that a callback's layout pass starts), inside a callback; outside
     * one, `begin` and `commit` wrap it. The library checks it at once, and refuses a property the tree cannot animate.
     */
    animate(node: number, key: string, value: Value, options: Init<NativeOptions>): string | undefined {
        const held = this.#held;
        const prop = propOf(value);
        if (held === null || prop === undefined) {
            return "invalid-value";
        }
        let staged = true;
        try {
            Native.animate(node, key, prop, options);
        } catch (error) {
            if (!unknownNode(error)) {
                return refusalOf(error);
            }
            // A node the library cannot hold writes of takes the value at the commit, from the engine, with no curve.
            staged = false;
        }
        held.set(`${node}:${key}`, { node, key, value: copy(value), staged });
        return undefined;
    }

    /** Throws for a call that has a side effect, while a callback is staging. Otherwise does nothing. */
    forbid(target: string): void {
        if (this.#held !== null) {
            throw this.#error(
                "invalid_state",
                target,
                "is not valid inside a requestTransition callback: it has a side effect, and a callback only stages property writes",
            );
        }
    }

    /**
     * A write made outside a callback: the engine writes it. While a transition runs the library is told first, so that the transition
     * that played the property lets it go. The engine's write travels in the frame's transaction and a transition's timed writes in
     * the dispatch's, so a plain write made in the same frame as the commit that animates it reaches the compositor first; a write
     * made in a later frame, which is what a script that takes a property from an animation does, ends the track.
     */
    #plain(node: number, key: string, value: Value): string | undefined {
        this.#echo.delete(`${node}:${key}`);
        if (this.#running > 0) {
            try {
                Native.release(node, key);
            } catch {
                // The library does not know the node: no transition of it played the property.
            }
        }
        return this.#backend.set(node, key, value);
    }
}
