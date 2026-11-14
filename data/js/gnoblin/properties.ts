// Property bindings: `node.properties.bind(name, compute)`, a value that follows what it was computed from.
//
//     client.properties.bind("opacity", () => Math.max(0.25, 1 - client.position.distanceTo(camera.position) / 2500));
//
// While `compute` runs, every tree property it reads through the host's getters and every collection it reads is
// recorded. The binding runs again when one of those changes and at no other time: the shell tells this module which
// nodes the compositor changed (`noteHostTouched`), which tree properties a script wrote (`noteHostWrite`) and which
// collections gained or lost a member (`raiseSignal`), and a binding whose reads did not change is not looked at. A
// frame runs each dirty binding once (`flushBindings`); a write one binding makes can make another dirty, so the frame
// repeats up to `MAX_PASSES` times and stops there, naming what was still changing.
//
// This module owns the engine and the handle a script holds. It does not own the host's getters: the shell wraps
// them and calls the `note*` functions. Plain script fields (`layout.radius`) are not tracked: the specification
// (section 16) says an untracked value must not be promised.

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
export const MAX_PASSES = 8;

let host: BindingHost | null = null;
const live = new Set<Binding>();
const byNode = new Map<NodeId, Set<Binding>>();
const bySignal = new WeakMap<object, Map<string, Set<Binding>>>();
const byOwner = new WeakMap<object, Map<string, Binding>>();
/** The binding whose `compute` is running: reads are recorded for it. */
let collecting: Binding | null = null;
/** The binding that is writing its result: the write is not a reason to run it again. */
let applying: Binding | null = null;

/** Gives the module its host. The shell calls it once per generation before any script runs. */
export function attachBindings(next: BindingHost): void {
    host = next;
}

function same(a: unknown, b: unknown): boolean {
    if (Array.isArray(a) && Array.isArray(b)) {
        return a.length === b.length && a.every((part, at) => part === b[at]);
    }
    return a === b;
}

function depKey(node: NodeId, key: string): string {
    return `${node}:${key}`;
}

/** Records that the running binding read tree property `key` of `node` and saw `value`. */
export function noteHostRead(node: NodeId, key: string, value: unknown): void {
    if (collecting === null) {
        return;
    }
    const id = depKey(node, key);
    if (!collecting.reads.has(id)) {
        collecting.reads.set(id, { node, key, seen: value });
        let set = byNode.get(node);
        if (set === undefined) {
            set = new Set();
            byNode.set(node, set);
        }
        set.add(collecting);
    }
}

/** Records that the running binding read what `name` of `source` stands for (a collection's membership). */
export function noteSignal(source: object, name: string): void {
    if (collecting === null) {
        return;
    }
    let names = collecting.signals.get(source);
    if (names === undefined) {
        names = new Set();
        collecting.signals.set(source, names);
    }
    names.add(name);
    let table = bySignal.get(source);
    if (table === undefined) {
        table = new Map();
        bySignal.set(source, table);
    }
    let set = table.get(name);
    if (set === undefined) {
        set = new Set();
        table.set(name, set);
    }
    set.add(collecting);
}

/** Tells the engine that `name` of `source` changed: every binding that read it runs again. */
export function raiseSignal(source: object, name: string): void {
    for (const binding of bySignal.get(source)?.get(name) ?? []) {
        if (binding !== applying) {
            binding.dirty = true;
        }
    }
}

/** Tells the engine a script wrote `value` to tree property `key` of `node`. */
export function noteHostWrite(node: NodeId, key: string, value: unknown): void {
    for (const binding of byNode.get(node) ?? []) {
        const dep = binding.reads.get(depKey(node, key));
        if (dep === undefined || same(dep.seen, value)) {
            continue;
        }
        if (binding === applying) {
            dep.seen = value;
        } else {
            binding.dirty = true;
        }
    }
}

