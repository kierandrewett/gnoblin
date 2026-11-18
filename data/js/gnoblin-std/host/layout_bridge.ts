// What the library asks of a workspace's layout: the `Layout` object a script gave a workspace, behind the table of functions the
// library calls (`generated/Layout`, the vtable of a foreign layout).
//
// The library places the windows (`libgnoblin-std`'s layout pass), so a layout is asked in the library's turn, with the windows
// as node numbers. This module gives it the `Client` objects, checks what it answers the way every placement was always checked
// (`checkPlacements`), and tells the library when the layout's own fields changed so the workspace computes again. A failure of
// the layout is reported to the shell (`ctx.reports`) as a layout's failure always was, and the library writes nothing for it.
//
// The stock floating layout is not asked at all: it is the library's own (`LayoutSpec.floating`), and the bridge only gives the
// library its two options.

import type { Client } from "gnoblin-core/host/client";
import { ctx } from "gnoblin-core/host/context";
import { stateOf } from "gnoblin-core/host/node_state";
import { LayoutKind, PlaceKind, PlacementFlags } from "gnoblin-std/generated/constants";
import type { Init } from "gnoblin-std/generated/native";
import type {
    LayoutContext as NativeContext,
    LayoutSpec,
    Placement as NativePlacement,
} from "gnoblin-std/generated/records";
import type { Layout as LayoutVtable } from "gnoblin-std/generated/records";
import { std } from "gnoblin-std/host/state";
import type { Layout, LayoutContext } from "gnoblin-std/layout";
import { FloatingLayout } from "gnoblin-std/layouts/floating";
import { checkPlacements, fieldsChanged, layoutFields } from "gnoblin-std/workspace";
import type { CheckedPlacement } from "gnoblin-std/workspace";
import type { Workspace } from "gnoblin-std/host/workspace";

/** What a workspace holds of the table it gave the library for its layout. */
export interface LayoutBridge {
    /** What the library takes as the workspace's layout. */
    readonly spec: LayoutSpec;
    /** Whether the library has to be given the layout again: the options of the library's floating layout changed. */
    stale(): boolean;
    /** Whether the layout's own fields changed since it was last asked: the workspace has to compute again. */
    changed(): boolean;
}

const PLACEMENTS: Readonly<Record<string, number>> = {
    smart: PlaceKind.SMART,
    center: PlaceKind.CENTER,
    cascade: PlaceKind.CASCADE,
};

/** The client object of a node: a live window, or one that closed while the library was still telling a layout. */
function clientOf(node: number): Client | undefined {
    return ctx.clientsByNode.get(node) ?? std.closed.get(node)?.client;
}

/** The context a layout reads, with the workspace's windows as the objects a script knows. */
function contextOf(cx: NativeContext): LayoutContext<Client> {
    const children = cx.children.flatMap((node) => clientOf(node) ?? []);
    return {
        workspace: std.byHandle.get(cx.workspace),
        children,
        area: { ...cx.area },
        gap: { inner: cx.gapInner, outer: cx.gapOuter },
        focused: cx.focused === 0 ? null : (clientOf(cx.focused) ?? null),
    };
}

/** One placement as the library takes it. */
function placementOf(placement: CheckedPlacement<Client>): Init<NativePlacement> {
    let flags = 0;
    if (placement.bounds !== null) {
        flags |= PlacementFlags.BOUNDS;
    }
    if (!placement.visible) {
        flags |= PlacementFlags.HIDDEN;
    }
    if (placement.tiled) {
        flags |= PlacementFlags.TILED;
    }
    if (placement.offset !== null) {
        flags |= PlacementFlags.OFFSET;
    }
    return {
        flags,
        node: stateOf(placement.node).node,
        bounds: placement.bounds ?? { x: 0, y: 0, width: 0, height: 0 },
        z: placement.z,
        rotationX: placement.rotation[0],
        rotationY: placement.rotation[1],
        rotationZ: placement.rotation[2],
        scaleX: placement.scale[0],
        scaleY: placement.scale[1],
        scaleZ: placement.scale[2],
        offsetX: placement.offset?.x ?? 0,
        offsetY: placement.offset?.y ?? 0,
    };
}

