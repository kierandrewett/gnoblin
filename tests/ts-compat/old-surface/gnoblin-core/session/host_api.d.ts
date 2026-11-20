import { Operation, Timers } from "gnoblin-core/operation";
import { ProcessTable } from "gnoblin-core/process";
import { Runtime, Session } from "gnoblin-core/session/calls";
import { SessionEvents } from "gnoblin-core/session/events";
import type { FindSource, Scalar } from "gnoblin-core/session/find";
import { FocusHistory } from "gnoblin-core/session/focus_history";
import type { HistoryClient } from "gnoblin-core/session/focus_history";
import type { Host, TrackEndedEvent, Version } from "gnoblin-core/session/types";
/** Object lists and focus state supplied by the shell. */
export interface ApiSources extends FindSource {
    /** The window that holds the keyboard focus, or null. */
    focusedWindow(): HistoryClient | null;
}
/**
 * Holds the script-facing APIs for one generation. Reloading creates fresh state; `retire` ends the old generation's
 * pending work.
 */
export declare class HostApi {
    #private;
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
    /** Called when the compositor reports that a timed transition has ended. */
    onTrackEnded: (event: TrackEndedEvent) => void;
    constructor(host: Host, sources: ApiSources);
    /** Returns the build identity, reading it on each call. */
    version(): Version;
    /**
     * A new pending operation for a call of the host's own that the host settles later, as `Output.apply` does when the
     * compositor answers. `method` names the call in a report.
     */
    operation<T>(method: string): Operation<T>;
    /** Returns matching live objects in list order. */
    find(kind: string, filter?: Record<string, Scalar> | ((object: never) => unknown)): object[];
    /** Delivers host events, updates focus history and fires due timers. */
    frame(batch: string): void;
    /** Ends this generation's pending timers, handlers and calls. */
    retire(): void;
}
