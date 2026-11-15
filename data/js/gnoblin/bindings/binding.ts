// The binding classes and the collection a layout keeps them in. A binding is the trigger plus the hooks the matcher
// of `gnoblin/input` calls; it reaches no host. A collection owns its bindings by id, refuses two with one
// trigger, and tells a watcher (the input context) when one is added or removed. `owners` is the private link from
// a binding to the collection that holds it, so `unbind()` works without the binding keeping a strong reference a
// script could reach.

import { parseTrigger } from "gnoblin/bindings/trigger";
import type { ParsedTrigger } from "gnoblin/bindings/trigger_types";
import type { BindingEvent, BindingOptions, DragEvent } from "gnoblin/bindings/types";
import type { ErrorCode } from "gnoblin/errors";

/** Two bindings of one collection match one input at one priority: the matcher could not choose. */
export class BindingConflictError extends Error {
    /** The trigger is taken at that priority. */
    readonly code: ErrorCode = "already_exists";
    readonly trigger: string;
    readonly ids: readonly [string, string];

    constructor(trigger: string, existing: string, added: string, priority: number) {
        super(
            `binding "${added}" conflicts with "${existing}": both match ${trigger} at priority ${priority}; ` +
                "give one a different priority or trigger",
        );
        this.name = "BindingConflictError";
        this.trigger = trigger;
        this.ids = [existing, added];
    }
}

const owners = new WeakMap<Binding, BindingCollection>();

/**
 * The base of every binding: an id, a trigger and an enabled flag. A subclass decides what a matched input does.
 */
export class Binding {
    readonly id: string;
    enabled: boolean;
    priority: number;
    #trigger: ParsedTrigger;
    #written: string;

    /**
     * How a binding reads its trigger. The default is the trigger grammar of keys, buttons and gestures; a binding
     * for another input source (an XR controller) overrides it, and this one is what its trigger setter uses too.
     */
    static readTrigger(trigger: unknown, what: string): ParsedTrigger {
        return parseTrigger(trigger, what);
    }

    constructor(options: BindingOptions) {
        if (typeof options?.id !== "string" || options.id === "") {
            throw new TypeError("Binding id must be a non-empty string");
        }
        const parsed = (new.target as typeof Binding).readTrigger(options.trigger, `Binding "${options.id}" trigger`);
        if (options.priority !== undefined && !Number.isFinite(options.priority)) {
            throw new RangeError(`Binding "${options.id}" priority must be a finite number`);
        }
        this.id = options.id;
        this.#trigger = parsed;
        this.#written = options.trigger;
        this.enabled = options.enabled ?? true;
        this.priority = options.priority ?? 0;
    }

    /** The trigger as it was written. Assigning a new one moves the binding, which cancels a gesture it runs. */
    get trigger(): string {
        return this.#written;
    }

    set trigger(value: string) {
        const parsed = (this.constructor as typeof Binding).readTrigger(value, `Binding "${this.id}" trigger`);
        const owner = owners.get(this);
        const previous = this.#trigger;
        owner?.assertFree(this, parsed, this.priority);
        this.#trigger = parsed;
        this.#written = value;
        owner?.retriggered(this, previous);
    }

    /** The grammar's reading of `trigger`. */
    get parsed(): ParsedTrigger {
        return this.#trigger;
    }

    /** Takes the binding out of the collection that holds it. A binding that is in none is left alone. */
    unbind(): void {
        owners.get(this)?.remove(this.id);
    }

    /** Returns the state object for this gesture, or false to decline it. */
    onStart(_event: DragEvent): object | false {
        return false;
    }

    onUpdate(_event: DragEvent, _state: object): void {}

    onEnd(_event: DragEvent, _state: object): void {}

    onCancel(_event: DragEvent, _state: object): void {}

    /**
     * A key press or a mouse click of the trigger. Return `false` to decline, which lets the next binding of
     * the input (a lower tier) have it. A binding that only drags or gestures leaves this alone.
     */
    onPress?(event: BindingEvent): void | false;
}

/** Options of `KeyBinding`. */
export interface KeyBindingOptions extends BindingOptions {
    /** What runs on a press. A subclass may override `onPress` instead. */
    action?: (event: BindingEvent) => void | false;
}

/** A key (or key chord) that runs a function on press. */
export class KeyBinding extends Binding {
    readonly #action: ((event: BindingEvent) => void | false) | undefined;

    constructor(options: KeyBindingOptions) {
        super(options);
        if (options.action !== undefined && typeof options.action !== "function") {
            throw new TypeError(`Binding "${options.id}" action must be a function`);
        }
        this.#action = options.action;
    }

    onPress(event: BindingEvent): void | false {
        return this.#action?.(event);
    }
}

/** A mouse button or wheel trigger that runs a function (a click). Subclassed for drags. */
export class PointerBinding extends KeyBinding {}

/** Options of `PointerDragBinding` and the drag bindings built on it. */
export interface PointerDragBindingOptions extends BindingOptions {
    /** How far the pointer must move from the press before `onStart`, in pixels. Default 4. */
    threshold?: number;
}

/**
 * The base of every behaviour that follows an input from its start to its end (owner-spec.md section 11): the host
 * calls `onStart`, then `onUpdate` per sample, then `onEnd` or `onCancel`, each with the state object `onStart`
 * returned. Abstract: extend `PointerDragBinding` for a pointer drag, `GestureBinding` for a touchpad gesture, or
 * this class for a source the library does not know.
 */
