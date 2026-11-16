// The public object exposed to configuration scripts and the console.

import type { Client } from "gnoblin-core/host/client";
import type { Collection } from "gnoblin-core/host/collections";
import { ctx } from "gnoblin-core/host/context";
import type { OutputCollection } from "gnoblin-core/host/roots";
import { motion } from "gnoblin-core/host/seat";
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
export class Gnoblin {
    /** Live clients in the order they were adopted. */
    readonly clients = ctx.clientRoot.self as Collection<Client>;
    /** The outputs, live across hotplug: an output is added when it is plugged in and removed when it goes. */
    readonly outputs = ctx.outputRoot.self as OutputCollection;
    /** The seats of the compositor as input contexts, with the bindings that run on their chords and drags. */
    readonly input = ctx.input;
    /** Policy over the compositor's own consent, passphrase and authentication dialogs: listeners that may refuse a
     * request.
     */
    readonly dialogs = ctx.dialogs;
    /** Watches for the session going idle, and holds that keep it awake. */
    readonly idle = ctx.idleMonitor;
    /** The session's polkit authentications: listeners that watch them and may refuse one. Nothing here can allow
     * one.
     */
    readonly authentication = ctx.authentication;
    /** Launch feedback: the busy state between asking for an application and its window. */
    readonly launches = ctx.launches;

    /** The programs this shell started: `spawn`, `list`, `byId` and the listeners told when one ends. */
    readonly process = ctx.hostApi.process;
    /** Calls that run later, on the frame loop: `after(ms, fn)`, and the monotonic clock `now()` in milliseconds. */
    readonly timers = ctx.hostApi.timers;
    /** The session: its state, and the verbs that log out, suspend, power off or lock it. */
    readonly session = ctx.hostApi.session;
    /** What a script asks of the process that runs it: `reloadConfig`. */
    readonly runtime = ctx.hostApi.runtime;
    /** The windows in the order they were last focused. */
    readonly focus = ctx.hostApi.focus as FocusHistory<Client>;

    /** The milliseconds the shell gives an animation of `duration`: 0 while the seat asks for reduced motion. */
    motion(duration: number): number {
        return motion(duration);
    }

    /**
     * Starts a program and returns its `Process`. A string is one command line for `/bin/sh`; an array is the
     * program and its arguments. The same as `gnoblin.process.spawn`.
     */
    spawn(command: string | string[], options?: SpawnOptions): Process {
        return ctx.hostApi.process.spawn(command, options);
    }

    /** The build identity: `gnoblin`, `gnome`, `mutter`, `js`, `api`, `gitRemote`, `gitSha` and `buildId`. */
    version(): Version {
        return ctx.hostApi.version();
    }

    /**
     * The live objects of one kind (`Window`, and `Output` where the shell has outputs, and the kinds a package adds,
     * such as `Workspace`) that match a table of field values or a predicate, in list order. An unknown kind or field
     * throws and names what works.
     */
    find(kind: "Window", filter?: FindFilter<Client>): Client[];
    find(kind: "Output", filter?: FindFilter<Output>): Output[];
    find(kind: string, filter?: FindFilter<never>): object[];
    find(kind: string, filter?: FindFilter<never>): object[] {
        return ctx.hostApi.find(kind, filter);
    }
}
