// Connects seat input, device settings and the property bindings to the compositor host.

import { attachDeviceSettings } from "gnoblin-core/devices";
import { GnoblinError } from "gnoblin-core/errors";
import { host } from "gnoblin-core/host/bridge";
import type { Client } from "gnoblin-core/host/client";
import { ctx } from "gnoblin-core/host/context";
import { hooks } from "gnoblin-core/host/hooks";
import { InputManager } from "gnoblin-core/input/manager";
import { attachBindings } from "gnoblin-core/properties";

/** Makes the input manager and attaches the device and property-binding backends. */
export function installInput(): void {
    // Seat input manager wired to host focus, window stack, work area, and the deferred callback queue. A package that
    // has layouts or a work area says so through the hooks.
    ctx.input = new InputManager(host.input, {
        clientOf: (node) => ctx.clientsByNode.get(node),
        stack: () => host.windows().flatMap((node) => ctx.clientsByNode.get(node) ?? []),
        workArea: (client) => hooks.workArea((client as Client | undefined) ?? undefined),
        layouts: () => hooks.layouts(),
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

    // A binding that reads a property re-runs when the property changes: the reads and writes are told to it.
    attachBindings({
        read: (node, key) =>
            key.startsWith("published:") ? host.published(node, key.slice("published:".length)) : host.get(node, key),
        report: (site, error) => {
            ctx.reports.push({ site, error });
        },
    });
}
