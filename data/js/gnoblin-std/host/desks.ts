// Desks represent what outputs show and where workspaces go when outputs change.

import { host } from "gnoblin-core/host/bridge";
import type { Client } from "gnoblin-core/host/client";
import { recompute } from "gnoblin-core/host/collection_events";
import { ctx } from "gnoblin-core/host/context";
import { std } from "gnoblin-std/host/state";
import type { Output } from "gnoblin-core/host/output";
import type { Workspace } from "gnoblin-std/host/workspace";
import { wsState } from "gnoblin-std/host/workspace_state";

/**
 * The workspace shown by an output and how its windows are handled when output availability changes. A parked desk
 * waits hidden for an output to return. With no outputs, one unparked desk keeps a workspace shown. Each workspace is
 * on one desk and may span several outputs.
 */
export interface Desk {
    /** Output using this desk, or null when it has none. */
    output: Output | null;
    /** Workspace currently shown by this desk, or null when empty. */
    shown: Workspace | null;
    /** Whether the desk waits hidden for an output to return. */
    parked: boolean;
    /** The outputs this desk's one workspace is stretched over (this desk's own first), or null for a desk of one
     * output.
     */
    spans: Output[] | null;
    /** The desk whose workspace covers this desk's output too. Such a desk shows no workspace of its own. */
    spannedBy: Desk | null;
}

/** The desk that decides what `desk`'s output shows: itself, or the desk spanning over it. */
export function effectiveDesk(desk: Desk): Desk {
    return desk.spannedBy ?? desk;
}

/** The outputs that are connected, in the order the compositor made them. */
export function liveOutputs(): Output[] {
    return ctx.outputRoot.list as Output[];
}

/** The outputs that have a desk: every connected output but a mirror and one that is off. */
export function deskOutputs(): Output[] {
    return liveOutputs().filter((output) => std.deskByOutput.has(output));
}

/** The output flagged primary, or the first one; null while the tree has none. */
export function primaryOutput(): Output | null {
    const list = deskOutputs();
    return list.find((output) => output.primary) ?? list[0] ?? null;
}

/** The desk of the primary output; with no output, the desk a tree without one keeps (or a parked one). */
export function primaryDesk(): Desk | null {
    const output = primaryOutput();
    return output === null
        ? (std.desks.find((desk) => desk.output === null) ?? null)
        : (std.deskByOutput.get(output) ?? null);
}

/** The desk a new window, a key and a plain `show` act on: the one under the pointer, else the primary one. */
export function currentDesk(): Desk | null {
    const node = host.pointerScreen();
    const under = node === undefined ? undefined : ctx.outputsByNode.get(node);
    const own = (under === undefined ? undefined : std.deskByOutput.get(under)) ?? primaryDesk();
    return own === null ? null : effectiveDesk(own);
}

/** Finds the desk that currently shows a workspace, if any. */
export function deskShowing(workspace: Workspace): Desk | null {
    return std.desks.find((desk) => desk.shown === workspace) ?? null;
}

/** Whether an output (or the tree's one output-less desk) shows `workspace`. A parked desk shows nothing. */
export function isShown(workspace: Workspace): boolean {
    const desk = deskShowing(workspace);
    return desk !== null && !desk.parked;
}

/** The desk the workspace `client` is on is shown on, or null. */
export function deskOfClient(client: Client): Desk | null {
    const workspace = std.membership.get(client);
    return workspace === undefined ? null : deskShowing(workspace);
}

/** A desk for a tree that has no output, so the first workspace is on show whatever the compositor has. */
export function ensureDesk(): void {
    if (std.desks.length === 0) {
        std.desks.push({ output: null, shown: null, parked: false, spans: null, spannedBy: null });
    }
}

/** Gives each desk that shows nothing the first workspace no desk shows. */
export function fillDesks(): void {
    for (const desk of std.desks) {
        if (desk.shown !== null || desk.spannedBy !== null) {
            continue;
        }
        const free = (std.workspaceRoot.list as Workspace[]).find((workspace) => deskShowing(workspace) === null);
        if (free !== undefined) {
            setShown(desk, free);
        }
    }
}

/** Makes `next` the workspace `desk` shows. */
export function setShown(desk: Desk, next: Workspace | null): void {
    const before = desk.shown;
    if (before === next) {
        return;
    }
    desk.shown = next;
    if (next) {
        wsState(next).dirty = true;
    }
    for (const changed of [before, next]) {
        if (changed && wsState(changed).attached) {
            recompute(std.workspaceRoot, changed);
        }
    }
}
