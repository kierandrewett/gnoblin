// `gnoblin.idle`: run code when the session has had no input for a set time, and hold the session awake.
// The compositor publishes how long the session has been idle (`idle_ms` of the `idle` object) and how many holds keep
// it awake (`inhibitors`: Wayland idle-inhibit surfaces and the holders `inhibit` recorded). A watch is a threshold
// over that reading and nothing else, so a script's watch behaves like the compositor's own idle notifications: an
// inhibitor delays `onIdle` as it delays a screen blank, and one that appears while the session is idle resumes it.
// The manager is plain TypeScript over two functions of the host (`IdleHost`), so it runs without a compositor. A
// handler runs as a queued call under the same wall-clock budget as every other listener: one that throws is reported
// once and the next still runs, and one that runs past its budget is interrupted.

/** The longest idle time a watch may ask for: a day. */
export const MAX_IDLE_MS = 86_400_000;

/** How many live watches one runtime may have. */
export const MAX_IDLE_WATCHES = 256;

/** What the compositor said about idleness, as the shell hands it over. */
export interface IdleReading {
    /** Milliseconds since the last input event. */
    idleMs: number;
    /** How many holds keep the session awake. */
    inhibitors: number;
}

/** A request for the shell to send to the compositor, taken once. */
export interface IdleCommand {
    kind: "inhibit" | "uninhibit";
    holder: string;
}

/** What the manager needs of the shell: the call queue. */
export interface IdleHost {
    schedule(site: string, task: () => void): void;
}

/** What a watch does when the session goes idle and when it comes back. Every member is optional. */
export interface IdleWatchHandlers {
    /** The session has been idle for the watch's time and nothing inhibits idle. */
    onIdle?(watch: IdleWatch): void;
    /** The first input after an idle period, or an inhibitor that took the idle state away. */
    onResume?(watch: IdleWatch): void;
    /** The watch has ended: `reason` is `"cancelled"`. */
    onClosed?(watch: IdleWatch, reason: string): void;
}

function validateHandlers(handlers: unknown): IdleWatchHandlers {
    if (handlers === undefined) {
        return {};
    }
    if (handlers === null || typeof handlers !== "object") {
        throw new TypeError("gnoblin.idle.watch: the handlers must be an object");
    }
    const given = handlers as Record<string, unknown>;
    for (const name of ["onIdle", "onResume", "onClosed"]) {
        if (given[name] !== undefined && typeof given[name] !== "function") {
            throw new TypeError(`gnoblin.idle.watch: ${name} must be a function`);
        }
    }
    return given as IdleWatchHandlers;
}

/**
 * One idle watch. It is not idle when it starts: the first `onIdle` comes when a reading shows the session idle for
 * `ms`, and every idle period after an input runs `onIdle` again until the watch is cancelled.
 */
export class IdleWatch {
    /** `"idle-"` and a number. */
    readonly id: string;
    /** The idle time asked for, in milliseconds. */
    readonly ms: number;
    #idle = false;
    #alive = true;
    readonly #handlers: IdleWatchHandlers;
    readonly #manager: IdleMonitor;

    /** Made by `gnoblin.idle.watch`; a script has no use for the constructor. */
    constructor(manager: IdleMonitor, id: string, ms: number, handlers: IdleWatchHandlers) {
        this.#manager = manager;
        this.id = id;
        this.ms = ms;
        this.#handlers = handlers;
    }

    /** True while the session has been idle for `ms` with no inhibitor. */
    get idle(): boolean {
        return this.#idle;
    }

    /** True until the watch has been cancelled. */
    get alive(): boolean {
        return this.#alive;
    }

    /** Ends the watch. Its `onClosed` runs once; a watch that has ended does nothing. */
    cancel(): void {
        this.#manager.cancelWatch(this);
    }

