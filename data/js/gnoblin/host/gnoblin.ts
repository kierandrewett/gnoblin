// The public `gnoblin` object: the root collections, the managers, and the calls a script makes on it.

import type { Client } from "gnoblin/host/client";
import type { Collection } from "gnoblin/host/collections";
import { ctx } from "gnoblin/host/context";
import type { DisplayCollection, OutputCollection, WorkspaceCollection } from "gnoblin/host/roots";
import { motion } from "gnoblin/host/seat";
import { showWorkspace } from "gnoblin/host/show";
import type { ShowOptions } from "gnoblin/host/show";
import type { Workspace } from "gnoblin/host/workspace";
import type { Subscription } from "gnoblin/operation";
import type { Process, SpawnOptions } from "gnoblin/process";
import { sceneOf } from "gnoblin/scene/scene";
import type { SignalEvent, Version } from "gnoblin/session/types";
import type { Transition } from "gnoblin/transition/transition";
import type { TransitionOptions } from "gnoblin/transition/types";
import { Extensions } from "gnoblin/xr/extensions";

/** The public `gnoblin` object exposed to configuration code and the console. */
export class Gnoblin {
    /** The optional extensions: `gnoblin.extensions.xr` is null until the XR program has a session (docs/xr.md). */
    readonly extensions = new Extensions(ctx.xrRuntime);
    readonly clients = ctx.clientRoot.self as Collection<Client>;
    readonly workspaces = ctx.workspaceRoot.self as WorkspaceCollection;
    /** The outputs, live across hotplug: an output is added when it is plugged in and removed when it goes. */
    readonly outputs = ctx.outputRoot.self as OutputCollection;
    /** The logical displays: what each monitor shows, and through which camera. See `Display`. */
    readonly displays = ctx.displayRoot.self as DisplayCollection;
    /** The seats of the compositor as input contexts, with the bindings that run on their chords and drags. */
    readonly input = ctx.input;
    /** Policy over the compositor's own consent, passphrase and authentication dialogs: listeners that may refuse a request. */
    readonly dialogs = ctx.dialogs;
    /** Watches for the session going idle, and holds that keep it awake. */
    readonly idle = ctx.idleMonitor;
    /** The session's polkit authentications: listeners that watch them and may refuse one. Nothing here can allow one. */
    readonly authentication = ctx.authentication;
    /** Launch feedback: the busy state between asking for an application and its window. */
    readonly launches = ctx.launches;
    /** The scene: its `root` is where a script mounts the nodes it makes (`Node`, `View`, `Camera3D`). */
    readonly scene = sceneOf();

    /** The programs this shell started: `spawn`, `list`, `byId` and the listeners told when one ends. */
    readonly process = ctx.hostApi.process;
    /** Calls that run later, on the frame loop: `after(ms, fn)`, and the monotonic clock `now()` in milliseconds. */
    readonly timers = ctx.hostApi.timers;
    /** The session: its state, and the verbs that log out, suspend, power off or lock it. */
    readonly session = ctx.hostApi.session;
    /** What a script asks of the process that runs it: `reloadConfig`. */
    readonly runtime = ctx.hostApi.runtime;
    /** The windows in the order they were last focused. */
    readonly focus = ctx.hostApi.focus;
    /** The touchpad gesture actions (`workspace.next` and four more) and `onSwipe`, `onPinch` and `direction`. */
    readonly gestures = ctx.gestureTools;

    /** The milliseconds the shell gives an animation of `duration`: 0 while the seat asks for reduced motion. */
    motion(duration: number): number {
        return motion(duration);
    }

    /**
     * Runs `callback` once, now, with every property write staged, and draws the change as one animation: each
     * written property moves from where it is drawn to the staged value over `options.duration` milliseconds. The
     * callback also holds layout changes (`workspace.layout = ...`), which move every window from its place. A
     * failure in the callback leaves the previous state. Nothing animates under reduced motion.
     */
    requestTransition(callback: () => void, options?: TransitionOptions): Transition {
        return ctx.transitions.request(callback, options);
    }

    /**
     * Starts a program and returns its `Process`. A string is one command line for `/bin/sh`; an array is the
     * program and its arguments. The same as `gnoblin.process.spawn`.
     */
    spawn(command: string | string[], options?: SpawnOptions): Process {
        return ctx.hostApi.process.spawn(command, options);
    }

    /** Shows a workspace, given as itself or by id, on the output under the pointer. See `WorkspaceCollection.show`. */
    show(workspace: Workspace | string, options?: ShowOptions): void {
        showWorkspace(workspace, options);
    }

    /** The build identity: `gnoblin`, `gnome`, `mutter`, `js`, `api`, `gitRemote`, `gitSha` and `buildId`. */
    version(): Version {
        return ctx.hostApi.version();
    }

    /**
     * The live objects of one kind (`Window`, `Workspace`, and `Output` where the shell has outputs) that match a
     * table of field values or a predicate, in list order. An unknown kind or field throws and names what works.
     */
    find(kind: string, filter?: Record<string, string | number | boolean> | ((object: never) => unknown)): object[] {
        return ctx.hostApi.find(kind, filter);
    }

    /**
     * Calls `fn` for every `name` the compositor raises, `namespace.object.signal` (hyphens are accepted for
     * underscores). An unknown name throws and lists the near ones.
     */
    on(name: string, fn: (event: SignalEvent) => void): Subscription {
        return ctx.hostApi.bus.on(name, fn);
    }

    /** Like `on`, for the first signal only. */
    once(name: string, fn: (event: SignalEvent) => void): Subscription {
        return ctx.hostApi.bus.once(name, fn);
    }

    /** Detaches a subscription, or every handler `fn` of the event `name`. */
    off(target: Subscription | string, fn?: (event: SignalEvent) => void): void {
        ctx.hostApi.bus.off(target, fn);
    }
}
