// The calls to the compositor that answer later: `gnoblin.session` (the verbs that log out, suspend, power off or
// lock it) and `gnoblin.runtime` (`reloadConfig`). Each returns an `Operation` the shell settles with the answer.

import type { ErrorCode } from "gnoblin/errors";
import { Operation } from "gnoblin/operation";
import type { OperationHost } from "gnoblin/operation";
import { subscriber } from "gnoblin/session/events";
import type { RuntimeListener, SessionEvents, SessionListener, Subscribe } from "gnoblin/session/events";
import type {
    Accepted,
    Failure,
    Host,
    LockRequest,
    OperationEvent,
    ReloadResult,
    SessionStatus,
} from "gnoblin/session/types";

/** What `Calls` holds of an operation: the ways to settle it, whatever it will carry. */
interface Settler {
    succeed(value: unknown): boolean;
    fail(code: ErrorCode, message: string): boolean;
}

/** The calls the shell has made and the compositor has not finished, by ticket. */
export class Calls {
    #host: Host;
    #operations: OperationHost;
    #pending = new Map<number, Settler>();

    constructor(host: Host, operations: OperationHost) {
        this.#host = host;
        this.#operations = operations;
    }

    /** Starts `verb`. The returned operation settles when the compositor finishes it, or at once when it refuses. */
    begin<T>(verb: string, method: string): Operation<T> {
        let ticket = 0;
        const operation = new Operation<T>(this.#operations, this.#host.operationId(), method, () => {
            this.#pending.delete(ticket);
        });
        const answer = JSON.parse(this.#host.begin(verb)) as { ticket?: number; error?: Failure };
        if (answer.error !== undefined) {
            operation.fail(answer.error.code as ErrorCode, answer.error.message);
            return operation;
        }
        ticket = answer.ticket as number;
        this.#pending.set(ticket, operation as Settler);
        return operation;
    }

    /** Settles a pending call from the compositor's response. */
    finish(event: OperationEvent): void {
        const call = this.#pending.get(event.ticket);
        if (call === undefined) {
            // Another generation's call: the one that asked is gone, so nobody is waiting for the answer.
            return;
        }
        this.#pending.delete(event.ticket);
        if (event.ok) {
            call.succeed(event.value);
        } else {
            const failure = event.error as Failure;
            call.fail(failure.code as ErrorCode, failure.message);
        }
    }

    /** The connection is gone: nothing more arrives, so every call still waiting fails. */
    failAll(message: string): void {
        const waiting = [...this.#pending.values()];
        this.#pending.clear();
        for (const call of waiting) {
            call.fail("unavailable", message);
        }
    }

    /** Drops pending call tracking when this instance is retired. */
    clear(): void {
        this.#pending.clear();
    }
}

/** The session's state and its logout, power, suspend and lock operations. */
export class Session {
    #host: Host;
    #calls: Calls;
    #events: SessionEvents;
    /** `addListener` for `SessionListener.onReady` alone: `handler(session)`. Returns the detaching function. */
    readonly onReady: Subscribe<SessionListener, "onReady">;
    /** `addListener` for `SessionListener.onBell` alone: `handler(session, count)`. Returns the detaching function. */
    readonly onBell: Subscribe<SessionListener, "onBell">;
    /** `addListener` for `SessionListener.onReloadRequested` alone. Returns the detaching function. */
    readonly onReloadRequested: Subscribe<SessionListener, "onReloadRequested">;
    /** `addListener` for `SessionListener.onLockStateChanged` alone. Returns the detaching function. */
    readonly onLockStateChanged: Subscribe<SessionListener, "onLockStateChanged">;

    constructor(host: Host, calls: Calls, events: SessionEvents) {
        this.#host = host;
        this.#calls = calls;
        this.#events = events;
        this.onReady = subscriber(events.session, "onReady");
        this.onBell = subscriber(events.session, "onBell");
        this.onReloadRequested = subscriber(events.session, "onReloadRequested");
        this.onLockStateChanged = subscriber(events.session, "onLockStateChanged");
    }

    /** Attaches a `SessionListener`; adding the same listener twice has no effect. */
    addListener(listener: SessionListener): void {
        this.#events.session.add(listener);
    }

    /** Detaches a listener. */
    removeListener(listener: SessionListener): void {
        this.#events.session.remove(listener);
    }

    /** The session as the compositor last published it. Read on each call. */
    status(): SessionStatus {
        const text = this.#host.status();
        if (text === undefined) {
            return { state: "unavailable", lockAvailable: false, revision: 0 };
        }
        return { state: "running", ...(JSON.parse(text) as Omit<SessionStatus, "state">) };
    }

    /**
     * Ends the session. The compositor exits with the logout code, and the session wrapper returns to the login
     * manager.
     */
    logout(): Operation<Accepted> {
        return this.#calls.begin("logout", "session.logout");
    }

    /** Powers the machine off. The operation succeeds when the init system accepts. */
    poweroff(): Operation<Accepted> {
        return this.#calls.begin("poweroff", "session.poweroff");
    }
    /** Restarts the machine. */
    reboot(): Operation<Accepted> {
        return this.#calls.begin("reboot", "session.reboot");
    }

    /**
     * Suspends to RAM after locking the session. Can fail with `denied`, `unsupported`, `busy`, `unavailable` or
     * `timed_out`.
     */
    suspend(): Operation<Accepted> {
        return this.#calls.begin("suspend", "session.suspend");
    }

    /** Hibernates to disk. Can fail with `unsupported` when the machine reports it cannot. */
    hibernate(): Operation<Accepted> {
        return this.#calls.begin("hibernate", "session.hibernate");
    }

    /** Asks the compositor to lock the session. Read `status()` to check whether it is locked. */
    lock(): Operation<LockRequest> {
        return this.#calls.begin("lock", "session.lock");
    }
}

/** Runtime operations available to scripts. */
export class Runtime {
    #host: Host;
    #calls: Calls;
    #events: SessionEvents;
    /** `addListener` for `RuntimeListener.onConfigReloaded` alone: `handler(runtime, reload)`. Returns the detaching function. */
    readonly onConfigReloaded: Subscribe<RuntimeListener, "onConfigReloaded">;
    /** `addListener` for `RuntimeListener.onConfigReloadFailed` alone. Returns the detaching function. */
    readonly onConfigReloadFailed: Subscribe<RuntimeListener, "onConfigReloadFailed">;

    constructor(host: Host, calls: Calls, events: SessionEvents) {
        this.#host = host;
        this.#calls = calls;
        this.#events = events;
        this.onConfigReloaded = subscriber(events.runtime, "onConfigReloaded");
        this.onConfigReloadFailed = subscriber(events.runtime, "onConfigReloadFailed");
    }

    /** Attaches a `RuntimeListener`; adding the same listener twice has no effect. */
    addListener(listener: RuntimeListener): void {
        this.#events.runtime.add(listener);
    }

    /** Detaches a listener. */
    removeListener(listener: RuntimeListener): void {
        this.#events.runtime.remove(listener);
    }

    /**
     * Reloads the configuration. The operation succeeds when the compositor accepts it, or fails with
     * `config_rejected` and the reason. A failed reload keeps the current configuration; a successful reload replaces
     * this generation, so its callbacks end. Hear the outcome with a `RuntimeListener`.
     *
     * Throws while the configuration is loading.
     */
    reloadConfig(): Operation<ReloadResult> {
        if (this.#host.loading()) {
            throw new RangeError(
                "gnoblin.runtime.reloadConfig: the configuration is loading, and each reload would ask for another",
            );
        }
        return this.#calls.begin("reload-config", "runtime.reloadConfig");
    }
}
