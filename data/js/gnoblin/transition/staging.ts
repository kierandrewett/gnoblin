// Staging: while a transition's callback runs, every write to a node (the host's `set`) is held here, not queued. A
// read gives the staged value, and nothing reaches the tree until the callback and the passes after it have finished
// without an error.

import type { ErrorCode } from "gnoblin/errors";

/** A value as it goes to the tree: a number, a flag, a text, a list of numbers or effect objects. */
export type Value = unknown;

/**
 * The timed write a staged value is sent as, when it has a curve of its own (`Client.animate` inside a callback). A
 * transition adds the `delay` in milliseconds before it starts and the `tween` node whose play state drives it.
 */
interface Curve {
    duration: number;
    samples: number[];
    delay?: number;
    tween?: number;
}

/** One write held while a transition callback runs. */
export interface Staged {
    node: number;
    key: string;
    value: Value;
    curve: Curve | null;
}

/**
 * The host functions staging stands in front of. Codes are the tree's: `unknown-node`, `read-only`, `invalid-value`
 * ...
 */
export interface StageBackend {
    get(node: number, key: string): Value;
    set(node: number, key: string, value: Value): string | undefined;
    /** A timed write: `delay` milliseconds before it starts (0 for none), joining the `tween` node (0 for none). */
    animate(
        node: number,
        key: string,
        value: Value,
        duration: number,
        samples: number[],
        delay?: number,
        tween?: number,
    ): string | undefined;
    /**
     * What `set` would answer, without queuing the write; with `timed` it also refuses a property the tree cannot
     * animate.
     */
    check(node: number, key: string, value: Value, timed: boolean): string | undefined;
}

/**
 * Makes the error a failed transition throws: `code` says why, `target` names the member or call, `message` the
 * reason.
 */
export type MakeError = (code: ErrorCode, target: string, message: string) => Error;

/** A copy of a staged value, so a later write does not change what was read. */
export const copy = (value: Value): Value => (Array.isArray(value) ? [...value] : value);

/**
 * The writes of one callback. Outside a callback it is the host: every call goes straight through, and a write tells
 * `onWrite` so that a transition that owned the property lets it go. Inside, a write is checked at once (so the setter
 * that made it throws, naming its own path) and kept until `end`.
 */
export class Staging {
    #backend: StageBackend;
    #entries: Map<string, Staged> | null = null;
    #error: MakeError;
    /** Told of each write made outside a callback, before it is queued. */
    onWrite: ((node: number, key: string) => void) | null = null;

    constructor(backend: StageBackend, error: MakeError) {
        this.#backend = backend;
        this.#error = error;
    }

    get active(): boolean {
        return this.#entries !== null;
    }

    begin(): void {
        this.#entries = new Map();
    }

    /** Stops staging and hands over what it kept, in the order the properties were first written. */
    end(): Staged[] {
        const kept = [...(this.#entries ?? []).values()];
        this.#entries = null;
        return kept;
    }

    /** Stops staging and drops what it kept. */
    discard(): void {
        this.#entries = null;
    }

    get(node: number, key: string): Value {
        const staged = this.#entries?.get(`${node}:${key}`);
        return staged === undefined ? this.#backend.get(node, key) : copy(staged.value);
    }

    set(node: number, key: string, value: Value): string | undefined {
        if (this.#entries === null) {
            this.onWrite?.(node, key);
            return this.#backend.set(node, key, value);
        }
        const code = this.#backend.check(node, key, value, false);
        if (code === undefined || code === null) {
            this.#keep({ node, key, value: copy(value), curve: null });
            return undefined;
        }
        return code;
    }

    animate(node: number, key: string, value: Value, duration: number, samples: number[]): string | undefined {
        if (this.#entries === null) {
            this.onWrite?.(node, key);
            return this.#backend.animate(node, key, value, duration, samples);
        }
        const code = this.#backend.check(node, key, value, true);
        if (code === undefined || code === null) {
            this.#keep({ node, key, value: copy(value), curve: { duration, samples } });
            return undefined;
        }
        return code;
    }

    /** Queues a write of the transaction's own, which is not a script's: it does not tell `onWrite`. */
    land(node: number, key: string, value: Value, curve: Curve | null): string | undefined {
        return curve === null
            ? this.#backend.set(node, key, value)
            : this.#backend.animate(
                  node,
                  key,
                  value,
                  curve.duration,
                  curve.samples,
                  curve.delay ?? 0,
                  curve.tween ?? 0,
              );
    }

    /** Throws for a call that has a side effect, while a callback is staging. Otherwise does nothing. */
    forbid(target: string): void {
        if (this.#entries !== null) {
            throw this.#error(
                "invalid_state",
                target,
                "is not valid inside a requestTransition callback: it has a side effect, and a callback only stages property writes",
            );
        }
    }

    #keep(entry: Staged): void {
        this.#entries?.set(`${entry.node}:${entry.key}`, entry);
    }
}
