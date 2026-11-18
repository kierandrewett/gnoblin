// The root collections of `gnoblin-std`: `gnoblin.workspaces`, `gnoblin.displays` and `gnoblin.outputs.profiles`. Each is
// made once per generation by `installRoots` and kept on the std context; `gnoblin-core` makes `gnoblin.clients` and
// `gnoblin.outputs`. The list of `gnoblin.workspaces` is the library's (`host/membership` keeps it equal).

import type { Display } from "gnoblin-std/display/display";
import type { OutputProfile } from "gnoblin-std/display/output_profile";
import { CREATE } from "gnoblin-core/host/bridge";
import { rootState, collectionState } from "gnoblin-core/host/collection_state";
import { MutableCollection } from "gnoblin-core/host/collections";
import { std } from "gnoblin-std/host/state";
import { showWorkspace } from "gnoblin-std/host/show";
import type { ShowOptions } from "gnoblin-std/host/show";
import { screenArea } from "gnoblin-std/host/work_area";
import type { Workspace } from "gnoblin-std/host/workspace";
import { addWorkspace, clearWorkspaces, removeWorkspace } from "gnoblin-std/host/workspaces";
import type { Rect } from "gnoblin-core/types";

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

/** `gnoblin.displays`: the logical displays, those of the script and the ones the shell keeps for the other outputs. */
export class DisplayCollection extends MutableCollection<Display> {
    /**
     * Swaps `old` for `next` as one validated change: `old` and any other script display that shares an output with
     * `next` leave, and `next` takes `old`'s place. Nothing changes when the check fails.
     */
    replace(old: Display, next: Display): void {
        std.displayManager.replaceDisplay(old, next);
    }
}

/** Creates the root collections of the package for the current generation. */
export function installRoots(): void {
    // Hardware profiles in `gnoblin.outputs.profiles`; membership is script-owned and applied by the display manager.
    std.profileRoot = collectionState(
        new MutableCollection<OutputProfile>(CREATE, {
            ...rootState("OutputProfile"),
            mutation: {
                add: (profile) => std.displayManager.addProfile(profile as OutputProfile),
                remove: (profile) => std.displayManager.removeProfile(profile as OutputProfile),
                clear: () => {
                    for (const profile of [...std.profileRoot.list] as OutputProfile[]) {
                        std.displayManager.removeProfile(profile);
                    }
                },
            },
        }),
    );

    // The workspaces: their list is the library's, which these calls change.
    std.workspaceRoot = collectionState(
        new WorkspaceCollection(CREATE, {
            ...rootState("Workspace"),
            mutation: {
                add: (workspace) => addWorkspace(workspace as Workspace),
                // The last workspace stays: a shell with nowhere to put a window is unusable, so `remove` refuses
                // to drop it.
                remove: (workspace) => removeWorkspace(workspace as Workspace),
                clear: clearWorkspaces,
            },
        }),
    );

    std.displayRoot = collectionState(
        new DisplayCollection(CREATE, {
            ...rootState("Display"),
            mutation: {
                add: (display) => std.displayManager.addDisplay(display as Display),
                remove: (display) => std.displayManager.removeDisplay(display as Display),
                clear: () => std.displayManager.clearDisplays(),
            },
        }),
    );
}