/** Tells the engine the compositor changed `nodes`: a binding that read one of their properties and sees another value runs again. */
export function noteHostTouched(nodes: readonly NodeId[]): void {
    if (host === null) {
        return;
    }
    for (const node of nodes) {
        for (const binding of byNode.get(node) ?? []) {
            if (binding.dirty) {
                continue;
            }
            for (const dep of binding.reads.values()) {
                if (dep.node === node && !same(host.read(node, dep.key), dep.seen)) {
                    binding.dirty = true;
                    break;
                }
            }
        }
    }
}

function forget(binding: Binding): void {
    for (const dep of binding.reads.values()) {
        const set = byNode.get(dep.node);
        set?.delete(binding);
        if (set?.size === 0) {
            byNode.delete(dep.node);
        }
    }
    binding.reads.clear();
    for (const [source, names] of binding.signals) {
        for (const name of names) {
            bySignal.get(source)?.get(name)?.delete(binding);
        }
    }
    binding.signals.clear();
}

/** The key an owner's table holds a binding under: its property, or for a watch a key no property name can be. */
function ownerKey(binding: Binding): string {
    return binding.property ?? `\0${binding.site}`;
}

function dispose(binding: Binding): void {
    if (binding.disposed) {
        return;
    }
    binding.disposed = true;
    forget(binding);
    live.delete(binding);
    const own = byOwner.get(binding.owner);
    const key = ownerKey(binding);
    if (own?.get(key) === binding) {
        own.delete(key);
    }
}

/** Ends every binding whose owner is `owner`: the owner was destroyed or left the shell. */
export function disposeBindingsOf(owner: object): void {
    for (const binding of [...(byOwner.get(owner)?.values() ?? [])]) {
        dispose(binding);
    }
}

function writable(owner: object, property: string): boolean {
    for (let at: object | null = owner; at !== null; at = Object.getPrototypeOf(at)) {
        const found = Object.getOwnPropertyDescriptor(at, property);
        if (found !== undefined) {
            return typeof found.set === "function" || found.writable === true;
        }
    }
    return false;
}

function isDead(owner: object): boolean {
    return "alive" in owner && owner.alive === false;
}

function fail(binding: Binding, error: unknown): void {
    const message = error instanceof Error ? error.message : String(error);
    if (binding.failure !== message) {
        binding.failure = message;
        host?.report(binding.site, error);
    }
}

function run(binding: Binding): void {
    binding.dirty = false;
    if (isDead(binding.owner)) {
        dispose(binding);
        return;
    }
    forget(binding);
    let value: unknown;
    const outer = collecting;
    const writer = applying;
    collecting = binding;
    // A watch writes as it runs; what it writes is not a reason to run it again.
    applying = binding.property === null ? binding : applying;
    try {
        value = binding.compute();
    } catch (error) {
        fail(binding, error);
        return;
    } finally {
        collecting = outer;
        applying = writer;
    }
    if (binding.property === null) {
        binding.failure = null;
        return;
    }
    applying = binding;
    try {
        if (!writable(binding.owner, binding.property)) {
            throw new TypeError(`${binding.property} is not a writable property`);
        }
        Reflect.set(binding.owner, binding.property, value);
        binding.failure = null;
    } catch (error) {
        fail(binding, error);
    } finally {
        applying = null;
    }
}

/**
 * Runs the bindings that are dirty, once each, and again for the ones their writes made dirty, up to `MAX_PASSES`
 * passes. A binding still dirty after that is in a cycle (it and another feed each other): it is left as it is and
 * reported once, with the tree properties it reads.
 */
export function flushBindings(): void {
    for (let pass = 0; pass < MAX_PASSES; pass++) {
        const due = [...live].filter((binding) => binding.dirty);
        if (due.length === 0) {
            return;
        }
        for (const binding of due) {
            if (!binding.disposed) {
                run(binding);
            }
        }
    }
    for (const binding of live) {
        if (binding.dirty) {
            binding.dirty = false;
            const reads = [...binding.reads.values()].map((dep) => `#${dep.node}.${dep.key}`).join(", ");
            fail(
                binding,
                new Error(
                    `still changing after ${MAX_PASSES} passes in one frame (it reads ${reads || "nothing"}); ` +
                        "a binding that writes what another reads, which writes what it reads, never settles",
                ),
            );
        }
    }
}

