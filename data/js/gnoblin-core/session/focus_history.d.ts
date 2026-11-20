/** The window fields used by focus history. */
export interface HistoryClient {
    /** Stable window identifier. */
    readonly id: string;
    /** Whether this window is still open. */
    readonly alive: boolean;
    /** The current workspace, or null when it is not assigned to one. */
    readonly workspace: {
        readonly id: string;
    } | null;
}
/** The filters accepted by `gnoblin.focus.history`. Snake_case spellings are accepted too. */
export interface FocusHistoryFilter {
    /** Only the windows on this workspace. */
    workspaceId?: string;
    /** Only the windows on this output; windows without an output match none. */
    monitorId?: string;
    /** The most windows to return, 1 to 256; 50 by default. */
    limit?: number;
}
/**
 * `gnoblin.focus`: the windows in the order they were last focused. The shell builds it over its own `Client`s, and
 * `gnoblin.focus` is typed as `FocusHistory<Client>`; the history itself reads only a `HistoryClient` of each.
 */
export declare class FocusHistory<C extends HistoryClient = HistoryClient> {
    #private;
    constructor(windows: () => C[], focused: () => C | null);
    /** Notes who has the focus now. The shell calls it once a frame. */
    observe(): void;
    /**
     * Returns live windows most recently focused first, then unseen windows in shell list order. Includes minimised
     * windows; omits closed windows. At startup, only the focused window seeds the order.
     */
    history(filter?: FocusHistoryFilter | Record<string, unknown>): C[];
}
