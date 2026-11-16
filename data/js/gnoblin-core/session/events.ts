// The session's and the configuration's events, as typed listeners: `gnoblin.session.addListener(new SessionListener())`
// and `gnoblin.runtime.addListener(new RuntimeListener())`. Like every listener of the library, a method runs as a
// queued call under the listener budget, a throw is reported once and the next listener still runs, and a listener
// that is added twice is called once. A reload detaches every listener the old file attached.

import type { Runtime, Session } from "gnoblin-core/session/calls";
import { camel } from "gnoblin-core/session/names";
import type { Host, LockState, SignalMessage } from "gnoblin-core/session/types";

/** Observes the session: it came up, a terminal rang the bell, a reload was asked for, the lock changed. */
export class SessionListener {
    /** The session is up: the compositor's first announcement to the shell. */
    onReady?(session: Session): void;
    /** A client rang the terminal bell. `count` is how many bells the session has rung. */
    onBell?(session: Session, count: number): void;
    /** Somebody asked for a reload of the shell; `generation` is the generation that is running. */
    onReloadRequested?(session: Session, generation: number): void;
    /**
     * The lock changed state. Only `locked` confirms that the lock took. `revision` counts the changes the session
     * has published.
     */
    onLockStateChanged?(session: Session, state: LockState, revision: number): void;
}

/** A reload the compositor accepted: the new configuration is in force. */
export interface ConfigReloaded {
    /** The configuration file the reload read. */
    readonly path: string;
    /** The number the compositor gave the committed document. */
    readonly revision: number;
    /** The problems of the sections the reload dropped, one string each. Empty when every section validated. */
    readonly warnings: readonly string[];
    /** The settings accepted without effect, one sentence each: `restart required`, or `no effect`. */
    readonly notApplied: readonly string[];
    /** How many reloads the compositor has accepted. */
    readonly sequence: number;
    /** The shell's monotonic clock when it happened. Not a wall clock. */
    readonly time: number;
}

/** A reload the compositor refused: the old configuration stays. */
export interface ConfigReloadFailed {
    /** The configuration file the reload read. */
    readonly path: string;
    /** Why: the first validation or evaluation failure. */
    readonly error: string;
    /** How many reloads the compositor has refused. */
    readonly sequence: number;
    /** The shell's monotonic clock when it happened. Not a wall clock. */
    readonly time: number;
}

/** Observes the configuration reloads of `gnoblin.runtime`. */
export class RuntimeListener {
    /** A reload was accepted. */
    onConfigReloaded?(runtime: Runtime, reload: ConfigReloaded): void;
    /** A reload was refused. */
    onConfigReloadFailed?(runtime: Runtime, failure: ConfigReloadFailed): void;
}

/** The listeners of one object, and the rules every kind of listener keeps. */
export class Listeners<L extends object> {
    readonly #site: string;
    #list: L[] = [];

    constructor(site: string) {
        this.#site = site;
    }

    /** Attaches `listener`; one that is attached already is not attached again. */
    add(listener: L): void {
        if (listener === null || typeof listener !== "object") {
            throw new TypeError(`${this.#site}.addListener: the listener must be an object`);
        }
        for (const [name, member] of Object.entries(listener)) {
            if (name.startsWith("on") && member !== undefined && typeof member !== "function") {
                throw new TypeError(`${this.#site}.addListener: ${name} must be a function`);
            }
        }
        if (!this.#list.includes(listener)) {
            this.#list.push(listener);
        }
    }

    /** Detaches `listener`; a call already queued for it does not run. */
    remove(listener: L): void {
        const at = this.#list.indexOf(listener);
        if (at >= 0) {
            this.#list.splice(at, 1);
        }
    }

    /** The listeners now attached, in the order they were added. */
    get all(): readonly L[] {
        return this.#list;
    }

    /** Detaches every listener. */
    clear(): void {
        this.#list = [];
    }
}

/** Stops a subscription made by an `on<Method>` function. Calling it twice is harmless. */
export type Unsubscribe = () => void;

/** The named subscription of one listener method: `handler` is the method, called with the object first. */
export type Subscribe<L, K extends keyof L> = (handler: NonNullable<L[K]>) => Unsubscribe;

/** The one implementation of every `on<Method>(handler)`: a listener that has just that method. */
export function subscriber<L extends object, K extends keyof L & string>(
    listeners: Listeners<L>,
    method: K,
): Subscribe<L, K> {
    return (handler) => {
        const listener = { [method]: handler } as unknown as L;
        listeners.add(listener);
        return () => listeners.remove(listener);
    };
}

/** The fields of a signal, named the way a script reads them. */
function fieldsOf(message: SignalMessage): Record<string, unknown> {
    const fields: Record<string, unknown> = {};
    for (const [field, value] of Object.entries(message.fields)) {
        fields[camel(field)] = value;
    }
    return fields;
}

/** Hands the compositor's session and configuration signals to the listeners of `gnoblin.session` and `gnoblin.runtime`. */
export class SessionEvents {
    readonly #host: Host;
    readonly #session: () => Session;
    readonly #runtime: () => Runtime;
    /** The listeners of `gnoblin.session`. */
    readonly session = new Listeners<SessionListener>("gnoblin.session");
    /** The listeners of `gnoblin.runtime`. */
    readonly runtime = new Listeners<RuntimeListener>("gnoblin.runtime");

    constructor(host: Host, session: () => Session, runtime: () => Runtime) {
        this.#host = host;
        this.#session = session;
        this.#runtime = runtime;
    }

    /** Queues the listener method a signal names, for every listener that has it. A signal nobody listens to is dropped. */
    deliver(message: SignalMessage): void {
        const fields = fieldsOf(message);
        switch (message.name) {
            case "gnoblin.session.ready":
                this.#call(this.session, "onReady", this.#session(), []);
                break;
            case "gnoblin.session.bell":
                this.#call(this.session, "onBell", this.#session(), [fields.count]);
                break;
            case "gnoblin.session.reload_requested":
                this.#call(this.session, "onReloadRequested", this.#session(), [fields.generation]);
                break;
            case "gnoblin.session.lock_state_changed":
                this.#call(this.session, "onLockStateChanged", this.#session(), [fields.state, fields.revision]);
                break;
            case "gnoblin.config.reloaded":
                this.#call(this.runtime, "onConfigReloaded", this.#runtime(), [
                    {
                        path: fields.path,
                        revision: fields.revision,
                        warnings: fields.warnings,
                        notApplied: fields.notApplied,
                        sequence: fields.sequence,
                        time: fields.time,
                    },
                ]);
                break;
            case "gnoblin.config.reload_failed":
                this.#call(this.runtime, "onConfigReloadFailed", this.#runtime(), [
                    { path: fields.path, error: fields.error, sequence: fields.sequence, time: fields.time },
                ]);
                break;
        }
    }

    #call<L extends object>(listeners: Listeners<L>, method: string, origin: object, args: unknown[]): void {
        for (const listener of listeners.all) {
            const member = (listener as Record<string, unknown>)[method];
            if (typeof member !== "function") {
                continue;
            }
            this.#host.schedule(`${listener.constructor.name}.${method}`, () => {
                // A listener detached since the signal arrived hears nothing.
                if (listeners.all.includes(listener)) {
                    member.call(listener, origin, ...args);
                }
            });
        }
    }

    /** Detaches every listener, so the calls queued for them do not run. */
    clear(): void {
        this.session.clear();
        this.runtime.clear();
    }
}