    /** The manager's side: the new idle state, and the handler to run when it changed. Not for scripts. */
    static settle(watch: IdleWatch, idle: boolean): ((host: IdleHost) => void) | null {
        if (!watch.#alive || watch.#idle === idle) {
            return null;
        }
        watch.#idle = idle;
        const name = idle ? "onIdle" : "onResume";
        const handler = watch.#handlers[name];
        if (handler === undefined) {
            return null;
        }
        return (host) => {
            host.schedule(`IdleWatch.${name}`, () => {
                if (watch.#alive) {
                    handler.call(watch.#handlers, watch);
                }
            });
        };
    }

    /** The manager's side of `cancel`: the watch ends and its `onClosed` is scheduled. */
    static close(watch: IdleWatch, host: IdleHost, reason: string): boolean {
        if (!watch.#alive) {
            return false;
        }
        watch.#alive = false;
        watch.#idle = false;
        const closed = watch.#handlers.onClosed;
        if (closed !== undefined) {
            host.schedule("IdleWatch.onClosed", () => closed.call(watch.#handlers, watch, reason));
        }
        return true;
    }
}

/** A hold on the session's idle state: while one is alive the session does not go idle. */
export class IdleInhibitor {
    /** The name the hold is recorded under. */
    readonly holder: string;
    #alive = true;
    readonly #manager: IdleMonitor;

    constructor(manager: IdleMonitor, holder: string) {
        this.#manager = manager;
        this.holder = holder;
    }

    /** True until the hold is released, or the compositor refused it. */
    get alive(): boolean {
        return this.#alive;
    }

    /** Gives the hold back. A hold that has ended does nothing. */
    release(): void {
        this.#manager.releaseInhibitor(this);
    }

    static end(inhibitor: IdleInhibitor): boolean {
        const was = inhibitor.#alive;
        inhibitor.#alive = false;
        return was;
    }
}

/** `gnoblin.idle`: the watches and holds of this runtime, and the last reading of the compositor. */
export class IdleMonitor {
    readonly #host: IdleHost;
    readonly #watches: IdleWatch[] = [];
    readonly #inhibitors = new Map<string, IdleInhibitor>();
    #commands: IdleCommand[] = [];
    #reading: IdleReading | null = null;
    #count = 0;

    constructor(host: IdleHost) {
        this.#host = host;
    }

    /** Milliseconds since the last input, as of the last reading; null before the compositor has said. */
    get idleMs(): number | null {
        return this.#reading === null ? null : this.#reading.idleMs;
    }

    /** How many holds keep the session awake, as of the last reading; null before the compositor has said. */
    get inhibitors(): number | null {
        return this.#reading === null ? null : this.#reading.inhibitors;
    }

    /** The live watches, oldest first. */
    get watches(): readonly IdleWatch[] {
        return [...this.#watches];
    }

    /**
     * Runs `handlers.onIdle` when the session has been idle for `ms` milliseconds, and `onResume` when it is active
     * again. `ms` is a number above 0 and at most a day; a runtime has at most 256 live watches.
     */
    watch(ms: number, handlers?: IdleWatchHandlers): IdleWatch {
        if (typeof ms !== "number" || !Number.isFinite(ms) || ms <= 0 || ms > MAX_IDLE_MS) {
            throw new RangeError(`gnoblin.idle.watch: ms must be a number above 0 and at most ${MAX_IDLE_MS}`);
        }
        const given = validateHandlers(handlers);
        if (this.#watches.length >= MAX_IDLE_WATCHES) {
            throw new RangeError(`gnoblin.idle.watch: a runtime has at most ${MAX_IDLE_WATCHES} live watches`);
        }
        this.#count += 1;
        const watch = new IdleWatch(this, `idle-${this.#count}`, ms, given);
        this.#watches.push(watch);
        // A reading the shell already has decides the first state at once, so a watch made while the session is idle
        // does not wait for the next change of the counter.
        this.#evaluate(watch);
        return watch;
    }

    /**
     * Holds the session awake under `holder`, a name of your choice: the compositor counts the hold in `inhibitors`
     * and sends no idle notification while one stands. Holding again under a name that is held returns that hold.
     */
    inhibit(holder: string): IdleInhibitor {
        if (typeof holder !== "string" || holder.length === 0) {
            throw new TypeError("gnoblin.idle.inhibit: the holder must be a non-empty string");
        }
        const held = this.#inhibitors.get(holder);
        if (held !== undefined) {
            return held;
        }
        const inhibitor = new IdleInhibitor(this, holder);
        this.#inhibitors.set(holder, inhibitor);
        this.#commands.push({ kind: "inhibit", holder });
        return inhibitor;
    }

    cancelWatch(watch: IdleWatch): void {
        if (IdleWatch.close(watch, this.#host, "cancelled")) {
            this.#watches.splice(this.#watches.indexOf(watch), 1);
        }
    }

    releaseInhibitor(inhibitor: IdleInhibitor): void {
        if (IdleInhibitor.end(inhibitor)) {
            this.#inhibitors.delete(inhibitor.holder);
            this.#commands.push({ kind: "uninhibit", holder: inhibitor.holder });
        }
    }

    /** Whether the shell should hold a connection to the compositor's idle state. */
    get wanted(): boolean {
        return this.#watches.length > 0 || this.#inhibitors.size > 0;
    }

    /** What the compositor said since the last frame. The shell calls this; a script does not. */
    deliver(reading: IdleReading): void {
        this.#reading = reading;
        for (const watch of [...this.#watches]) {
            this.#evaluate(watch);
        }
    }

    /** The requests to send, each once. The shell calls this; a script does not. */
    takeCommands(): IdleCommand[] {
        const taken = this.#commands;
        this.#commands = [];
        return taken;
    }

    /** The names of the holds that are alive, so the shell can give them back when the runtime ends. */
    holders(): string[] {
        return [...this.#inhibitors.keys()];
    }

    /** The compositor refused a hold: it is not a hold, and `alive` says so. */
    failed(holder: string): void {
        const inhibitor = this.#inhibitors.get(holder);
        if (inhibitor !== undefined) {
            IdleInhibitor.end(inhibitor);
            this.#inhibitors.delete(holder);
        }
    }

    /**
     * The connection to the compositor ended: nothing is idle any more, and the last reading is forgotten. The holds
     * went with it, because the compositor keeps each one for the connection that made it, so they are not holds now.
     */
    lost(): void {
        this.#reading = null;
        for (const inhibitor of this.#inhibitors.values()) {
            IdleInhibitor.end(inhibitor);
        }
        this.#inhibitors.clear();
        for (const watch of this.#watches) {
            const resume = IdleWatch.settle(watch, false);
            resume?.(this.#host);
        }
    }

    #evaluate(watch: IdleWatch): void {
        if (this.#reading === null) {
            return;
        }
        const idle = this.#reading.inhibitors === 0 && this.#reading.idleMs >= watch.ms;
        IdleWatch.settle(watch, idle)?.(this.#host);
    }
}
