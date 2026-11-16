// Creates the entry points the shell calls during each frame.

import { deviceDocument, deviceSettingsRevision } from "gnoblin/devices";
import { ctx } from "gnoblin/host/context";
import type { Entry } from "gnoblin/host/contract";
import { next, pending, takeErrors } from "gnoblin/host/dispatch";
import { plan, pointer, ready } from "gnoblin/host/frame";
import { layout } from "gnoblin/host/layout_driver";
import { declareDecorations } from "gnoblin/host/services";
import { reconcileWorkspaces, snapshotWorkspaces } from "gnoblin/host/snapshot";
import { ensureDefaultWorkspace } from "gnoblin/host/workspaces";
import { inspect } from "gnoblin/inspect";
import { lockRelease, runLockFrame } from "gnoblin/lock";
import { sceneReleaseAll } from "gnoblin/scene/registry";
import { uiReleaseAll } from "gnoblin/ui/frame";
import { NEVER, NOW, nextWake, onWake } from "gnoblin/wake";

/** What the shell calls each frame, over the object model of `gnoblin`. */
export function makeEntry(): Entry {
    // Wake on the next frame when listeners are waiting in the queue.
    onWake("listener queue", () => (ctx.queue.length > 0 ? NOW : NEVER));
    return {
        render: (value, maxBytes) => inspect.render(value, { maxBytes }),
        wake: () => nextWake((owner, error) => ctx.reports.push({ site: `wake source ${owner}`, error })),
        ui: () =>
            runLockFrame(
                (site, error) => ctx.reports.push({ site, error }),
                () => {
                    try {
                        ctx.xrRuntime.frame();
                    } catch (error) {
                        ctx.reports.push({ site: "XRRuntime.frame", error });
                    }
                },
            ),
        plan,
        next,
        pending,
        ready,
        takeErrors,
        takeNotices: () => ctx.notices.splice(0),
        document: () => JSON.stringify(deviceDocument()),
        documentRevision: deviceSettingsRevision,
        dialogs: (events) => {
            ctx.dialogs.deliver(events);
            return ctx.queue.length;
        },
        takeVerdicts: () => {
            const taken = ctx.verdicts;
            ctx.verdicts = [];
            return taken;
        },
        dialogsWanted: () => ctx.dialogs.wanted,
        wants: () => (ctx.idleMonitor.wanted ? 1 : 0),
        idleReading: (reading) => {
            ctx.idleMonitor.deliver(reading);
            return ctx.queue.length;
        },
        idleLost: () => {
            ctx.idleMonitor.lost();
            return ctx.queue.length;
        },
        takeIdleCommands: () => ctx.idleMonitor.takeCommands(),
        idleHolders: () => ctx.idleMonitor.holders(),
        idleFailed: (holder) => ctx.idleMonitor.failed(holder),
        layout,
        finishLoad: ensureDefaultWorkspace,
        snapshot: snapshotWorkspaces,
        reconcile: reconcileWorkspaces,
        pointer,
        input: (messages) => {
            ctx.input.deliver(messages);
            try {
                ctx.focusPolicy?.frame();
            } catch (error) {
                ctx.reports.push({ site: "FocusPolicy.frame", error });
            }
            return ctx.queue.length;
        },
        apiFrame: (batch) => {
            // Only what this frame queued: calls an earlier stage left over wait for the frame's own budget.
            const before = ctx.queue.length;
            ctx.hostApi.frame(batch);
            return ctx.queue.length - before;
        },
        cancelAll: () => {
            ctx.input.cancelAll();
            ctx.xrRuntime.release();
            sceneReleaseAll();
            ctx.transitions.release();
            lockRelease();
            uiReleaseAll();
            ctx.hostApi.retire();
            if (ctx.decorations !== null) {
                declareDecorations(false);
            }
            return ctx.queue.length;
        },
    };
}