export class DragBinding extends Binding {
    constructor(options: BindingOptions) {
        if (new.target === DragBinding) {
            throw new TypeError("DragBinding is abstract: extend it, or PointerDragBinding");
        }
        super(options);
    }
}

/** The base of the pointer drag behaviours: the host calls `onStart`, `onUpdate` and `onEnd` or `onCancel`. */
export class PointerDragBinding extends DragBinding {
    threshold: number;

    constructor(options: PointerDragBindingOptions) {
        super(options);
        const threshold = options.threshold ?? 4;
        if (!Number.isFinite(threshold) || threshold < 0) {
            throw new RangeError(`Binding "${options.id}" threshold must be a finite number of at least 0`);
        }
        this.threshold = threshold;
    }
}

/** A touchpad gesture the host follows from its begin to its end: same hooks and per-gesture state as a drag. */
export class GestureBinding extends Binding {}

/** A watcher of a collection. The input context registers claims through one. */
export interface BindingWatcher {
    added?(binding: Binding): void;
    removed?(binding: Binding): void;
    /** The trigger changed from `previous`; the binding already holds the new one. */
    retriggered?(binding: Binding, previous: ParsedTrigger): void;
}

/** The bindings of one layout, by id, in the order they were added. */
export class BindingCollection implements Iterable<Binding> {
    private readonly byId = new Map<string, Binding>();
    private readonly watchers: BindingWatcher[] = [];

    get size(): number {
        return this.byId.size;
    }

    /**
     * Adds a binding. A binding that is in another collection, an id that is taken, and a binding that matches
     * the same trigger at the same priority as one already here (the matcher could not choose) are refused.
     */
    add(binding: Binding): Binding {
        if (this.byId.has(binding.id)) {
            throw new Error(`a binding with id "${binding.id}" is already in this collection`);
        }
        const owner = owners.get(binding);
        if (owner !== undefined) {
            throw new Error(`binding "${binding.id}" is already in a collection; remove it there first`);
        }
        this.assertFree(binding, binding.parsed, binding.priority);
        this.byId.set(binding.id, binding);
        owners.set(binding, this);
        try {
            for (const watcher of [...this.watchers]) {
                watcher.added?.(binding);
            }
        } catch (error) {
            // A watcher that cannot take the binding (the compositor refused its claim) leaves the collection
            // as it was, so a refused binding is not half there.
            this.byId.delete(binding.id);
            owners.delete(binding);
            throw error;
        }
        return binding;
    }

    get(id: string): Binding | undefined {
        return this.byId.get(id);
    }

    has(id: string): boolean {
        return this.byId.has(id);
    }

    remove(id: string): boolean {
        const binding = this.byId.get(id);
        if (binding === undefined) {
            return false;
        }
        this.byId.delete(id);
        owners.delete(binding);
        for (const watcher of [...this.watchers]) {
            watcher.removed?.(binding);
        }
        return true;
    }

    clear(): void {
        for (const id of [...this.byId.keys()]) {
            this.remove(id);
        }
    }

    /** Calls `watcher` for later changes. Members that are already here are not replayed. Returns the unwatch. */
    watch(watcher: BindingWatcher): () => void {
        this.watchers.push(watcher);
        return () => {
            const at = this.watchers.indexOf(watcher);
            if (at >= 0) {
                this.watchers.splice(at, 1);
            }
        };
    }

    /** @internal Refuses `trigger` at `priority` for `binding` when another member already matches it. */
    assertFree(binding: Binding, trigger: ParsedTrigger, priority: number): void {
        for (const other of this.byId.values()) {
            if (other !== binding && other.parsed.text === trigger.text && other.priority === priority) {
                throw new BindingConflictError(trigger.text, other.id, binding.id, priority);
            }
        }
    }

    /** @internal Tells the watchers a member's trigger changed. */
    retriggered(binding: Binding, previous: ParsedTrigger): void {
        for (const watcher of [...this.watchers]) {
            watcher.retriggered?.(binding, previous);
        }
    }

    [Symbol.iterator](): Iterator<Binding> {
        return this.byId.values();
    }
}

/** Asserts `trigger` is a trigger the grammar accepts. `what` names the holder in the message. */
export function checkTrigger(trigger: unknown, what: string): asserts trigger is string {
    parseTrigger(trigger, what);
}

/**
 * The trigger of each named binding of a layout: `defaults` with `overrides` on top. A name that is
 * omitted or `undefined` keeps its default and `null` turns that binding off; `{ ...defaults, ...overrides }`
 * would let an explicit `undefined` erase the default (spec-review.md 1.19). A name that is not in
 * `defaults` is a typo and throws.
 */
export function resolveTriggers<K extends string>(
    defaults: Readonly<Record<K, string>>,
    overrides: { readonly [P in K]?: string | null | undefined } = {},
): Record<K, string | null> {
    const resolved = { ...defaults } as Record<K, string | null>;
    for (const name of Object.keys(overrides)) {
        if (!Object.hasOwn(defaults, name)) {
            throw new TypeError(`unknown trigger "${name}"; expected one of ${Object.keys(defaults).join(", ")}`);
        }
        const value = overrides[name as K];
        if (value === undefined) {
            continue;
        }
        if (value !== null) {
            checkTrigger(value, `trigger "${name}"`);
        }
        resolved[name as K] = value;
    }
    return resolved;
}
