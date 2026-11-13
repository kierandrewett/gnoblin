// Bindings: the declared shape of an input binding and the collection a layout keeps them in.
// The trigger matcher, hit-testing and input capture are host services that are not wired yet, so a
// Binding here is data plus the four lifecycle hooks of owner-spec.md section 11. `onStart` declines by
// default, which means a binding nobody subclassed never captures input once the host wires it.

/** What the host passes to a hook. The fields are the host's; the library never reads them. */
export interface BindingEvent {
    readonly timestamp: number;
    readonly [field: string]: unknown;
}

export interface BindingOptions {
    /** Unique among the bindings of one collection. */
    id: string;
    /** The chord, gesture or drag spelling, for example `Super+MouseLeft+Drag`. */
    trigger: string;
    enabled?: boolean;
    /** Higher wins when two bindings match one input. */
    priority?: number;
}

export class Binding {
    readonly id: string;
    trigger: string;
    enabled: boolean;
    priority: number;

    constructor(options: BindingOptions) {
        if (typeof options?.id !== "string" || options.id === "") {
            throw new TypeError("Binding id must be a non-empty string");
        }
        checkTrigger(options.trigger, `Binding "${options.id}" trigger`);
        if (options.priority !== undefined && !Number.isFinite(options.priority)) {
            throw new RangeError(`Binding "${options.id}" priority must be a finite number`);
        }
        this.id = options.id;
        this.trigger = options.trigger;
        this.enabled = options.enabled ?? true;
        this.priority = options.priority ?? 0;
    }

    /** Returns the state object for this gesture, or false to decline it. */
    onStart(_event: BindingEvent): object | false {
        return false;
    }

    onUpdate(_event: BindingEvent, _state: object): void {}

    onEnd(_event: BindingEvent, _state: object): void {}

    onCancel(_event: BindingEvent, _state: object): void {}
}

/** The bindings of one layout, by id, in the order they were added. */
export class BindingCollection implements Iterable<Binding> {
    private readonly byId = new Map<string, Binding>();

    get size(): number {
        return this.byId.size;
    }

    add(binding: Binding): Binding {
        if (this.byId.has(binding.id)) {
            throw new Error(`a binding with id "${binding.id}" is already in this collection`);
        }
        this.byId.set(binding.id, binding);
        return binding;
    }

    get(id: string): Binding | undefined {
        return this.byId.get(id);
    }

    has(id: string): boolean {
        return this.byId.has(id);
    }

    remove(id: string): boolean {
        return this.byId.delete(id);
    }

    [Symbol.iterator](): Iterator<Binding> {
        return this.byId.values();
    }
}

/** A trigger spelling: `+`-separated, no empty part. The host owns the grammar beyond that. */
export function checkTrigger(trigger: unknown, what: string): asserts trigger is string {
    if (typeof trigger !== "string" || trigger === "" || trigger.split("+").some((part) => part === "")) {
        throw new TypeError(`${what} must be a trigger such as "Super+Return", got ${String(trigger)}`);
    }
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
