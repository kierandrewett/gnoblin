// The shell's optional services (titlebars, the window menu, the not-responding alert) and the always-on ones
// (lock screen, dialog and idle observation, launch feedback). A script starts the optional ones with
// `startDecorations`, `startWindowMenu` and `startNotRespondingAlert`.

import { notRespondingAlerts } from "gnoblin/alerts";
import { AuthenticationManager } from "gnoblin/authentication";
import { DialogManager } from "gnoblin/dialogs";
import { errorCodeOfRefusal, GnoblinError } from "gnoblin/errors";
import { host } from "gnoblin/host/bridge";
import { Client } from "gnoblin/host/client";
import { clientHolders, emitMember } from "gnoblin/host/collection_events";
import { ctx } from "gnoblin/host/context";
import { ClientListener, CollectionListener } from "gnoblin/host/listeners";
import { transferClient } from "gnoblin/host/membership";
import { stateOf } from "gnoblin/host/node_state";
import { motion } from "gnoblin/host/seat";
import type { Workspace } from "gnoblin/host/workspace";
import { IdleMonitor } from "gnoblin/idle";
import { LaunchFeedback } from "gnoblin/launches";
import { attachLock } from "gnoblin/lock";
import { windowMenu } from "gnoblin/menus/windows";
import { snappedZone, unsnapClient } from "gnoblin/snap";
import type { SnapClient } from "gnoblin/snap";
import { Decorations, defaultTitlebarStyle } from "gnoblin/titlebar";
import { windowState } from "gnoblin/window_state";

/**
 * Puts the stock titlebar on every window, once: the buttons, the drag and the double click. Library code for an
 * `init.ts` that wants titlebars; a config that does not call it has none. Calling it again changes nothing.
 */
export function startDecorations(): void {
    if (ctx.decorations === null) {
        ctx.decorations = new Decorations(
            (client) => stateOf(client as Client).node,
            { ...defaultTitlebarStyle },
            {
                menu: (client) => {
                    if (windowMenu.started) {
                        windowMenu.show(client as Client);
                    }
                },
                lower: (client) => (client as Client).lower(),
                maximise: (client, on) => {
                    if (on) {
                        windowState.maximise(client as Client);
                    } else {
                        windowState.unmaximise(client as Client);
                    }
                },
                minimise: (client) => {
                    windowState.minimise(client as Client);
                },
                motion,
                // A press on the bar is a press on its window: the focus policy hears it as it hears one on the window.
                press: (client, event) => {
                    if (stateOf(client as Client).alive) {
                        emitMember(client as Client, clientHolders(client as Client), "onPointerPress", [
                            client,
                            event,
                        ]);
                    }
                },
                release: (client, event) => {
                    if (stateOf(client as Client).alive) {
                        emitMember(client as Client, clientHolders(client as Client), "onPointerRelease", [
                            client,
                            event,
                        ]);
                    }
                },
            },
        );
        ctx.clientRoot.self.addListener(ctx.decorations);
    }
}

/** Tells the compositor whether this shell draws window decorations, so a client that asks for server-side is told so. */
export function declareDecorations(available: boolean): void {
    const seat = host.input.seats()[0];
    if (seat === undefined) {
        return;
    }
    const code = host.input.declareDecorations(seat, available);
    if (code != null) {
        ctx.reports.push({
            site: "Decorations.declare",
            error: new GnoblinError(
                errorCodeOfRefusal(code),
                "Seat.decorations_available",
                `the host refused the write (${code})`,
            ),
        });
    }
}

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

/** Starts the window menu once: it answers the compositor's menu requests. A config that does not call it has none. */
export function startWindowMenu(): void {
    if (windowMenu.started) {
        return;
    }
    windowMenu.start({
        clientOf: (node) => ctx.clientsByNode.get(node),
        workspaces: () => ctx.workspaceRoot.list as Workspace[],
        moveToWorkspace: (client, workspace) => transferClient(workspace as Workspace, client as Client),
        stack: () => host.windows().flatMap((node) => ctx.clientsByNode.get(node) ?? []),
        raise: (client) => (client as Client).raise(),
        maximise: (client, on) => {
            if (on) {
                windowState.maximise(client as Client);
            } else {
                windowState.unmaximise(client as Client);
            }
        },
        minimise: (client) => {
            windowState.minimise(client as Client);
        },
        leaveFullscreen: (client) => {
            windowState.leaveFullscreen(client as Client);
        },
        snapped: (client) => snappedZone(client) !== null,
        unsnap: (client) => {
            unsnapClient(client as unknown as SnapClient);
        },
    });
}

/** Starts the not-responding alert once: the dialog for a window that stops answering. */
export function startNotRespondingAlert(): void {
    if (!notRespondingAlerts.started) {
        notRespondingAlerts.start({
            clients: () => ctx.clientRoot.list as Client[],
            closeMenu: () => {
                windowMenu.close();
            },
        });
    }
}

/** Attaches the lock screen and makes the dialog, idle, authentication and launch managers. */
export function installServices(): void {
    // The stock lock screen is part of the runtime, not of a config: a locked session must always have one to type into.
    attachLock(host.lock);

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
