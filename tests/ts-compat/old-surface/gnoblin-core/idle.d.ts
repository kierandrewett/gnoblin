/** The longest idle time a watch may ask for: a day. */
export declare const MAX_IDLE_MS = 86400000;
/** How many live watches one runtime may have. */
export declare const MAX_IDLE_WATCHES = 256;
/** What the compositor said about idleness, as the shell hands it over. */
export interface IdleReading {
    /** Milliseconds since the last input event. */
    idleMs: number;
    /** How many holds keep the session awake. */
    inhibitors: number;
}
/** A request for the shell to send to the compositor, taken once. */
export interface IdleCommand {
    /** Whether the compositor should add or remove the hold. */
    kind: "inhibit" | "uninhibit";
    /** Name identifying the hold. */
    holder: string;
}
/** What the manager needs of the shell: the call queue. */
export interface IdleHost {
    /** Queues a listener callback for the shell to run. */
    schedule(site: string, task: () => void): void;
}
/** Observes one watch: pass it to `gnoblin.idle.watch`. Every method is optional, and each receives the watch first. */
export declare class IdleListener {
    /** The session has been idle for the watch's time and nothing inhibits idle. */
    onIdle?(watch: IdleWatch): void;
    /** The first input after an idle period, or an inhibitor that took the idle state away. */
    onResume?(watch: IdleWatch): void;
    /** The watch has ended: `reason` is `"cancelled"`. */
    onClosed?(watch: IdleWatch, reason: string): void;
}
/**
 * One idle watch. It is not idle when it starts: the first `onIdle` comes when a reading shows the session idle for
 * `ms`, and every idle period after an input runs `onIdle` again until the watch is cancelled.
 */
export declare class IdleWatch {
    #private;
    /** `"idle-"` and a number. */
    readonly id: string;
    /** The idle time asked for, in milliseconds. */
    readonly ms: number;
    /** Made by `gnoblin.idle.watch`; a script has no use for the constructor. */
    constructor(manager: IdleMonitor, id: string, ms: number, handlers: IdleListener);
    /** True while the session has been idle for `ms` with no inhibitor. */
    get idle(): boolean;
    /** True until the watch has been cancelled. */
    get alive(): boolean;
    /** Ends the watch. Its `onClosed` runs once; a watch that has ended does nothing. */
    cancel(): void;
    /** The manager's side: the new idle state, and the handler to run when it changed. Not for scripts. */
    static settle(watch: IdleWatch, idle: boolean): ((host: IdleHost) => void) | null;
    /** The manager's side of `cancel`: the watch ends and its `onClosed` is scheduled. */
    static close(watch: IdleWatch, host: IdleHost, reason: string): boolean;
}
/** A hold on the session's idle state: while one is alive the session does not go idle. */
export declare class IdleInhibitor {
    #private;
    /** The name the hold is recorded under. */
    readonly holder: string;
    constructor(manager: IdleMonitor, holder: string);
    /** True until the hold is released, or the compositor refused it. */
    get alive(): boolean;
    /** Gives the hold back. A hold that has ended does nothing. */
    release(): void;
    /** Ends an inhibitor and reports whether it was still active. */
    static end(inhibitor: IdleInhibitor): boolean;
}
/** `gnoblin.idle`: the watches and holds of this runtime, and the last reading of the compositor. */
export declare class IdleMonitor {
    #private;
    constructor(host: IdleHost);
    /** Milliseconds since the last input, as of the last reading; null before the compositor has said. */
    get idleMs(): number | null;
    /** How many holds keep the session awake, as of the last reading; null before the compositor has said. */
    get inhibitors(): number | null;
    /** The live watches, oldest first. */
    get watches(): readonly IdleWatch[];
    /**
     * Calls `listener.onIdle` when the session has been idle for `ms` milliseconds, and `onResume` when it is active
     * again. `ms` is a number above 0 and at most a day; a runtime has at most 256 live watches.
     */
    watch(ms: number, listener?: IdleListener): IdleWatch;
    /**
     * Holds the session awake under `holder`, a name of your choice: the compositor counts the hold in `inhibitors`
     * and sends no idle notification while one stands. Holding again under a name that is held returns that hold.
     */
    inhibit(holder: string): IdleInhibitor;
    /** Cancels a watch and removes it from this monitor. */
    cancelWatch(watch: IdleWatch): void;
    /** Releases an inhibitor and queues its removal from the compositor. */
    releaseInhibitor(inhibitor: IdleInhibitor): void;
    /** Whether the shell should hold a connection to the compositor's idle state. */
    get wanted(): boolean;
    /** What the compositor said since the last frame. The shell calls this; a script does not. */
    deliver(reading: IdleReading): void;
    /** The requests to send, each once. The shell calls this; a script does not. */
    takeCommands(): IdleCommand[];
    /** The names of the holds that are alive, so the shell can give them back when the runtime ends. */
    holders(): string[];
    /** The compositor refused a hold: it is not a hold, and `alive` says so. */
    failed(holder: string): void;
    /**
     * The connection to the compositor ended: nothing is idle any more, and the last reading is forgotten. The holds
     * went with it, because the compositor keeps each one for the connection that made it, so they are not holds now.
     */
    lost(): void;
}
