// The host API that is not about windows: `gnoblin.session`, `gnoblin.runtime`, `gnoblin.process`, `gnoblin.timers`,
// `gnoblin.on`, `gnoblin.find`, `gnoblin.focus` and `gnoblin.version`.
// `HostApi` is plain TypeScript over a few functions of the shell (`ApiHost`), so it runs without a compositor. The
// shell calls `frame` once a frame with what happened outside the script: a call the compositor finished, a signal it
// raised, a process that ended. Every listener and callback runs as a queued call under the usual wall-clock budget.
// Errors are typed. A call that cannot be made at all throws (`TypeError` for a bad argument). A call that is made and
// refused returns an `Operation` that fails with an `OperationError`; its `code` is one of `docs/shell-api/errors.md`.

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

/** What the shell gives the module that is not a host function. */
export interface ApiSources extends FindSource {
    /** The window that holds the keyboard focus, or null. */
    focusedWindow(): HistoryClient | null;
}

/**
 * The host API of one generation: its timers, processes, bus and session calls. A generation's state is its own, so
 * a reload starts these afresh and `retire` ends what the old one had pending.
 */
export class HostApi {
    readonly timers: Timers;
    readonly process: ProcessTable;
    readonly session: Session;
    readonly runtime: Runtime;
    readonly bus: EventBus;
    readonly focus: FocusHistory;
    #host: Host;
    #sources: ApiSources;
    #calls: Calls;
    /** Told of each timed write the compositor says has ended; `gnoblin.requestTransition` is the one that listens. */
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

    /** The build identity, read on each call. */
    version(): Version {
        return Object.freeze(JSON.parse(this.#host.version()) as Version);
    }

    find(kind: string, filter?: Record<string, Scalar> | ((object: never) => unknown)): object[] {
        return find(this.#sources, kind, filter);
    }

    /** One frame: what the host collected, then the timers that are due. */
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

    /** Ends what the generation had pending: its timers, its handlers and the calls it was waiting on. */
    retire(): void {
        this.timers.cancelAll();
        this.bus.clear();
        this.#calls.clear();
    }
}
