// The root collections: `gnoblin.clients`, `workspaces`, `outputs`, `displays` and the output profiles. Each is made
// once per generation by `installRoots` and kept on the context.

import type { Display } from "gnoblin/display/display";
import type { OutputProfile } from "gnoblin/display/output_profile";
import { GnoblinError } from "gnoblin/errors";
import { CREATE } from "gnoblin/host/bridge";
import type { Client } from "gnoblin/host/client";
import { collectionState, rootState } from "gnoblin/host/collection_state";
import { Collection, MutableCollection } from "gnoblin/host/collections";
import { ctx } from "gnoblin/host/context";
import type { Output } from "gnoblin/host/output";
import { showWorkspace } from "gnoblin/host/show";
import type { ShowOptions } from "gnoblin/host/show";
import { screenArea } from "gnoblin/host/work_area";
import type { Workspace } from "gnoblin/host/workspace";
import { addWorkspace, removeWorkspace } from "gnoblin/host/workspaces";
import type { Rect } from "gnoblin/types";
import { VirtualOutput } from "gnoblin/virtual_output";

/** The workspaces of the script, which also say which of them each output shows. */
export class WorkspaceCollection extends MutableCollection<Workspace> {
    /** The area windows are laid out in on the primary output; null while the tree has none. */
    get area(): Rect | null {
        return screenArea();
    }

    /**
     * Shows a workspace, given as itself or by id, on `options.output`, or on the output under the pointer (the primary
     * output when no pointer says). With `options.duration` the windows slide from one workspace to the other; without
     * it the switch is at once.
     */
    show(workspace: Workspace | string, options?: ShowOptions): void {
        showWorkspace(workspace, options);
    }
}

/**
 * `gnoblin.outputs.add`: a script describes a `VirtualOutput` and the compositor would create it. The compositor makes
 * outputs only for the displays and test sessions it owns and has no request that creates one (the work is listed in
 * design/js-spec-native.md, "XR extension"), so the only honest answer today is a typed refusal.
 */
function addVirtualOutput(member: unknown): void {
    if (!(member instanceof VirtualOutput)) {
        throw new TypeError("gnoblin.outputs.add: expected a VirtualOutput; the compositor makes every other output");
    }
    throw new GnoblinError("unsupported", "Collection.add", "virtual outputs are not supported by this backend");
}

/** `gnoblin.outputs`: the connected outputs, and the profiles that say how each is configured. */
export class OutputCollection extends MutableCollection<Output> {
    /** The persistent hardware settings: an `OutputProfile` applies when its monitor is connected, also after a hotplug. */
    readonly profiles = ctx.profileRoot.self as MutableCollection<OutputProfile>;
}

/** `gnoblin.displays`: the logical displays, those of the script and the ones the shell keeps for the other outputs. */
export class DisplayCollection extends MutableCollection<Display> {
    /**
     * Swaps `old` for `next` as one validated change: `old` and any other script display that shares an output with
     * `next` leave, and `next` takes `old`'s place. Nothing changes when the check fails.
     */
    replace(old: Display, next: Display): void {
        ctx.displayManager.replaceDisplay(old, next);
    }
}

/** Makes the root collections of the generation. */
export function installRoots(): void {
    // All native clients in compositor order.
    ctx.clientRoot = collectionState(new Collection<Client>(CREATE, rootState("Client")));

    // Hardware profiles in `gnoblin.outputs.profiles`; membership is script-owned and applied by the display manager.
    ctx.profileRoot = collectionState(
        new MutableCollection<OutputProfile>(CREATE, {
            ...rootState("OutputProfile"),
            mutation: {
                add: (profile) => ctx.displayManager.addProfile(profile as OutputProfile),
                remove: (profile) => ctx.displayManager.removeProfile(profile as OutputProfile),
                clear: () => {
                    for (const profile of [...ctx.profileRoot.list] as OutputProfile[]) {
                        ctx.displayManager.removeProfile(profile);
                    }
                },
            },
        }),
    );

    // Connected outputs in compositor creation order. A script may add a `VirtualOutput`; it may remove nothing.
    ctx.outputRoot = collectionState(
        new OutputCollection(CREATE, {
            ...rootState("Output"),
            mutation: {
                add: addVirtualOutput,
                remove: () => false,
                clear: () => {
                    throw new GnoblinError("read_only", "Collection.clear", "outputs belong to the compositor");
                },
            },
        }),
    );

    // Script workspaces and their named sets; membership is script-owned.
    ctx.workspaceRoot = collectionState(
        new WorkspaceCollection(CREATE, {
            ...rootState("Workspace"),
            mutation: {
                add: (workspace) => addWorkspace(workspace as Workspace),
                // The last workspace stays: a shell with nowhere to put a window is unusable (Before: `remove_workspace` refuses).
                remove: (workspace) => {
                    if (ctx.workspaceRoot.list.length < 2) {
                        return false;
                    }
                    removeWorkspace(workspace as Workspace);
                    return true;
                },
                clear: () => {
                    for (const workspace of [...ctx.workspaceRoot.list]) {
                        removeWorkspace(workspace as Workspace);
                    }
                },
            },
        }),
    );

    ctx.displayRoot = collectionState(
        new DisplayCollection(CREATE, {
            ...rootState("Display"),
            mutation: {
                add: (display) => ctx.displayManager.addDisplay(display as Display),
                remove: (display) => ctx.displayManager.removeDisplay(display as Display),
                clear: () => ctx.displayManager.clearDisplays(),
            },
        }),
    );
}
