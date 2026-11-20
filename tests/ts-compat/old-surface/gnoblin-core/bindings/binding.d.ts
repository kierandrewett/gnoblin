import type { ParsedTrigger } from "gnoblin-core/bindings/trigger_types";
import type { BindingEvent, BindingOptions, DragEvent } from "gnoblin-core/bindings/types";
import type { ErrorCode } from "gnoblin-core/errors";
/** Two bindings of one collection match one input at one priority: the matcher could not choose. */
export declare class BindingConflictError extends Error {
    /** The trigger is taken at that priority. */
    readonly code: ErrorCode;
    /** The canonical trigger that caused the conflict. */
    readonly trigger: string;
    /** The two conflicting binding ids, in existing-then-added order. */
    readonly ids: readonly [string, string];
    constructor(trigger: string, existing: string, added: string, priority: number);
}
/**
 * The base of every binding: an id, a trigger and an enabled flag. A subclass decides what a matched input does. A
 * drag or gesture binding names the type of the state object its `onStart` returns, which each later hook receives.
 */
export declare class Binding<State extends object = object> {
    #private;
    /** Unique within the collection that holds this binding. */
    readonly id: string;
    /** Whether the matcher considers this binding. Defaults to true. */
    enabled: boolean;
    /** Which matching binding wins. Defaults to 0; higher values win. */
    priority: number;
    /**
     * How a binding reads its trigger. The default is the trigger grammar of keys, buttons and gestures; a binding
     * for another input source (an XR controller) overrides it, and this one is what its trigger setter uses too.
     */
    static readTrigger(trigger: unknown, what: string): ParsedTrigger;
    constructor(options: BindingOptions);
    /** The trigger as it was written. Assigning a new one moves the binding, which cancels a gesture it runs. */
    get trigger(): string;
    /** Replaces the trigger; invalid spellings throw `TriggerError`. */
    set trigger(value: string);
    /** The grammar's reading of `trigger`. */
    get parsed(): ParsedTrigger;
    /** Takes the binding out of the collection that holds it. A binding that is in none is left alone. */
    unbind(): void;
    /** Returns the state object for this gesture, or false to decline it. */
    onStart(_event: DragEvent): State | false;
    /** Called for each sample after `onStart` accepts the input. */
    onUpdate(_event: DragEvent, _state: State): void;
    /** Called when an accepted input ends normally. */
    onEnd(_event: DragEvent, _state: State): void;
    /** Called when an accepted input is cancelled. */
    onCancel(_event: DragEvent, _state: State): void;
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
export declare class KeyBinding extends Binding {
    #private;
    constructor(options: KeyBindingOptions);
    /** Runs the optional action for this press. */
    onPress(event: BindingEvent): void | false;
}
/** A mouse button or wheel trigger that runs a function (a click). Subclassed for drags. */
export declare class PointerBinding extends KeyBinding {
}
/** Options of `PointerDragBinding` and the drag bindings built on it. */
export interface PointerDragBindingOptions extends BindingOptions {
    /** How far the pointer must move from the press before `onStart`, in pixels. Default 4. */
    threshold?: number;
}
/**
 * The base of every behaviour that follows an input from its start to its end: the host calls `onStart`, then
 * `onUpdate` for each sample, then `onEnd` or `onCancel`, each with the state object returned by `onStart`.
 * Extend `PointerDragBinding` for a pointer drag, `GestureBinding` for a touchpad gesture, or this class for an
 * input source the library does not know.
 */
export declare class DragBinding<State extends object = object> extends Binding<State> {
    constructor(options: BindingOptions);
}
/** The base of the pointer drag behaviours: the host calls `onStart`, `onUpdate` and `onEnd` or `onCancel`. */
export declare class PointerDragBinding<State extends object = object> extends DragBinding<State> {
    /** Minimum pointer travel in pixels before a drag starts. */
    threshold: number;
    constructor(options: PointerDragBindingOptions);
}
/** A touchpad gesture the host follows from its begin to its end: same hooks and per-gesture state as a drag. */
export declare class GestureBinding<State extends object = object> extends Binding<State> {
}
/** A watcher of a collection. The input context registers claims through one. */
export interface BindingWatcher {
    /** Called after a binding joins the collection. */
    added?(binding: Binding): void;
    /** Called after a binding leaves the collection. */
    removed?(binding: Binding): void;
    /** The trigger changed from `previous`; the binding already holds the new one. */
    retriggered?(binding: Binding, previous: ParsedTrigger): void;
}
/** The bindings of one layout, by id, in the order they were added. */
export declare class BindingCollection implements Iterable<Binding> {
    private readonly byId;
    private readonly watchers;
    /** Number of bindings currently in the collection. */
    get size(): number;
    /**
     * Adds a binding. A binding that is in another collection, an id that is taken, and a binding that matches
     * the same trigger at the same priority as one already here (the matcher could not choose) are refused.
     */
    add(binding: Binding): Binding;
    /** Returns the binding with this id, or `undefined` if absent. */
    get(id: string): Binding | undefined;
    /** Whether a binding with this id is in the collection. */
    has(id: string): boolean;
    /** Removes a binding by id. Returns false when the id is absent. */
    remove(id: string): boolean;
    /** Removes every binding from the collection. */
    clear(): void;
    /** Calls `watcher` for later changes. Members that are already here are not replayed. Returns the unwatch. */
    watch(watcher: BindingWatcher): () => void;
    /** @internal Refuses `trigger` at `priority` for `binding` when another member already matches it. */
    assertFree(binding: Binding, trigger: ParsedTrigger, priority: number): void;
    /** @internal Tells the watchers a member's trigger changed. */
    retriggered(binding: Binding, previous: ParsedTrigger): void;
    /** Iterates over bindings in insertion order. */
    [Symbol.iterator](): Iterator<Binding>;
}
/** Asserts `trigger` is a trigger the grammar accepts. `what` names the holder in the message. */
export declare function checkTrigger(trigger: unknown, what: string): asserts trigger is string;
/**
 * The trigger of each named binding: `defaults` with `overrides` on top. An omitted or `undefined` name keeps its
 * default; `null` turns that binding off. A name not in `defaults` is a typo and throws. Explicit `undefined`
 * keeps the default rather than erasing it.
 */
export declare function resolveTriggers<K extends string>(defaults: Readonly<Record<K, string>>, overrides?: {
    readonly [P in K]?: string | null | undefined;
}): Record<K, string | null>;
