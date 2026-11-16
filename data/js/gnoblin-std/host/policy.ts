// Installs the stock window-request, gesture, move-mode and focus policies.

import { FocusPolicy } from "gnoblin-std/focus";
import { gestureDirection, onPinch, onSwipe } from "gnoblin-std/gesture_actions";
import type { GestureActions } from "gnoblin-std/gesture_actions";
import { host } from "gnoblin-core/host/bridge";
import type { Client } from "gnoblin-core/host/client";
import { clientHolders } from "gnoblin-core/host/collection_events";
import { ctx } from "gnoblin-core/host/context";
import { currentDesk, isShown, primaryDesk, primaryOutput } from "gnoblin-std/host/desks";
import { describeListener, handlerOf } from "gnoblin-core/host/dispatch";
import type { Source } from "gnoblin-core/host/dispatch";
import { stateOf } from "gnoblin-core/host/node_state";
import type { Output } from "gnoblin-core/host/output";
import { currentFocusSettings, focusedClient, hasFocus, mainSeat } from "gnoblin-core/host/seat";
import { showWorkspace, SLIDE_DURATION } from "gnoblin-std/host/show";
import { framed, outputRectOf, workAreaOf } from "gnoblin-std/host/work_area";
import type { Workspace } from "gnoblin-std/host/workspace";
import { wsState } from "gnoblin-std/host/workspace_state";
import { repositioning } from "gnoblin-std/reposition";
import { snappedZone, unsnapClient } from "gnoblin-std/snap";
import type { SnapClient } from "gnoblin-std/snap";
import { barsGiveWayTo, barsOnOutputs } from "gnoblin-std/widgets";
import { windowState } from "gnoblin-std/window_state";
import type { ClientRequest } from "gnoblin-std/window_state";

/** Whether a non-minimised fullscreen window is shown on the workspace for `output`. */
function fullscreenShowing(output: { alive: boolean } | null): boolean {
    const desk = output === null ? primaryDesk() : (ctx.deskByOutput.get(output as Output) ?? null);
    const shown = desk === null || desk.parked ? null : desk.shown;
    if (shown === null) {
        return false;
    }
    return (wsState(shown).clientsState.list as Client[]).some((client) => client.fullscreen && !client.minimised);
}

/** Calls the `onRequest` of the client's own listeners, then of the collections that hold it, until one handles it. */
function dispatchRequest(client: Client, request: ClientRequest<Client>): void {
    const sources: Source[] = [stateOf(client), ...clientHolders(client)];
    for (const source of sources) {
        for (const listener of [...source.listeners]) {
            const handler = handlerOf(listener, "onRequest");
            if (typeof handler !== "function") {
                continue;
            }
            try {
                handler.call(listener, client, request);
            } catch (error) {
                ctx.reports.push({ site: `${describeListener(listener)}.onRequest`, error });
            }
            if (request.handled) {
                return;
            }
        }
    }
}

/** Shows the workspace `step` places from the one the output under the pointer shows; nothing past either end. */
function stepWorkspace(step: number): void {
    const shown = currentDesk()?.shown ?? null;
    if (shown === null) {
        return;
    }
    const target = ctx.workspaceRoot.list[ctx.workspaceRoot.list.indexOf(shown) + step];
    if (target !== undefined) {
        showWorkspace(target as Workspace, { duration: SLIDE_DURATION });
    }
}

/** Attaches the stock focus policy to every client. */
export function installFocus(): void {
    const focus = new FocusPolicy<Client>({
        settings: currentFocusSettings,
        focused: hasFocus,
        // A tree with no seat has nowhere to put the focus.
        focus: (client) => {
            if (host.input.seats().length > 0) {
                client.focus();
            }
        },
        clearFocus: () => ctx.input.primary.clearFocus(),
        raise: (client) => client.raise(),
        now: () => host.input.now(),
        isArrival: (client) => !ctx.preexisting.has(client),
        fromLauncher: (client) => client.fromLauncher,
        alive: (client) => client.alive,
    });
    ctx.focusPolicy = focus;
    ctx.clientRoot.self.addListener(focus);
}

/** Attaches the stock answers to window requests, gestures and the keyboard move mode. */
export function installPolicy(): void {
    barsGiveWayTo(fullscreenShowing);

    barsOnOutputs(primaryOutput);

    // The five actions run by touchpad gestures, also exposed through `gnoblin.gestures`.
    const gestureActions: GestureActions = {
        workspaceNext: () => stepWorkspace(1),
        workspacePrevious: () => stepWorkspace(-1),
        windowClose: () => focusedClient()?.close(),
        windowMinimise: () => {
            const client = focusedClient();
            if (client !== null) {
                windowState.minimise(client);
            }
        },
        windowToggleMaximise: () => {
            const client = focusedClient();
            if (client === null) {
                return;
            }
            if (client.maximised) {
                windowState.unmaximise(client);
            } else {
                windowState.maximise(client);
            }
        },
    };

    // `gnoblin.gestures`: the five actions and the helpers that bind them to a swipe or a pinch.
    ctx.gestureTools = {
        actions: gestureActions,
        direction: gestureDirection,
        onSwipe: (map, threshold) => onSwipe(map, threshold, { actions: gestureActions }),
        onPinch: (map, threshold) => onPinch(map, threshold, { actions: gestureActions }),
    };

    // Attach the stock responses to window requests as soon as the module starts.
    windowState.start<Client>({
        clientOf: (node) => ctx.clientsByNode.get(node),
        siblings: (client) => {
            const own = ctx.membership.get(client) ?? null;
            return host
                .windows()
                .flatMap((node) => ctx.clientsByNode.get(node) ?? [])
                .filter((other) => (ctx.membership.get(other) ?? null) === own);
        },
        raise: (client) => client.raise(),
        lower: (client) => client.lower(),
        close: (client) => client.close(),
        focused: focusedClient,
        focus: (client) => {
            // A tree with no seat has nowhere to put the focus.
            if (host.input.seats().length === 0) {
                return;
            }
            if (client === null) {
                ctx.input.primary.clearFocus();
            } else {
                client.focus();
            }
        },
        onActiveWorkspace: (client) => {
            const own = ctx.membership.get(client);
            return own !== undefined && isShown(own);
        },
        output: (client) => outputRectOf(client ?? null),
        workArea: (client) => workAreaOf(client ?? null),
        framed,
        snapped: (client) => snappedZone(client) !== null,
        unsnap: (client) => {
            unsnapClient(client as unknown as SnapClient);
        },
        dispatch: dispatchRequest,
        report: (site, error) => {
            ctx.reports.push({ site, error });
        },
        now: () => host.input.now(),
        dragConstraint: () => {
            const seat = host.input.seats()[0];
            return seat === undefined ? undefined : host.input.dragConstraint(seat);
        },
        edgeTiling: () => {
            const seat = host.input.seats()[0];
            return seat !== undefined && host.input.settings(seat)?.edgeTiling === true;
        },
    });

    // Start keyboard move and resize mode with the desktop work area.
    repositioning.start({
        workArea: (client) => workAreaOf(client as unknown as Client),
        dragConstraint: () => {
            const seat = mainSeat();
            return seat === undefined ? undefined : host.input.dragConstraint(seat);
        },
        focus: (client) => (client as unknown as Client).focus(),
    });
}
