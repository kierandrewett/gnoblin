import { CompositorSettings } from "gnoblin-core/devices";
import type { Client } from "gnoblin-core/host/client";
import type { Collection } from "gnoblin-core/host/collections";
import type { ClientListener, CollectionListener, OutputListener } from "gnoblin-core/host/listeners";
import type { Prefixed, Subscribe } from "gnoblin-core/subscriptions";
import type { OutputCollection } from "gnoblin-core/host/roots";
import type { Output } from "gnoblin-core/host/output";
import type { Process, SpawnOptions } from "gnoblin-core/process";
import type { FocusHistory } from "gnoblin-core/session/focus_history";
import type { Version } from "gnoblin-core/session/types";
/** What `find` takes: a table of field values, or a predicate that keeps an object when it returns a truthy value. */
export type FindFilter<T> = Record<string, string | number | boolean> | ((object: T) => unknown);
/**
 * The public `gnoblin` object exposed to configuration code and the console. A package built on core adds its own
 * members to it when it is imported (`gnoblin.workspaces`, `gnoblin.scene`, ...), and merges their types into this class.
 */
export declare class Gnoblin {
    /** Live clients in the order they were adopted. */
    readonly clients: Collection<Client>;
    /** The outputs, live across hotplug: an output is added when it is plugged in and removed when it goes. */
    readonly outputs: OutputCollection;
    /** The seats of the compositor as input contexts, with the bindings that run on their chords and drags. */
    readonly input: import("..").InputManager;
    /** Policy over the compositor's own consent, passphrase and authentication dialogs: listeners that may refuse a
     * request.
     */
    readonly dialogs: import("..").DialogManager;
    /** Watches for the session going idle, and holds that keep it awake. */
    readonly idle: import("..").IdleMonitor;
    /** The session's polkit authentications: listeners that watch them and may refuse one. Nothing here can allow
     * one.
     */
    readonly authentication: import("..").AuthenticationManager;
    /** Launch feedback: the busy state between asking for an application and its window. */
    readonly launches: import("..").LaunchFeedback;
    /** The programs this shell started: `spawn`, `list`, `byId` and the listeners told when one ends. */
    readonly process: import("gnoblin-core/process").ProcessTable;
    /** Calls that run later, on the frame loop: `after(ms, fn)`, and the monotonic clock `now()` in milliseconds. */
    readonly timers: import("../operation").Timers;
    /** The session: its state, and the verbs that log out, suspend, power off or lock it. */
    readonly session: import("../session/calls").Session;
    /** What a script asks of the process that runs it: `reloadConfig`. */
    readonly runtime: import("../session/calls").Runtime;
    /** The compositor's own settings, read when it starts: `userShaders`. A change applies at the next login. */
    readonly compositor: CompositorSettings;
    /** The windows in the order they were last focused. */
    readonly focus: FocusHistory<Client>;
    /** The milliseconds the shell gives an animation of `duration`: 0 while the seat asks for reduced motion. */
    motion(duration: number): number;
    /**
     * Starts a program and returns its `Process`. A string is one command line for `/bin/sh`; an array is the
     * program and its arguments. The same as `gnoblin.process.spawn`.
     */
    spawn(command: string | string[], options?: SpawnOptions): Process;
    /** The build identity: `gnoblin`, `gnome`, `mutter`, `js`, `api`, `gitRemote`, `gitSha` and `buildId`. */
    version(): Version;
    /**
     * The live objects of one kind (`Window`, and `Output` where the shell has outputs, and the kinds a package adds,
     * such as `Workspace`) that match a table of field values or a predicate, in list order. An unknown kind or field
     * throws and names what works.
     */
    find(kind: "Window", filter?: FindFilter<Client>): Client[];
    find(kind: "Output", filter?: FindFilter<Output>): Output[];
    find(kind: string, filter?: FindFilter<never>): object[];
}
/**
 * The named subscriptions of the `gnoblin` object, which forward to `gnoblin.clients` and `gnoblin.outputs`:
 *
 * - `onClientCreated(handler)`: a window appeared. `handler(client)` also runs, once each, for the windows that are
 *   there when it is attached, as `CollectionListener.onAdded` does.
 * - `onClient<Event>(handler)` for each method of `ClientListener`: `onClientTitleChanged`, `onClientFocusChanged`,
 *   `onClientClosed`, `onClientReady` and the rest. `handler(client, ...)` hears the event of every window.
 * - `onOutputAdded` and `onOutputRemoved` for `CollectionListener`, and `onOutput<Event>` for each method of
 *   `OutputListener` (`onOutputConnected`, `onOutputModeChanged`, ...): the same for every output.
 *
 * Each returns the function that detaches the handler.
 */
export interface Gnoblin extends Prefixed<"Client", ClientListener>, Prefixed<"Output", OutputListener>, Prefixed<"Output", CollectionListener<Output>> {
    /**
     * A window appeared: `handler(client)` runs for every window that joins `gnoblin.clients`, and for each one there
     * already when it is attached. The same as `gnoblin.clients.onAdded`.
     */
    readonly onClientCreated: Subscribe<CollectionListener<Client>, "onAdded">;
}
