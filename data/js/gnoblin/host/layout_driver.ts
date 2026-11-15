// The layout pass: for each workspace that changed, hand its layout the clients and area, check what it returns,
// and write the placements. Runs once per frame after the listeners.

import { host } from "gnoblin/host/bridge";
import type { Client } from "gnoblin/host/client";
import { ctx } from "gnoblin/host/context";
import { deskShowing, fillDesks } from "gnoblin/host/desks";
import { guard } from "gnoblin/host/dispatch";
import { followBar, refitClient } from "gnoblin/host/fitting";
import { stateOf } from "gnoblin/host/node_state";
import { applyPlacements, startSlides, syncVisibility } from "gnoblin/host/placements";
import { focusedClient } from "gnoblin/host/seat";
import { applyStyles } from "gnoblin/host/styles";
import { areaOfDesk, screenArea } from "gnoblin/host/work_area";
import type { Workspace } from "gnoblin/host/workspace";
import { wsState } from "gnoblin/host/workspace_state";
import type { LayoutContext } from "gnoblin/layout";
import { flushBindings } from "gnoblin/properties";
import { checkPlacements, fieldsChanged, layoutFields, rectsEqual } from "gnoblin/workspace";

/**
 * The layout driver, run once at the end of each frame after every listener. For each workspace it first
 * notices what changed: a layout's own enumerable fields (a snapshot diff, because a field is a plain
 * property the host never sees written), the workspace's gaps, and the area. Then the layout hears of the
 * windows that came and went, and the shown workspace, when it is dirty, computes once and its placements
 * are written. A workspace that is not shown stays dirty until it is.
 */
export function layout(): number {
    // Bindings first, so what they write is laid out in the same frame.
    guard("gnoblin.properties", flushBindings);
    guard("gnoblin.lostWrites", relayLostWrites);
    ctx.flags.laying = true;
    try {
        return layoutWorkspaces();
    } finally {
        ctx.flags.laying = false;
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
        const workspace = client === undefined ? undefined : ctx.membership.get(client);
        if (client === undefined || workspace === undefined) {
            continue;
        }
        ctx.writtenVisible.delete(client);
        wsState(workspace).dirty = true;
    }
}

/** Runs one guarded layout pass over workspaces and returns the number recomputed. */
function layoutWorkspaces(): number {
    ctx.displayManager.frame();
    fillDesks();
    // A workspace no output shows hears its hooks for the primary output's area; one an output shows, for that output's.
    const fallback = screenArea();
    let laidOut = 0;
    for (const workspace of ctx.workspaceRoot.list as Workspace[]) {
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
                if (client.alive && ctx.membership.get(client) === workspace) {
                    guard(`${workspace}.layout.onClientAdded`, () => target.onClientAdded?.(client, context));
                }
            }
            // A bar that came or went since a window was last fitted moves its content, before the layout places the
            // windows it owns; a window that has just arrived is placed with the bar it has, so it only records it.
            guard(`${workspace}.followBar`, () => {
                for (const client of children) {
                    if (client.alive) {
                        followBar(client, area);
                    }
                }
            });
            if (desk !== null && (state.dirty || !rectsEqual(state.area, area))) {
                const before = state.area;
                state.dirty = false;
                state.area = { ...area };
                if (before !== null && !rectsEqual(before, area)) {
                    // The work area moved under the windows (a bar came, went or grew): a maximised window fills it
                    // again, a snapped one tiles to its zone again and a floating one is brought back inside, as the the retired shell
                    // `Desk:arrange` did. The layout places what it owns after this.
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
                    // A client the script floats (a window rule, `client.floating`) keeps its own place, and so does one
                    // that covers the output (fullscreen) or fills the work area (maximised). A dialog stays with its
                    // parent, centred in it, and is never tiled.
                    const tiled = children.filter(
                        (client) =>
                            !stateOf(client).floating &&
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
            // What the hooks and compute did to the layout's own fields is not a change for the next frame.
            state.fields = layoutFields(state.layout);
        });
    }
    ctx.flags.lastFocus = focusedClient();
    startSlides();
    guard("gnoblin.workspaces.visibility", syncVisibility);
    guard("gnoblin.style", applyStyles);
    return laidOut;
}
