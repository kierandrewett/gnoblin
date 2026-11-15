// `gnoblin.on`: handlers for compositor signals, called as queued script work.

import type { Subscription } from "gnoblin/operation";
import { camel, nearMatches } from "gnoblin/session/names";
import type { Host, SignalEvent, SignalMessage } from "gnoblin/session/types";

/** The maximum number of handlers the bus can hold. */
export const MAX_HANDLERS = 1024;

/** Tracks a signal handler and whether it still accepts queued calls. */
interface Handler {
    name: string;
    fn: (event: SignalEvent) => void;
    once: boolean;
    /** False once the handler was detached by a script, so a call already queued does not run. */
    active: boolean;
}

/** Handlers for schema signals, keyed by `namespace.object.signal`. */
export class EventBus {
    #host: Host;
    #known: Set<string> | null = null;
    #handlers: Handler[] = [];

    constructor(host: Host) {
        this.#host = host;
    }

    /** Returns every accepted signal name in schema order. */
    get names(): string[] {
        return [...this.#signals()];
    }

    #signals(): Set<string> {
        if (this.#known === null) {
            const table = JSON.parse(this.#host.signals()) as { name: string }[];
            this.#known = new Set(table.map((entry) => entry.name));
        }
        return this.#known;
    }

    /** Returns the schema name for an event, or throws with suggestions for an unknown name. */
    canonical(name: string, site: string): string {
        if (typeof name !== "string") {
            throw new TypeError(`${site}: the event name must be a string`);
        }
        const schema = name.replaceAll("-", "_");
        const known = this.#signals();
        if (!known.has(schema)) {
            const near = nearMatches(schema, [...known]);
            const hint = near.length > 0 ? `; near matches: ${near.join(", ")}` : "";
            throw new TypeError(
                `${site}: no schema signal named \`${schema}\`; expected \`namespace.object.signal\`${hint}`,
            );
        }
        return schema;
    }

    /** Validates a handler and stores it for future signals. */
    #add(name: string, fn: (event: SignalEvent) => void, once: boolean, site: string): Subscription {
        const schema = this.canonical(name, site);
        if (typeof fn !== "function") {
            throw new TypeError(`${site}: needs a function to call`);
        }
        if (this.#handlers.length >= MAX_HANDLERS) {
            throw new RangeError(`${site}: ${MAX_HANDLERS} event handlers are already attached`);
        }
        const handler: Handler = { name: schema, fn, once, active: true };
        this.#handlers.push(handler);
        return { unsubscribe: () => this.#remove(handler) };
    }

    /** Calls `fn` for each matching signal until its subscription is removed. */
    on(name: string, fn: (event: SignalEvent) => void): Subscription {
        return this.#add(name, fn, false, "gnoblin.on");
    }

    /** Calls `fn` for the next matching signal, then removes its subscription. */
    once(name: string, fn: (event: SignalEvent) => void): Subscription {
        return this.#add(name, fn, true, "gnoblin.once");
    }

    /** Removes a subscription or every matching handler for an event name. */
    off(target: Subscription | string, fn?: (event: SignalEvent) => void): void {
        if (typeof target === "string") {
            const schema = this.canonical(target, "gnoblin.off");
            for (const handler of this.#handlers.filter(
                (h) => h.name === schema && (fn === undefined || h.fn === fn),
            )) {
                this.#remove(handler);
            }
        } else if (target !== null && typeof target === "object" && typeof target.unsubscribe === "function") {
            target.unsubscribe();
        } else {
            throw new TypeError("gnoblin.off: expects a subscription, or an event name and the handler");
        }
    }

    /** Marks a handler inactive and removes it from the list. */
    #remove(handler: Handler): void {
        handler.active = false;
        this.#detach(handler);
    }

    /** Removes a handler without preventing an already queued call. */
    #detach(handler: Handler): void {
        const index = this.#handlers.indexOf(handler);
        if (index >= 0) {
            this.#handlers.splice(index, 1);
        }
    }

    /** Queues each handler for a signal and maps its fields to camelCase. */
    deliver(message: SignalMessage): void {
        const event: Record<string, unknown> = { name: message.name, object: message.object };
        for (const [field, value] of Object.entries(message.fields)) {
            event[camel(field)] = value;
        }
        for (const handler of this.#handlers.filter((h) => h.name === message.name)) {
            if (handler.once) {
                // Out of the list now, so a second signal in the same frame does not queue it again.
                this.#detach(handler);
            }
            this.#host.schedule(`gnoblin.on(${message.name})`, () => {
                if (handler.active) {
                    handler.fn(event as unknown as SignalEvent);
                }
            });
        }
    }

    /** Removes all handlers and prevents their queued calls from running. */
    clear(): void {
        for (const handler of [...this.#handlers]) {
            this.#remove(handler);
        }
    }
}
