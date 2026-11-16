// Connects logical displays and output profiles to the compositor's connected outputs.

import { Display } from "gnoblin-std/display/display";
import { attachDisplays } from "gnoblin-std/display/state";
import type { OutputConfig } from "gnoblin-core/output_config";
import type { OutputProfile } from "gnoblin-std/display/output_profile";
import { DisplayManager } from "gnoblin-std/display_manager/manager";
import type { DisplayBackend, OutputSet } from "gnoblin-std/display_manager/backend";
import { GnoblinError } from "gnoblin-core/errors";
import { CREATE } from "gnoblin-core/host/bridge";
import { holders, recompute } from "gnoblin-core/host/collection_events";
import { collectionState, rootState } from "gnoblin-core/host/collection_state";
import { Collection } from "gnoblin-core/host/collections";
import { ctx } from "gnoblin-core/host/context";
import { std } from "gnoblin-std/host/state";
import { effectiveDesk, fillDesks, liveOutputs, primaryOutput, setShown } from "gnoblin-std/host/desks";
import { queueFor, say } from "gnoblin-core/host/dispatch";
import { Output } from "gnoblin-core/host/output";
import { applyOutput } from "gnoblin-core/host/output_apply";
import { showWorkspace } from "gnoblin-std/host/show";
import { Workspace } from "gnoblin-std/host/workspace";
import { wsState } from "gnoblin-std/host/workspace_state";

/** Whether two lists hold the same outputs in the same order. */
function sameOutputs(a: readonly Output[], b: readonly Output[]): boolean {
    return a.length === b.length && a.every((output, at) => output === b[at]);
}

/**
 * Makes the desk of `lead` show one workspace over `members` (the desks of the other members show nothing of their own
 * and cover the same workspace), or ends the span with null. Asking for the span that holds changes nothing.
 */
function spanDesks(lead: Output, members: readonly Output[] | null): void {
    const desk = std.deskByOutput.get(lead);
    if (desk === undefined) {
        return;
    }
    const covered = (members ?? []).filter((output) => std.deskByOutput.has(output));
    const next = covered.length > 1 ? covered : null;
    if (next === null ? desk.spans === null : desk.spans !== null && sameOutputs(desk.spans, next)) {
        return;
    }
    for (const other of std.desks) {
        if (other.spannedBy === desk) {
            other.spannedBy = null;
        }
    }
    desk.spans = next;
    for (const output of next ?? []) {
        const under = std.deskByOutput.get(output);
        if (under !== undefined && under !== desk) {
            under.spannedBy = desk;
            setShown(under, null);
        }
    }
    if (desk.shown !== null) {
        wsState(desk.shown).dirty = true;
    }
    fillDesks();
}

/** Sends an output request on behalf of a profile or display; a request the shell cannot send is reported, not
 * thrown.
 */
function requestOutput(
    output: Output,
    setting: Omit<OutputConfig, "mirrorOf"> & { mirrorOf?: Output | string | null },
    path: string,
): void {
    try {
        applyOutput(output, setting, `gnoblin.${path}`, path);
    } catch (error) {
        say(`${path}: ${error instanceof Error ? error.message : String(error)}`);
    }
}

/** A `Collection<Output>` for one display, kept equal to what the display presents. */
function makeOutputSet(): OutputSet {
    const collection = new Collection<Output>(CREATE, rootState("Output"));
    const state = collectionState(collection);
    return {
        collection,
        set(outputs) {
            const before = state.list as Output[];
            if (sameOutputs(before, outputs as Output[])) {
                return;
            }
            state.list = [...outputs] as Output[];
            for (const output of outputs as Output[]) {
                if (!before.includes(output)) {
                    queueFor(state, "onAdded", [output, collection]);
                }
            }
            for (const output of before) {
                if (!outputs.includes(output)) {
                    queueFor(state, "onRemoved", [output, collection]);
                }
            }
        },
    };
}

/** Tells the collection that holds `member` that it joined (`onAdded`) or left (`onRemoved`). */
function announceMember(member: Display | OutputProfile, joined: boolean): void {
    const root = member instanceof Display ? std.displayRoot : std.profileRoot;
    if (joined) {
        queueFor(root, "onAdded", [member, root.self]);
        recompute(root, member);
        return;
    }
    for (const view of holders(root, member)) {
        view.members?.delete(member);
        queueFor(view, "onRemoved", [member, view.self]);
    }
}

/** Makes the display manager and attaches `Display` to it. */
export function installDisplays(): void {
    const displayBackend: DisplayBackend = {
        outputs: () => liveOutputs(),
        primary: () => primaryOutput(),
        hasDesk: (output) => std.deskByOutput.has(output as Output),
        shownOn: (output) => {
            const desk = std.deskByOutput.get(output as Output);
            return desk === undefined ? null : effectiveDesk(desk).shown;
        },
        attached: (workspace) => wsState(workspace as Workspace).attached,
        show: (workspace, output) => showWorkspace(workspace as Workspace, { output: output as Output }),
        span: (lead, members) => spanDesks(lead as Output, members as readonly Output[] | null),
        mirrorOf: (output) => (output as Output).mirrorOf,
        mirror: (output, source, path) => requestOutput(output as Output, { mirrorOf: source as Output | null }, path),
        apply: (output, setting, path) => requestOutput(output as Output, setting as OutputConfig, path),
        pending: (output) => [...ctx.applications.values()].some((pending) => pending.output === output),
        say,
        error: (code, target, message) => new GnoblinError(code, target, message),
        outputSet: makeOutputSet,
        listed: (member) => announceMember(member, true),
        unlisted: (member) => announceMember(member, false),
    };

    // Reconciles the script's profiles and displays with the connected outputs.
    std.displayManager = new DisplayManager(
        displayBackend,
        std.profileRoot.list as OutputProfile[],
        std.displayRoot.list as Display[],
    );

    attachDisplays({
        changed: () => std.displayManager.changed(),
        isWorkspace: (value) => value instanceof Workspace,
        isOutput: (value) => value instanceof Output,
        error: (code, target, message) => new GnoblinError(code, target, message),
        shown: (display) => std.displayManager.shown(display),
        show: (display, workspace) => std.displayManager.assignContent(display, workspace),
        outputs: (display) => std.displayManager.outputsOf(display),
    });
}
