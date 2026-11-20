import type { Display } from "gnoblin-std/display/display";
import { MutableCollection } from "gnoblin-core/host/collections";
import type { ShowOptions } from "gnoblin-std/host/show";
import type { Workspace } from "gnoblin-std/host/workspace";
import type { Rect } from "gnoblin-core/types";
/** The workspaces of the script, which also say which of them each output shows. */
export declare class WorkspaceCollection extends MutableCollection<Workspace> {
    /** The area windows are laid out in on the primary output; null while the tree has none. */
    get area(): Rect | null;
    /**
     * Shows a workspace, given as itself or by id, on `options.output`, or on the output under the pointer (the primary
     * output when no pointer says). With `options.duration` the windows slide from one workspace to the other; without
     * it the switch is at once.
     */
    show(workspace: Workspace | string, options?: ShowOptions): void;
}
/** `gnoblin.displays`: the logical displays, those of the script and the ones the shell keeps for the other outputs. */
export declare class DisplayCollection extends MutableCollection<Display> {
    /**
     * Swaps `old` for `next` as one validated change: `old` and any other script display that shares an output with
     * `next` leave, and `next` takes `old`'s place. Nothing changes when the check fails.
     */
    replace(old: Display, next: Display): void;
}
/** Creates the root collections of the package for the current generation. */
export declare function installRoots(): void;
