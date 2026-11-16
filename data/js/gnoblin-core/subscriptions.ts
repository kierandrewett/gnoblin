// Named subscriptions: `gnoblin.session.onBell(handler)`, `client.onTitleChanged(handler)`, `gnoblin.onClientCreated(handler)`.
//
// Every typed listener of the library (`SessionListener`, `ClientListener`, `OutputListener`, `CollectionListener`, ...)
// is a class whose methods are optional. Each method is also a named subscription on the objects that take the
// listener: `object.onXxx(handler)` attaches a listener that has that one method and returns the function that detaches
// it. This module is the one implementation of that. The members are declared by the types below, from the listener
// class, and defined at run time by `defineSubscriptions`, from the list of method names that sits beside the class.
// Nothing here is written out once per method.

/** Stops a subscription made by an `on<Method>` function. Calling it twice is harmless. */
export type Unsubscribe = () => void;

/** An object that takes listeners: `addListener` and `removeListener` of a `Session`, a `Client`, a `Collection`. */
export interface ListenerHost<L> {
    /** Attaches `listener`; one that is attached already is not attached again. */
    addListener(listener: L): void;
    /** Detaches `listener`; one that is not attached is ignored. */
    removeListener(listener: L): void;
}

/** The named subscription of one listener method: `handler` is the method, called with the object first. */
export type Subscribe<L, K extends keyof L> = (handler: NonNullable<L[K]>) => Unsubscribe;

/** One named subscription for every method of the listener `L`: the members `defineSubscriptions` puts on a class. */
export type Subscriptions<L> = { readonly [K in keyof L]-?: Subscribe<L, K> };

/**
 * The subscriptions of `L` under names with `Prefix` after the `on`: `Prefixed<"Client", ClientListener>` has
 * `onClientTitleChanged` for the method `onTitleChanged`. They are the members of an object that forwards to another
 * (`gnoblin.onClientTitleChanged` forwards to `gnoblin.clients`).
 */
export type Prefixed<Prefix extends string, L> = {
    readonly [K in keyof L as K extends `on${infer Event}` ? `on${Prefix}${Event}` : never]-?: Subscribe<L, K>;
};

/** The one implementation of every `on<Method>(handler)`: attaches a listener that has just that method. */
export function subscriber<L extends object, K extends keyof L & string>(
    target: ListenerHost<L>,
    method: K,
): Subscribe<L, K> {
    return (handler) => {
        const listener = { [method]: handler } as unknown as L;
        target.addListener(listener);
        return () => target.removeListener(listener);
    };
}

/** The name of the subscription that forwards `onTitleChanged` under `prefix` ("Client"): `onClientTitleChanged`. */
export function withPrefix(prefix: string): (method: string) => string {
    return (method) => `on${prefix}${method.slice(2)}`;
}

/** How `defineSubscriptions` names a member and finds the object that holds the listeners. */
export interface SubscriptionOptions<L> {
    /** The member's name for a listener method: the method's own name when not given. */
    name?(method: string): string;
    /** The object that takes the listener, given the one the member was called on: that object itself when not given. */
    target?(self: never): ListenerHost<L>;
}

/**
 * Defines `on<Method>(handler)` on `prototype` for each of `methods`, as methods that a class would have: not
 * enumerable, writable and configurable. A package that adds a method to a listener class (`onRequest` of
 * `ClientListener`) calls this for the new name, so the subscription exists wherever the listener is taken.
 */
export function defineSubscriptions<L extends object>(
    prototype: object,
    methods: readonly (keyof L & string)[],
    options: SubscriptionOptions<L> = {},
): void {
    for (const method of methods) {
        const name = options.name?.(method) ?? method;
        Object.defineProperty(prototype, name, {
            value(this: ListenerHost<L>, handler: unknown): Unsubscribe {
                if (typeof handler !== "function") {
                    throw new TypeError(`${name}: the handler must be a function`);
                }
                const target = options.target === undefined ? this : options.target(this as never);
                return subscriber<L, typeof method>(
                    target,
                    method,
                )(handler as unknown as NonNullable<L[typeof method]>);
            },
            writable: true,
            configurable: true,
        });
    }
}
