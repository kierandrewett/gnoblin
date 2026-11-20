import type { Output } from "gnoblin-core/host/output";
import type { Workspace } from "gnoblin-std/host/workspace";
/** Default duration, in milliseconds, of a workspace slide. */
export declare const SLIDE_DURATION = 250;
/** Options for showing a workspace. */
export interface ShowOptions {
    /** Milliseconds the slide between the two workspaces takes. Zero or absent switches at once. */
    duration?: number;
    /** How much of the slide a touchpad gesture has made already, 0 to 1: the slide carries on from there. */
    progress?: number;
    /** The output to show it on. Left out, the one under the pointer, else the primary one. */
    output?: Output;
}
/** Shows a workspace by object or id, optionally sliding from the current workspace. */
export declare function showWorkspace(workspace: Workspace | string, options?: ShowOptions): void;
