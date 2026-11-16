// The desktop's own services: window decorations, the window menu and the not-responding alert, and the lock screen and
// frame host the stock library installs. `startDecorations`, `startWindowMenu` and `startNotRespondingAlert` are library
// code for an `init.ts` that wants them; a config that does not call them has none.

import { notRespondingAlerts } from "gnoblin-std/alerts";
import { errorCodeOfRefusal, GnoblinError } from "gnoblin-core/errors";
import type { FrameHooks } from "gnoblin-std/frame/client";
import { attachFrameHost, Decorations } from "gnoblin-std/frame/decorations";
import type { DecorationsOptions } from "gnoblin-std/frame/decorations";
import { host } from "gnoblin-core/host/bridge";
import type { Client } from "gnoblin-core/host/client";
import { clientHolders, emitMember } from "gnoblin-core/host/collection_events";
import { ctx } from "gnoblin-core/host/context";
import { transferClient } from "gnoblin-std/host/membership";
import { std } from "gnoblin-std/host/state";
import { stateOf } from "gnoblin-core/host/node_state";
import { motion } from "gnoblin-core/host/seat";
import type { Workspace } from "gnoblin-std/host/workspace";
import { attachLock } from "gnoblin-std/lock";
import { windowMenu } from "gnoblin-std/menus/windows";
import { snappedZone, unsnapClient } from "gnoblin-std/snap";
import type { SnapClient } from "gnoblin-std/snap";
import { windowState } from "gnoblin-std/window_state";

/** Provides the menu, stacking, maximise, minimise and focus actions used by window frames. */
function frameHooks(): FrameHooks {
    return {
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
        // A press on the frame is a press on its window: the focus policy hears it as it hears one on the window.
        press: (client, event) => {
            if (stateOf(client as Client).alive) {
                emitMember(client as Client, clientHolders(client as Client), "onPointerPress", [client, event]);
            }
        },
        release: (client, event) => {
            if (stateOf(client as Client).alive) {
                emitMember(client as Client, clientHolders(client as Client), "onPointerRelease", [client, event]);
            }
        },
    };
}

/**
 * Gives every window a frame, once: the stock titlebar with its buttons, the drag and the double click, or the frame
 * the options fill. Library code for an `init.ts` that wants frames; a config that does not call it has none. Calling
 * it again changes nothing. It declares to the compositor that this shell draws decorations.
 */
export function startDecorations(options: DecorationsOptions = {}): void {
    if (std.decorations === null) {
        std.decorations = new Decorations(options);
        ctx.clientRoot.self.addListener(std.decorations);
    }
}

/** Tells the compositor whether this shell draws window decorations, so a client that asks for server-side is told
 * so. */
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

/** Starts the window menu once: it answers the compositor's menu requests. A config that does not call it has none. */
export function startWindowMenu(): void {
    if (windowMenu.started) {
        return;
    }
    windowMenu.start({
        clientOf: (node) => ctx.clientsByNode.get(node),
        workspaces: () => std.workspaceRoot.list as Workspace[],
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

/** Attaches the lock screen and the frame host. */
export function installServices(): void {
    // The stock lock screen is part of the runtime, not of a config: a locked session must always have one to type
    // into.
    attachLock(host.lock);

    // The frames of the windows find a window's node and what their buttons ask of the shell here.
    attachFrameHost({ nodeOf: (client) => stateOf(client as Client).node, hooks: frameHooks() });
}
