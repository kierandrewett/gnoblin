// Exposes session, process, timer and focus APIs to scripts through the host.

import { GnoblinError } from "gnoblin/errors";
import { host } from "gnoblin/host/bridge";
import type { Client } from "gnoblin/host/client";
import type { Collection } from "gnoblin/host/collections";
import { ctx } from "gnoblin/host/context";
import { HostApi } from "gnoblin/session/host_api";

/** Connects the public session APIs to the host. */
export function installHostApi(): void {
    // Session-facing process, runtime, timer, bus, and focus APIs.
    ctx.hostApi = new HostApi(
        {
            schedule: (site, task) => {
                ctx.queue.push({ site, task });
            },
            now: () => host.input.now(),
            spawn: (argv, options) => host.api.spawn(argv, options),
            kill: (id, signal) => host.api.kill(id, signal),
            pid: (id) => host.api.pid(id),
            begin: (verb) => host.api.begin(verb),
            status: () => host.api.status(),
            version: () => host.api.version(),
            operationId: () => ++ctx.flags.operationIds,
            error: (code, target, message) => new GnoblinError(code, target, message),
            // The entry module runs before the first `ready`: a reload asked for from there would ask again after it.
            loading: () => !ctx.flags.opened,
        },
        {
            windows: () => [...ctx.clientRoot.list],
            workspaces: () => [...ctx.workspaceRoot.list],
            outputs: () => [...(ctx.outputRoot.self as Iterable<object>)],
            focusedWindow: () => (ctx.clientRoot.self as Collection<Client>).focused,
        },
    );
}
