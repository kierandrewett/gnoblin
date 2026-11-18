// Snapshots state outside the compositor tree and installs staged transition handling.

import { GnoblinError } from "gnoblin-core/errors";
import { ctx } from "gnoblin-core/host/context";
import { std } from "gnoblin-std/host/state";
import { layout } from "gnoblin-std/host/frame";
import { snapshotWorkspaceState } from "gnoblin-std/host/restore";
import { everyStyle } from "gnoblin-std/host/style_scopes";
import { snapshotStyles } from "gnoblin-std/style/snapshot";
import { TransitionEngine } from "gnoblin-std/transition/engine";

/**
 * What a transition callback can change that is not a tree write, put back when the transition fails: the workspaces (their
 * layouts, gaps and names, and which one each output shows, `host/restore`) and every style.
 */
function snapshotScriptState(): () => void {
    const restores: (() => void)[] = [snapshotWorkspaceState()];
    restores.push(snapshotStyles(everyStyle));
    restores.push(std.displayManager.snapshot());
    return () => {
        for (const restore of restores) {
            restore();
        }
    };
}

/**
 * Runs the layout and style passes once against the staged state, so their writes are staged with the callback's. A
 * placement a layout got wrong is a report the pass would log; here it fails the transition, which is what a
 * transition with an invalid target does.
 */
function settleStaged(): void {
    const before = ctx.reports.length;
    layout();
    const failed = ctx.reports.splice(before);
    if (failed.length > 0) {
        throw failed[0].error;
    }
}

/** Installs the engine behind `requestTransition`: the callback runs over the library's staging, the passes below run at its end. */
export function installTransitions(): void {
    std.transitions = new TransitionEngine({
        staging: std.staging,
        schedule: (site, task) => {
            ctx.queue.push({ site, task });
        },
        snapshot: snapshotScriptState,
        settle: settleStaged,
    });
}
