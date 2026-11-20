import type { Runtime, Session } from "gnoblin-core/session/calls";
import type { Host, LockState, SignalMessage } from "gnoblin-core/session/types";
/** Observes the session: it came up, a terminal rang the bell, a reload was asked for, the lock changed. */
export declare class SessionListener {
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
export declare class RuntimeListener {
    /** A reload was accepted. */
    onConfigReloaded?(runtime: Runtime, reload: ConfigReloaded): void;
    /** A reload was refused. */
    onConfigReloadFailed?(runtime: Runtime, failure: ConfigReloadFailed): void;
}
/** The listeners of one object, and the rules every kind of listener keeps. */
export declare class Listeners<L extends object> {
    #private;
    constructor(site: string);
    /** Attaches `listener`; one that is attached already is not attached again. */
    add(listener: L): void;
    /** Detaches `listener`; a call already queued for it does not run. */
    remove(listener: L): void;
    /** The listeners now attached, in the order they were added. */
    get all(): readonly L[];
    /** Detaches every listener. */
    clear(): void;
}
/** The names of the methods of `SessionListener`: the subscriptions of `gnoblin.session`. */
export declare const SESSION_EVENTS: readonly ["onReady", "onBell", "onReloadRequested", "onLockStateChanged"];
/** The names of the methods of `RuntimeListener`: the subscriptions of `gnoblin.runtime`. */
export declare const RUNTIME_EVENTS: readonly ["onConfigReloaded", "onConfigReloadFailed"];
/** Hands the compositor's session and configuration signals to the listeners of `gnoblin.session` and `gnoblin.runtime`. */
export declare class SessionEvents {
    #private;
    /** The listeners of `gnoblin.session`. */
    readonly session: Listeners<SessionListener>;
    /** The listeners of `gnoblin.runtime`. */
    readonly runtime: Listeners<RuntimeListener>;
    constructor(host: Host, session: () => Session, runtime: () => Runtime);
    /** Queues the listener method a signal names, for every listener that has it. A signal nobody listens to is dropped. */
    deliver(message: SignalMessage): void;
    /** Detaches every listener, so the calls queued for them do not run. */
    clear(): void;
}
