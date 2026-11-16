// Script-facing session, process and event APIs backed by functions supplied by the shell.

import { Timers } from "gnoblin/operation";
import type { OperationHost } from "gnoblin/operation";
import { ProcessTable } from "gnoblin/process";
import { EventBus } from "gnoblin/session/bus";
import { Calls, Runtime, Session } from "gnoblin/session/calls";
import { find } from "gnoblin/session/find";
import type { FindSource, Scalar } from "gnoblin/session/find";
import { FocusHistory } from "gnoblin/session/focus_history";
import type { HistoryClient } from "gnoblin/session/focus_history";
import type { Host, HostEvent, Version } from "gnoblin/session/types";
import type { TrackEndedEvent } from "gnoblin/transition/types";

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
    /** Sends and receives script events. */
    readonly bus: EventBus;
    /** Returns windows in most-recently-focused order. */
    readonly focus: FocusHistory;
    #host: Host;
    #sources: ApiSources;
    #calls: Calls;
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
        this.process = new ProcessTable({
            ...operations,
            spawn: (argv, options) => host.spawn(argv, options),
            kill: (id, signal) => host.kill(id, signal),
            pid: (id) => host.pid(id),
            operationId: () => host.operationId(),
            error: (code, target, message) => host.error(code, target, message),
        });
        this.#calls = new Calls(host, operations);
        this.session = new Session(host, this.#calls);
        this.runtime = new Runtime(host, this.#calls);
        this.bus = new EventBus(host);
        this.focus = new FocusHistory(
            () => sources.windows() as HistoryClient[],
            () => sources.focusedWindow(),
        );
    }

    /** Returns the build identity, reading it on each call. */
    version(): Version {
        return Object.freeze(JSON.parse(this.#host.version()) as Version);
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
                    this.bus.deliver(event);
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
        this.bus.clear();
        this.#calls.clear();
    }
}
