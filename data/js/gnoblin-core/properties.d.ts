type NodeId = number;
/** What the engine needs of the shell. */
export interface BindingHost {
    /** The value of a tree property now, without recording the read. */
    read(node: NodeId, key: string): unknown;
    /** Reports the failure of one binding. `site` names it. The other bindings are not affected. */
    report(site: string, error: unknown): void;
}
/** One tree property a binding read, and the value it saw. */
interface HostDep {
    node: NodeId;
    key: string;
    seen: unknown;
}
interface Binding {
    owner: object;
    /** The property written with the result, or null for a watch: `compute` is an effect and its result is dropped. */
    property: string | null;
    site: string;
    compute: () => unknown;
    reads: Map<string, HostDep>;
    signals: Map<object, Set<string>>;
    dirty: boolean;
    disposed: boolean;
    /** The message of the last failure, so the same one is reported once and not every frame. */
    failure: string | null;
}
/** The most times one frame runs bindings that the previous pass made dirty. */
export declare const MAX_PASSES = 8;
/** Gives the module its host. The shell calls it once per generation before any script runs. */
export declare function attachBindings(next: BindingHost): void;
/** Records that the running binding read tree property `key` of `node` and saw `value`. */
export declare function noteHostRead(node: NodeId, key: string, value: unknown): void;
/** Records that the running binding read what `name` of `source` stands for (a collection's membership). */
export declare function noteSignal(source: object, name: string): void;
/** Tells the engine that `name` of `source` changed: every binding that read it runs again. */
export declare function raiseSignal(source: object, name: string): void;
/** Tells the engine a script wrote `value` to tree property `key` of `node`. */
export declare function noteHostWrite(node: NodeId, key: string, value: unknown): void;
/**
 * Tells the engine the compositor changed `nodes`: a binding that read one of their properties and sees another
 * value runs again.
 */
export declare function noteHostTouched(nodes: readonly NodeId[]): void;
/** Ends every binding whose owner is `owner`: the owner was destroyed or left the shell. */
export declare function disposeBindingsOf(owner: object): void;
/**
 * Runs the bindings that are dirty, once each, and again for the ones their writes made dirty, up to `MAX_PASSES`
 * passes. A binding still dirty after that is in a cycle (it and another feed each other): it is left as it is and
 * reported once, with the tree properties it reads.
 */
export declare function flushBindings(): void;
/** What `bind` returns: the way to end the binding. */
export declare class PropertyBinding {
    #private;
    constructor(binding: Binding);
    /** The name of the property the binding writes. */
    get property(): string;
    /** Whether the binding still follows its reads: false after `dispose` and after its owner is destroyed. */
    get alive(): boolean;
    /** Ends the binding. The property keeps the last value written. Calling it twice is harmless. */
    dispose(): void;
}
/** The bindings of one object: `owner.properties`. */
export declare class PropertyBindings {
    #private;
    constructor(owner: object, label: () => string);
    /**
     * Makes `property` follow `compute`: it is set to the result now and again when a value `compute` read changes (a
     * node property or collection membership). `compute` must not write, spawn or wait. A second binding of the same
     * property replaces the first. A failure of `compute` or the write is reported once; the binding keeps following
     * its reads.
     */
    bind(property: string, compute: () => unknown): PropertyBinding;
}
/**
 * Runs `effect` now and again whenever what it read changes, with the same rules as a binding but no property: the
 * effect does its own writing. A failure is reported under `site`, once. The host uses it to keep a mirror of a
 * workspace in step; `owner` ends it when destroyed.
 */
export declare function watch(owner: object, site: string, effect: () => void): PropertyBinding;
/** Runs `body` without recording reads, for the reads an effect makes of what it also writes. */
export declare function untracked<T>(body: () => T): T;
/** The `properties` object of `owner`; `label` names the owner in errors. The same object every time. */
export declare function propertiesOf(owner: object, label: () => string): PropertyBindings;
export {};
