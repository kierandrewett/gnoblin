// Installs the services the compositor's own surfaces need: the dialog, idle, authentication and launch managers. They
// start with the runtime.

import { AuthenticationManager } from "gnoblin-core/authentication";
import { DialogManager } from "gnoblin-core/dialogs";
import { host } from "gnoblin-core/host/bridge";
import { Client } from "gnoblin-core/host/client";
import { ctx } from "gnoblin-core/host/context";
import { ClientListener, CollectionListener } from "gnoblin-core/host/listeners";
import { IdleMonitor } from "gnoblin-core/idle";
import { LaunchFeedback } from "gnoblin-core/launches";

/**
 * Tells `launches` of the windows that appear, take focus or name their application. The listeners are attached
 * before any launch can begin, so the windows that exist now answer none.
 */
export function trackLaunches(): void {
    const arrivals = new CollectionListener();
    arrivals.onAdded = (member) => {
        if (member instanceof Client) {
            ctx.launches.answered(member.appId);
        }
    };
    ctx.clientRoot.self.addListener(arrivals);
    const changes = new ClientListener();
    changes.onFocusChanged = (client, focused) => {
        if (focused) {
            ctx.launches.answered(client.appId);
        }
    };
    changes.onAppIdChanged = (client, appId) => {
        ctx.launches.answered(appId);
    };
    ctx.clientRoot.self.addListener(changes);
}

/** Makes the dialog, idle, authentication and launch managers. */
export function installServices(): void {
    // Dialog request manager backed by the shell callback queue.
    ctx.dialogs = new DialogManager({
        schedule: (site, task) => {
            ctx.queue.push({ site, task });
        },
        verdict: (request, deny, reason) => {
            ctx.verdicts.push({ request, deny, reason });
        },
    });

    // Idle watches and holds scheduled alongside regular listeners.
    ctx.idleMonitor = new IdleMonitor({
        schedule: (site, task) => {
            ctx.queue.push({ site, task });
        },
    });

    // Authentication manager connected to compositor-owned dialog observations.
    ctx.authentication = new AuthenticationManager(ctx.dialogs);

    // Launch feedback manager resolved by windows associated with each launch.
    ctx.launches = new LaunchFeedback({
        now: () => host.input.now(),
        schedule: (site, task) => {
            ctx.queue.push({ site, task });
        },
    });
}
