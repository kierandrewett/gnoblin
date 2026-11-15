// `gnoblin.on`: the event bus. A handler is called for each `namespace.object.signal` the compositor raises, as a
// queued call under the usual budget; an unknown name throws and lists the near ones.

import type { Subscription } from "gnoblin/operation";
import { camel, nearMatches } from "gnoblin/session/names";
import type { Host, SignalEvent, SignalMessage } from "gnoblin/session/types";

/** The most handlers the bus holds, as the retired Lua host's. */
export const MAX_HANDLERS = 1024;

interface Handler {
    name: string;
    fn: (event: SignalEvent) => void;
    once: boolean;
    /** False once the handler was detached by a script, so a call already queued does not run. */
    active: boolean;
}

/** `gnoblin.on`: handlers for the schema's signals, by their `namespace.object.signal` names. */
export class EventBus {
    #host: Host;
    #known: Set<string> | null = null;
    #handlers: Handler[] = [];

    constructor(host: Host) {
        this.#host = host;
    }

    /** Every name `on` accepts, in schema order. */
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

    /** The schema spelling of a name: hyphens are underscores. An unknown name throws and lists the near ones. */
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

    on(name: string, fn: (event: SignalEvent) => void): Subscription {
        return this.#add(name, fn, false, "gnoblin.on");
    }

    once(name: string, fn: (event: SignalEvent) => void): Subscription {
        return this.#add(name, fn, true, "gnoblin.once");
    }

    /** Detaches a subscription, or every handler `fn` of the event `name`. */
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

    #remove(handler: Handler): void {
        handler.active = false;
        this.#detach(handler);
    }

    #detach(handler: Handler): void {
        const index = this.#handlers.indexOf(handler);
        if (index >= 0) {
            this.#handlers.splice(index, 1);
        }
    }

    /** One signal the compositor raised: each handler of its name is queued. */
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

    clear(): void {
        for (const handler of [...this.#handlers]) {
            this.#remove(handler);
        }
    }
}
