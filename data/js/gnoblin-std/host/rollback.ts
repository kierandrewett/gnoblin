// Snapshots state outside the compositor tree and installs staged transition handling.

import { GnoblinError } from "gnoblin-core/errors";
import { ctx } from "gnoblin-core/host/context";
import { std } from "gnoblin-std/host/state";
import { layout } from "gnoblin-std/host/layout_driver";
import { everyStyle } from "gnoblin-std/host/style_scopes";
import type { Workspace } from "gnoblin-std/host/workspace";
import { wsState } from "gnoblin-std/host/workspace_state";
import { snapshotStyles } from "gnoblin-std/style/snapshot";
import { TransitionEngine } from "gnoblin-std/transition/engine";
import { layoutFields } from "gnoblin-std/workspace";

/**
 * What a transition callback can change that is not a tree write, put back when the transition fails: each workspace's
 * layout, gaps, name and dirty flag, the own fields of the layout it had, and every style. A layout the callback
 * assigned (and so was not the workspace's layout when the transition began) keeps the field changes made to it.
 */
function snapshotScriptState(): () => void {
    if (std.flags.laying) {
        throw new GnoblinError("busy", "gnoblin.requestTransition", "cannot start inside a layout pass");
    }
    const restores: (() => void)[] = [];
    for (const workspace of std.workspaceRoot.list as Workspace[]) {
        const state = wsState(workspace);
        const { layout: held, gaps, name, dirty, fields, gapsSeen } = state;
        const own = layoutFields(held);
        restores.push(() => {
            Object.assign(state, { layout: held, gaps, name, dirty, fields, gapsSeen });
            for (const key of Reflect.ownKeys(held)) {
                if (Object.getOwnPropertyDescriptor(held, key)?.enumerable && !own.has(key)) {
                    Reflect.deleteProperty(held, key);
                }
            }
            for (const [key, value] of own) {
                Reflect.set(held, key, value);
            }
        });
    }
    restores.push(snapshotStyles(everyStyle));
    restores.push(std.displayManager.snapshot());
    const shownBefore = std.desks.map((desk) => [desk, desk.shown] as const);
    restores.push(() => {
        for (const [desk, shown] of shownBefore) {
            desk.shown = shown;
            if (shown !== null) {
                wsState(shown).dirty = true;
            }
        }
    });
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
