// What a transition callback can change that is not a tree write, and the engine that stages and animates its
// writes.

import { errorCodeOfRefusal, GnoblinError } from "gnoblin/errors";
import { host, nodeName } from "gnoblin/host/bridge";
import { ANIMATED_KEYS } from "gnoblin/host/client";
import { ctx } from "gnoblin/host/context";
import { layout } from "gnoblin/host/layout_driver";
import { motion } from "gnoblin/host/seat";
import { everyStyle } from "gnoblin/host/style_scopes";
import type { Workspace } from "gnoblin/host/workspace";
import { wsState } from "gnoblin/host/workspace_state";
import { snapshotStyles } from "gnoblin/style/snapshot";
import { TransitionEngine } from "gnoblin/transition/engine";
import { layoutFields } from "gnoblin/workspace";

/**
 * What a transition callback can change that is not a tree write, put back when the transition fails: each workspace's
 * layout, gaps, name and dirty flag, the own fields of the layout it had, and every style. A layout the callback
 * assigned (and so was not the workspace's layout when the transition began) keeps the field changes made to it.
 */
function snapshotScriptState(): () => void {
    if (ctx.flags.laying) {
        throw new GnoblinError("busy", "gnoblin.requestTransition", "cannot start inside a layout pass");
    }
    const restores: (() => void)[] = [];
    for (const workspace of ctx.workspaceRoot.list as Workspace[]) {
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
    restores.push(ctx.displayManager.snapshot());
    const shownBefore = ctx.desks.map((desk) => [desk, desk.shown] as const);
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

/** Makes the transition engine. */
export function installTransitions(): void {
    // Transition engine that stages writes, validates placements, and drives animation tracks.
    ctx.transitions = new TransitionEngine({
        staging: ctx.staging,
        motion,
        animated: (key) => Object.hasOwn(ANIMATED_KEYS, key),
        placed: (node) =>
            (host.get(node, "width") as number) > 0 &&
            (host.get(node, "height") as number) > 0 &&
            host.get(node, "visible") !== false,
        label: nodeName,
        makeTween: () => {
            const made = host.tween.create();
            if (typeof made !== "number") {
                throw new GnoblinError(
                    errorCodeOfRefusal(String(made)),
                    "gnoblin.requestTransition",
                    "the shell could not make the node that holds the transition's play state",
                );
            }
            return made;
        },
        dropTween: (tween) => {
            host.tween.destroy(tween);
        },
        schedule: (site, task) => {
            ctx.queue.push({ site, task });
        },
        snapshot: snapshotScriptState,
        settle: settleStaged,
        error: (code, target, message) => new GnoblinError(code, target, message),
    });

    ctx.hostApi.onTrackEnded = (event) => ctx.transitions.trackEnded(event);
}