/** Runs a hook of the layout; a failure is the shell's to report, and the pass goes on. */
function guarded(site: string, hook: () => void): void {
    try {
        hook();
    } catch (error) {
        ctx.reports.push({ site, error });
    }
}

/** The table the library calls for a layout a script wrote, and how to ask whether the layout's fields changed. */
function vtableOf(layout: Layout, owner: () => Workspace | undefined): { vtable: LayoutVtable; changed(): boolean } {
    const site = (part: string): string => `${owner()}.layout${part}`;
    let seen = layoutFields(layout);
    let revision = 0;
    // The layout's own fields are what it is: a change of one is a new revision, as it always made the workspace compute again.
    const changed = (): boolean => {
        const now = layoutFields(layout);
        if (!fieldsChanged(seen, now)) {
            return false;
        }
        seen = now;
        revision += 1;
        return true;
    };
    const vtable: LayoutVtable = {
        compute(cx) {
            const at = site(".compute");
            try {
                const context = contextOf(cx);
                return checkPlacements(layout.compute(context), context.children, at).map(placementOf);
            } catch (error) {
                ctx.reports.push({ site: at, error });
                throw error;
            }
        },
        revision() {
            changed();
            return revision;
        },
    };
    if (typeof layout.onClientAdded === "function") {
        vtable.clientAdded = (cx, node) => {
            const client = clientOf(node);
            if (client !== undefined) {
                guarded(site(".onClientAdded"), () => layout.onClientAdded?.(client, contextOf(cx)));
            }
        };
    }
    if (typeof layout.onClientRemoved === "function") {
        vtable.clientRemoved = (cx, node) => {
            const client = clientOf(node);
            if (client !== undefined) {
                guarded(site(".onClientRemoved"), () => layout.onClientRemoved?.(client, contextOf(cx)));
            }
        };
    }
    if (typeof layout.onAreaChanged === "function") {
        vtable.areaChanged = (before, now) => {
            guarded(site(".onAreaChanged"), () => layout.onAreaChanged?.({ ...before }, { ...now }));
        };
    }
    if (typeof layout.onClientsSwapped === "function") {
        vtable.clientsSwapped = (a, b) => {
            const [first, second] = [clientOf(a), clientOf(b)];
            if (first !== undefined && second !== undefined) {
                guarded(site(".onClientsSwapped"), () => layout.onClientsSwapped?.(first, second));
            }
        };
    }
    return { vtable, changed };
}

/** The library's floating layout with the options of `layout`; the same words `place` would have refused. */
function floatingSpec(layout: FloatingLayout): LayoutSpec {
    const placement = PLACEMENTS[layout.placement];
    if (placement === undefined) {
        throw new TypeError(
            `FloatingLayout placement must be smart, center or cascade, got ${String(layout.placement)}`,
        );
    }
    if (!Number.isFinite(layout.cascadeStep) || layout.cascadeStep < 1) {
        throw new RangeError("FloatingLayout cascadeStep must be a finite number of at least 1");
    }
    return { kind: LayoutKind.FLOATING, placement, cascadeStep: layout.cascadeStep, vtable: null };
}

/**
 * What a workspace gives the library for `layout`. The stock floating layout is the library's own; any other layout, a
 * subclass of it included, is a foreign one the library asks through `owner`'s windows.
 */
export function bridgeLayout(layout: Layout, owner: () => Workspace | undefined): LayoutBridge {
    if (layout.constructor === FloatingLayout) {
        const floating = layout as FloatingLayout;
        const signature = (): string => `${floating.placement}|${floating.cascadeStep}`;
        let given = signature();
        return {
            spec: floatingSpec(floating),
            stale() {
                const now = signature();
                if (now === given) {
                    return false;
                }
                given = now;
                return true;
            },
            changed: () => false,
        };
    }
    const { vtable, changed } = vtableOf(layout, owner);
    return {
        spec: { kind: LayoutKind.FOREIGN, placement: 0, cascadeStep: 0, vtable },
        stale: () => false,
        changed,
    };
}
