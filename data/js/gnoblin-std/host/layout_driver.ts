// Recomputes changed workspaces and applies their window placements once per frame, after listeners.

import { host } from "gnoblin-core/host/bridge";
import type { Client } from "gnoblin-core/host/client";
import { ctx } from "gnoblin-core/host/context";
import { std } from "gnoblin-std/host/state";
import { deskShowing, fillDesks } from "gnoblin-std/host/desks";
import { guard } from "gnoblin-core/host/dispatch";
import { followFrame, refitClient } from "gnoblin-std/host/fitting";
import { stateOf } from "gnoblin-core/host/node_state";
import { applyPlacements, startSlides, syncVisibility } from "gnoblin-std/host/placements";
import { focusedClient } from "gnoblin-core/host/seat";
import { applyStyles } from "gnoblin-std/host/styles";
import { areaOfDesk, screenArea } from "gnoblin-std/host/work_area";
import type { Workspace } from "gnoblin-std/host/workspace";
import { wsState } from "gnoblin-std/host/workspace_state";
import type { LayoutContext } from "gnoblin-std/layout";
import { flushBindings } from "gnoblin-core/properties";
import { checkPlacements, fieldsChanged, layoutFields, rectsEqual } from "gnoblin-std/workspace";

/**
 * Runs after listeners. It checks layout fields, workspace gaps and the work area, then notifies layouts about window
 * membership and shown workspaces. Dirty, shown workspaces are computed once and their placements are applied. Unshown
 * workspaces remain dirty until shown.
 */
export function layout(): number {
    // Bindings first, so what they write is laid out in the same frame.
    guard("gnoblin.properties", flushBindings);
    guard("gnoblin.lostWrites", relayLostWrites);
    std.flags.laying = true;
    try {
        return layoutWorkspaces();
    } finally {
        std.flags.laying = false;
    }
}

/**
 * Takes the windows whose writes the shell gave up on (the compositor refused them `stale-base` more often than the
 * shell sends a write again) and lays their workspaces out again. A placement is computed only when its workspace is
 * dirty, and the pass cleared that before the write went out, so without this a lost placement would stay lost until
 * something else happened to the workspace. What this module last wrote to the window (its visibility) is forgotten
 * too, as the next pass writes only what differs from it.
 */
function relayLostWrites(): void {
    for (const node of host.lostWrites()) {
        const client = ctx.clientsByNode.get(node);
        const workspace = client === undefined ? undefined : std.membership.get(client);
        if (client === undefined || workspace === undefined) {
            continue;
        }
        std.writtenVisible.delete(client);
        wsState(workspace).dirty = true;
    }
}

/** Runs one guarded layout pass over workspaces and returns the number recomputed. */
function layoutWorkspaces(): number {
    std.displayManager.frame();
    fillDesks();
    // An unshown workspace uses the primary output's work area for its hooks.
    const fallback = screenArea();
    let laidOut = 0;
    for (const workspace of std.workspaceRoot.list as Workspace[]) {
        const state = wsState(workspace);
        const desk = deskShowing(workspace);
        const area = desk === null ? fallback : areaOfDesk(desk);
        guard(`${workspace}.layout`, () => {
            const fields = layoutFields(state.layout);
            if (fieldsChanged(state.fields, fields)) {
                state.dirty = true;
            }
            if (state.gaps.inner !== state.gapsSeen[0] || state.gaps.outer !== state.gapsSeen[1]) {
                state.gapsSeen = [state.gaps.inner, state.gaps.outer];
                state.dirty = true;
            }
            state.fields = fields;
            if (area === null) {
                return;
            }
            const children = [...state.clientsState.list] as Client[];
            const focused = children.find((client) => client.focused) ?? null;
            if (focused !== state.focusSeen) {
                state.focusSeen = focused;
                state.dirty = true;
            }
            const covering = children
                .filter((client) => client.fullscreen || client.maximised)
                .map((client) => stateOf(client).node)
                .join(",");
            if (covering !== state.coverSeen) {
                state.coverSeen = covering;
                state.dirty = true;
            }
            const context: LayoutContext<Client> = {
                workspace,
                children,
                area: { ...area },
                gap: { ...state.gaps },
                focused,
            };
            const target = state.layout;
            for (const client of state.departed.splice(0)) {
                guard(`${workspace}.layout.onClientRemoved`, () => target.onClientRemoved?.(client, context));
            }
            for (const client of state.arrived.splice(0)) {
                if (client.alive && std.membership.get(client) === workspace) {
                    guard(`${workspace}.layout.onClientAdded`, () => target.onClientAdded?.(client, context));
                }
            }
            // Refit changed frames before placing their windows; new arrivals keep their current frame.
            guard(`${workspace}.followFrame`, () => {
                for (const client of children) {
                    if (client.alive && followFrame(client, area)) {
                        state.dirty = true;
                    }
                }
            });
            if (desk !== null && (state.dirty || !rectsEqual(state.area, area))) {
                const before = state.area;
                state.dirty = false;
                state.area = { ...area };
                if (before !== null && !rectsEqual(before, area)) {
                    // When the work area changes, maximised windows fill it, snapped windows retile and floating
                    // windows move inside it. The layout then places the windows it owns.
                    guard(`${workspace}.refit`, () => {
                        for (const client of children) {
                            if (client.alive) {
                                refitClient(client, area);
                            }
                        }
                    });
                    guard(`${workspace}.layout.onAreaChanged`, () => target.onAreaChanged?.(before, { ...area }));
                }
                guard(`${workspace}.layout.compute`, () => {
                    const site = `${workspace}.layout.compute`;
                    // Floating, fullscreen and maximised windows keep their own placement. Dialogs stay with their
                    // parent, are centred in it and are never tiled.
                    const tiled = children.filter(
                        (client) =>
                            !std.floating.has(client) &&
                            !client.fullscreen &&
                            !client.maximised &&
                            client.parentWindow === null,
                    );
                    applyPlacements(
                        tiled,
                        checkPlacements(target.compute({ ...context, children: tiled }), tiled, site),
                        area,
                    );
                });
                laidOut += 1;
            }
            // Record hook and compute changes so they do not trigger another pass next frame.
            state.fields = layoutFields(state.layout);
        });
    }
    std.flags.lastFocus = focusedClient();
    startSlides();
    guard("gnoblin.workspaces.visibility", syncVisibility);
    guard("gnoblin.style", applyStyles);
    return laidOut;
}
