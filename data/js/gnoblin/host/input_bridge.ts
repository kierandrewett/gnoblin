// Connects seat input, device settings and toolkit state to the compositor host.

import { attachDeviceSettings } from "gnoblin/devices";
import { GnoblinError } from "gnoblin/errors";
import { host } from "gnoblin/host/bridge";
import type { Client } from "gnoblin/host/client";
import { ctx } from "gnoblin/host/context";
import { workAreaOf } from "gnoblin/host/work_area";
import type { Workspace } from "gnoblin/host/workspace";
import { wsState } from "gnoblin/host/workspace_state";
import { InputManager } from "gnoblin/input/manager";
import { attachUi } from "gnoblin/ui/state";

/** Makes the input manager and attaches the device and toolkit backends. */
export function installInput(): void {
    // Seat input manager wired to host focus, window stack, layouts, and the deferred callback queue.
    ctx.input = new InputManager(host.input, {
        clientOf: (node) => ctx.clientsByNode.get(node),
        stack: () => host.windows().flatMap((node) => ctx.clientsByNode.get(node) ?? []),
        workArea: (client) => workAreaOf((client as Client | undefined) ?? null),
        displayAt: (point) => ctx.displayManager.displayAt(point),
        layouts: () => ctx.workspaceRoot.list.map((workspace) => wsState(workspace as Workspace).layout),
        schedule: (site, task) => {
            ctx.queue.push({ site, task });
        },
        report: (site, error) => {
            ctx.reports.push({ site, error });
        },
        error: (code, target, message) => new GnoblinError(code, target, message),
    });

    attachDeviceSettings({
        check: (patch) => host.config.check(patch),
        error: (code, target, message) => new GnoblinError(code, target, message),
    });

    attachUi({ ...host.ui, now: () => host.input.now() }, (site, error) => {
        ctx.reports.push({ site, error });
    });
}
