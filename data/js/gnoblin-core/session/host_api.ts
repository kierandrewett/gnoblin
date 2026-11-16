// Script-facing session, process and event APIs backed by functions supplied by the shell.

import { Operation, Timers } from "gnoblin-core/operation";
import type { OperationHost } from "gnoblin-core/operation";
import { ProcessTable } from "gnoblin-core/process";
import { Calls, Runtime, Session } from "gnoblin-core/session/calls";
import { SessionEvents } from "gnoblin-core/session/events";
import { find } from "gnoblin-core/session/find";
import type { FindSource, Scalar } from "gnoblin-core/session/find";
import { FocusHistory } from "gnoblin-core/session/focus_history";
import type { HistoryClient } from "gnoblin-core/session/focus_history";
import type { Host, HostEvent, TrackEndedEvent, Version } from "gnoblin-core/session/types";

/** Object lists and focus state supplied by the shell. */
export interface ApiSources extends FindSource {
    /** The window that holds the keyboard focus, or null. */
    focusedWindow(): HistoryClient | null;
}

/**
 * Holds the script-facing APIs for one generation. Reloading creates fresh state; `retire` ends the old generation's
 * pending work.
 */
export class HostApi {
    /** Schedules callbacks and runs them on frame updates. */
    readonly timers: Timers;
    /** Tracks child processes started by scripts. */
    readonly process: ProcessTable;
    /** Sends session and power requests to the shell. */
    readonly session: Session;
    /** Provides runtime information and operations. */
    readonly runtime: Runtime;
    /** Delivers the session's and the configuration's signals to the listeners. */
    readonly events: SessionEvents;
    /** Returns windows in most-recently-focused order. */
    readonly focus: FocusHistory;
    #host: Host;
    #sources: ApiSources;
    #calls: Calls;
    #operations: OperationHost;
    /** Called when the compositor reports that a timed transition has ended. */
    onTrackEnded: (event: TrackEndedEvent) => void = () => {};

    constructor(host: Host, sources: ApiSources) {
        this.#host = host;
        this.#sources = sources;
        this.timers = new Timers(host);
        const operations: OperationHost = {
            schedule: (site, task) => host.schedule(site, task),
            now: () => host.now(),
            after: (ms, fn) => this.timers.after(ms, fn),
        };
        this.#operations = operations;
        this.process = new ProcessTable({
            ...operations,
            spawn: (argv, options) => host.spawn(argv, options),
            kill: (id, signal) => host.kill(id, signal),
            pid: (id) => host.pid(id),
            operationId: () => host.operationId(),
            error: (code, target, message) => host.error(code, target, message),
        });
        this.#calls = new Calls(host, operations);
        this.events = new SessionEvents(
            host,
            () => this.session,
            () => this.runtime,
        );
        this.session = new Session(host, this.#calls, this.events);
        this.runtime = new Runtime(host, this.#calls, this.events);
        this.focus = new FocusHistory(
            () => sources.windows() as HistoryClient[],
            () => sources.focusedWindow(),
        );
    }

    /** Returns the build identity, reading it on each call. */
    version(): Version {
        return Object.freeze(JSON.parse(this.#host.version()) as Version);
    }

    /**
     * A new pending operation for a call of the host's own that the host settles later, as `Output.apply` does when the
     * compositor answers. `method` names the call in a report.
     */
    operation<T>(method: string): Operation<T> {
        return new Operation<T>(this.#operations, this.#host.operationId(), method);
    }

    /** Returns matching live objects in list order. */
    find(kind: string, filter?: Record<string, Scalar> | ((object: never) => unknown)): object[] {
        return find(this.#sources, kind, filter);
    }

    /** Delivers host events, updates focus history and fires due timers. */
    frame(batch: string): void {
        for (const event of JSON.parse(batch) as HostEvent[]) {
            switch (event.kind) {
                case "op":
                    this.#calls.finish(event);
                    break;
                case "signal":
                    this.events.deliver(event);
                    break;
                case "lost":
                    this.#calls.failAll(event.message);
                    break;
                case "exit":
                    this.process.deliver(event);
                    break;
                case "trackEnded":
                    this.onTrackEnded(event);
                    break;
            }
        }
        this.focus.observe();
        this.timers.fire();
    }

    /** Ends this generation's pending timers, handlers and calls. */
    retire(): void {
        this.timers.cancelAll();
        this.events.clear();
        this.#calls.clear();
    }
}
