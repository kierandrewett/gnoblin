import { Operation } from "gnoblin-core/operation";
import type { OperationHost } from "gnoblin-core/operation";
import type { RuntimeListener, SessionEvents, SessionListener } from "gnoblin-core/session/events";
import type { Subscriptions } from "gnoblin-core/subscriptions";
import type { Accepted, Host, LockRequest, OperationEvent, ReloadResult, SessionStatus } from "gnoblin-core/session/types";
/** The calls the shell has made and the compositor has not finished, by ticket. */
export declare class Calls {
    #private;
    constructor(host: Host, operations: OperationHost);
    /** Starts `verb`. The returned operation settles when the compositor finishes it, or at once when it refuses. */
    begin<T>(verb: string, method: string): Operation<T>;
    /** Settles a pending call from the compositor's response. */
    finish(event: OperationEvent): void;
    /** The connection is gone: nothing more arrives, so every call still waiting fails. */
    failAll(message: string): void;
    /** Drops pending call tracking when this instance is retired. */
    clear(): void;
}
/** The session's state and its logout, power, suspend and lock operations. */
export declare class Session {
    #private;
    constructor(host: Host, calls: Calls, events: SessionEvents);
    /** Attaches a `SessionListener`; adding the same listener twice has no effect. */
    addListener(listener: SessionListener): void;
    /** Detaches a listener. */
    removeListener(listener: SessionListener): void;
    /** The session as the compositor last published it. Read on each call. */
    status(): SessionStatus;
    /**
     * Ends the session. The compositor exits with the logout code, and the session wrapper returns to the login
     * manager.
     */
    logout(): Operation<Accepted>;
    /** Powers the machine off. The operation succeeds when the init system accepts. */
    poweroff(): Operation<Accepted>;
    /** Restarts the machine. */
    reboot(): Operation<Accepted>;
    /**
     * Suspends to RAM after locking the session. Can fail with `denied`, `unsupported`, `busy`, `unavailable` or
     * `timed_out`.
     */
    suspend(): Operation<Accepted>;
    /** Hibernates to disk. Can fail with `unsupported` when the machine reports it cannot. */
    hibernate(): Operation<Accepted>;
    /** Asks the compositor to lock the session. Read `status()` to check whether it is locked. */
    lock(): Operation<LockRequest>;
}
export interface Session extends Subscriptions<SessionListener> {
}
/** Runtime operations available to scripts. */
export declare class Runtime {
    #private;
    constructor(host: Host, calls: Calls, events: SessionEvents);
    /** Attaches a `RuntimeListener`; adding the same listener twice has no effect. */
    addListener(listener: RuntimeListener): void;
    /** Detaches a listener. */
    removeListener(listener: RuntimeListener): void;
    /**
     * Reloads the configuration. The operation succeeds when the compositor accepts it, or fails with
     * `config_rejected` and the reason. A failed reload keeps the current configuration; a successful reload replaces
     * this generation, so its callbacks end. Hear the outcome with a `RuntimeListener`.
     *
     * Throws while the configuration is loading.
     */
    reloadConfig(): Operation<ReloadResult>;
}
export interface Runtime extends Subscriptions<RuntimeListener> {
}
