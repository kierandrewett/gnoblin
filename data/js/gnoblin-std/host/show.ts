// Shows workspaces on outputs. The desks, the swap of two outputs' workspaces and the slide between them are the library's; this
// checks what a script gave and puts the words of the script's errors on the library's refusals.

import { GnoblinError } from "gnoblin-core/errors";
import { isOutput } from "gnoblin-core/host/node_state";
import type { Output } from "gnoblin-core/host/output";
import { motion } from "gnoblin-core/host/seat";
import { FAILED, nativeOutput, throughLibrary } from "gnoblin-std/host/library";
import { syncMembership } from "gnoblin-std/host/membership";
import { std } from "gnoblin-std/host/state";
import type { Workspace } from "gnoblin-std/host/workspace";
import { wsState } from "gnoblin-std/host/workspace_state";

/** Default duration, in milliseconds, of a workspace slide. */
export const SLIDE_DURATION = 250;

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
export function showWorkspace(workspace: Workspace | string, options: ShowOptions = {}): void {
    const target = typeof workspace === "string" ? std.workspaceRoot.list.find((w) => w.id === workspace) : workspace;
    if (target === undefined) {
        throw new GnoblinError("not_found", "gnoblin.workspaces.show", `there is no workspace "${workspace}"`);
    }
    const state = wsState(target);
    if (state.native.info().attached === 0) {
        throw new GnoblinError("invalid_state", `${target}.show`, "add the workspace to gnoblin.workspaces first");
    }
    const asked = options.duration ?? 0;
    const progress = options.progress ?? 0;
    if (typeof asked !== "number" || !Number.isFinite(asked) || asked < 0) {
        throw new TypeError(`${target}.show: duration must be a finite number of milliseconds of at least 0`);
    }
    if (typeof progress !== "number" || !(progress >= 0 && progress <= 1)) {
        throw new TypeError(`${target}.show: progress must be a number from 0 to 1`);
    }
    if (options.output !== undefined && !isOutput(options.output)) {
        throw new TypeError(`${target}.show: output must be an Output`);
    }
    if (options.output !== undefined && !options.output.alive) {
        throw new GnoblinError("not_found", `${target}.show`, `${options.output} is gone`);
    }
    throughLibrary(`${target}.show`, { [FAILED.refused]: "unavailable" }, () =>
        state.native.show({
            durationMs: motion(asked),
            progress,
            output: options.output === undefined ? 0 : nativeOutput(options.output).node,
        }),
    );
    syncMembership();
}
