// `gnoblin.focus`: the windows in the order they were last focused.

/** A window as the focus history reads it. */
export interface HistoryClient {
    readonly id: string;
    readonly alive: boolean;
    readonly workspace: { readonly id: string } | null;
}

/** What `gnoblin.focus.history` takes. The snake_case spellings are accepted too. */
export interface FocusHistoryFilter {
    /** Only the windows on this workspace. */
    workspaceId?: string;
    /** Only the windows on this output; a window with no `output` property matches none. */
    monitorId?: string;
    /** The most windows to return, 1 to 256; 50 by default. */
    limit?: number;
}

const HISTORY_LIMIT = 256;

const HISTORY_DEFAULT = 50;

/** `gnoblin.focus`: the windows in the order they were last focused. */
export class FocusHistory {
    #windows: () => HistoryClient[];
    #focused: () => HistoryClient | null;
    #order: HistoryClient[] = [];

    constructor(windows: () => HistoryClient[], focused: () => HistoryClient | null) {
        this.#windows = windows;
        this.#focused = focused;
    }

    /** Notes who has the focus now. The shell calls it once a frame. */
    observe(): void {
        const focused = this.#focused();
        this.#order = this.#order.filter((client) => client.alive && client !== focused);
        if (focused !== null && focused.alive) {
            this.#order.unshift(focused);
        }
    }

    /**
     * The live windows, most recently focused first, then those whose focus was never seen in the order the shell
     * lists them. Minimised windows are included and a window that closed is gone. At startup only the focused window
     * seeds the order.
     */
    history(filter?: FocusHistoryFilter | Record<string, unknown>): HistoryClient[] {
        const given = filter ?? {};
        if (typeof given !== "object" || given === null) {
            throw new TypeError("gnoblin.focus.history: the filter is an object");
        }
        const unknown = Object.keys(given).find(
            (key) => !["workspaceId", "workspace_id", "monitorId", "monitor_id", "limit"].includes(key),
        );
        if (unknown !== undefined) {
            throw new TypeError(
                `gnoblin.focus.history: no filter named ${unknown}; the filters are workspaceId, monitorId, limit`,
            );
        }
        const take = (camelName: string, snake: string): unknown =>
            Reflect.get(given, camelName) ?? Reflect.get(given, snake);
        const limit = take("limit", "limit") ?? HISTORY_DEFAULT;
        if (typeof limit !== "number" || !Number.isInteger(limit) || limit < 1 || limit > HISTORY_LIMIT) {
            throw new TypeError(`gnoblin.focus.history: limit is an integer from 1 to ${HISTORY_LIMIT}`);
        }
        const workspace = take("workspaceId", "workspace_id");
        const monitor = take("monitorId", "monitor_id");
        const live = this.#windows();
        const seen = this.#order.filter((client) => live.includes(client));
        const ordered = [...seen, ...live.filter((client) => !seen.includes(client))];
        return ordered
            .filter((client) => workspace === undefined || client.workspace?.id === workspace)
            .filter(
                (client) =>
                    monitor === undefined ||
                    (Reflect.get(client, "output") as { id?: string } | undefined)?.id === monitor,
            )
            .slice(0, limit);
    }
}