/** What `bind` returns: the way to end the binding. */
export class PropertyBinding {
    readonly #binding: Binding;

    constructor(binding: Binding) {
        this.#binding = binding;
    }

    /** The name of the property the binding writes. */
    get property(): string {
        return this.#binding.property ?? "";
    }

    /** Whether the binding still follows its reads: false after `dispose` and after its owner is destroyed. */
    get alive(): boolean {
        return !this.#binding.disposed && !isDead(this.#binding.owner);
    }

    /** Ends the binding. The property keeps the last value written. Calling it twice is harmless. */
    dispose(): void {
        dispose(this.#binding);
    }
}

/** The bindings of one object: `owner.properties`. */
export class PropertyBindings {
    readonly #owner: object;
    readonly #label: () => string;

    constructor(owner: object, label: () => string) {
        this.#owner = owner;
        this.#label = label;
    }

    /**
     * Makes `property` of the owner follow `compute`: it is set to the result now and again when anything `compute`
     * read changes (a property of a node, or a collection gaining or losing a member). `compute` must not write, spawn
     * or wait. A second binding of the same property replaces the first. A failure of `compute` or of the write is
     * reported once and the binding goes on following its reads.
     */
    bind(property: string, compute: () => unknown): PropertyBinding {
        if (typeof property !== "string" || property === "") {
            throw new TypeError(`${this.#label()}.properties.bind: the property name must be a non-empty string`);
        }
        if (typeof compute !== "function") {
            throw new TypeError(
                `${this.#label()}.properties.bind(${property}): the second argument must be a function`,
            );
        }
        if (isDead(this.#owner)) {
            throw new TypeError(`${this.#label()}.properties.bind(${property}): the object was destroyed`);
        }
        let own = byOwner.get(this.#owner);
        if (own === undefined) {
            own = new Map();
            byOwner.set(this.#owner, own);
        }
        const earlier = own.get(property);
        if (earlier !== undefined) {
            dispose(earlier);
        }
        const binding: Binding = {
            owner: this.#owner,
            property,
            site: `${this.#label()}.${property}`,
            compute,
            reads: new Map(),
            signals: new Map(),
            dirty: false,
            disposed: false,
            failure: null,
        };
        own.set(property, binding);
        live.add(binding);
        run(binding);
        return new PropertyBinding(binding);
    }
}

/**
 * Runs `effect` now and again whenever what it read changes, with the same rules as a binding but no property: the
 * effect does its own writing. A failure is reported under `site`, once. The host uses it to keep a mirror of a
 * workspace in step; `owner` ends it when destroyed.
 */
export function watch(owner: object, site: string, effect: () => void): PropertyBinding {
    const binding: Binding = {
        owner,
        property: null,
        site,
        compute: effect,
        reads: new Map(),
        signals: new Map(),
        dirty: false,
        disposed: false,
        failure: null,
    };
    let own = byOwner.get(owner);
    if (own === undefined) {
        own = new Map();
        byOwner.set(owner, own);
    }
    const earlier = own.get(ownerKey(binding));
    if (earlier !== undefined) {
        dispose(earlier);
    }
    own.set(ownerKey(binding), binding);
    live.add(binding);
    run(binding);
    return new PropertyBinding(binding);
}

/** Runs `body` without recording reads, for the reads an effect makes of what it also writes. */
export function untracked<T>(body: () => T): T {
    const outer = collecting;
    collecting = null;
    try {
        return body();
    } finally {
        collecting = outer;
    }
}

const handles = new WeakMap<object, PropertyBindings>();

/** The `properties` object of `owner`; `label` names the owner in errors. The same object every time. */
export function propertiesOf(owner: object, label: () => string): PropertyBindings {
    let found = handles.get(owner);
    if (found === undefined) {
        found = new PropertyBindings(owner, label);
        handles.set(owner, found);
    }
    return found;
}
