// Creates the entry points the shell calls during each frame.

import { deviceDocument, deviceSettingsRevision } from "gnoblin-core/devices";
import { ctx } from "gnoblin-core/host/context";
import type { Entry } from "gnoblin-core/host/contract";
import { next, pending, takeErrors } from "gnoblin-core/host/dispatch";
import { plan, pointer, ready } from "gnoblin-core/host/frame";
import { hooks } from "gnoblin-core/host/hooks";
import { inspect } from "gnoblin-core/inspect";
import { NEVER, NOW, nextWake, onWake } from "gnoblin-core/wake";

/** What the shell calls each frame, over the object model of `gnoblin`. */
export function makeEntry(): Entry {
    // Wake on the next frame when listeners are waiting in the queue.
    onWake("listener queue", () => (ctx.queue.length > 0 ? NOW : NEVER));
    return {
        render: (value, maxBytes) => inspect.render(value, { maxBytes }),
        wake: () => nextWake((owner, error) => ctx.reports.push({ site: `wake source ${owner}`, error })),
        ui: () => hooks.ui(),
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
        layout: () => hooks.layout(),
        finishLoad: () => hooks.finishLoad(),
        snapshot: () => hooks.snapshot(),
        reconcile: (previous) => hooks.reconcile(previous),
        pointer,
        input: (messages) => {
            ctx.input.deliver(messages);
            hooks.inputDelivered();
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
            hooks.release();
            ctx.hostApi.retire();
            return ctx.queue.length;
        },
    };
}
