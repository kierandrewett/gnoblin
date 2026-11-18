// ArcLayout places nodes around the viewer, with the focused node at the front. It uses transform placements: positions
// on a circle in the x-z plane and turns about the vertical axis that face each node towards the centre. See
// `gnoblin-std/layout` for how a flat workspace uses `Placement.transform`. The geometry is the library's
// (`libgnoblin-std`, kind `arc`); this class holds the options.

import { BuiltinKind } from "gnoblin-std/generated/constants";
import type { LayoutContext, LayoutNode } from "gnoblin-std/layout";
import { NativeLayout } from "gnoblin-std/layouts/native";
import type { OptionSpecs } from "gnoblin-std/layouts/native";

/** Options of `ArcLayout`. */
export interface ArcOptions {
    /** The distance from the centre of the arc to each node, in the scene's units. Default 1200. */
    radius?: number;
    /** The angle between neighbouring nodes in radians. Default 0.35. */
    spacing?: number;
}

const OPTIONS: OptionSpecs = {
    radius: { native: "radius" },
    spacing: { native: "spacing" },
};

/** The checks the constructor always made, which need no library; the library checks again when it makes its layout. */
function check(name: string, value: unknown, positive: boolean): number {
    if (typeof value !== "number" || !Number.isFinite(value) || (positive && value <= 0)) {
        throw new TypeError(
            `ArcLayout: ${name} must be a finite number${positive ? " above 0" : ""}, got ${String(value)}`,
        );
    }
    return value;
}

/** What a View shows, so a layout of Views follows the focus of the client behind each. Undefined for other nodes. */
function contentOf(node: LayoutNode): unknown {
    return "content" in node ? node.content : undefined;
}

/**
 * Places the nodes on an arc: node `i` at the angle `(i - selected) * spacing`, where `selected` is the focused node's
 * index (0 when none is focused). Its position is `(sin(angle) * radius, 0, -cos(angle) * radius)` and it is turned
 * `-angle` about Y, so the arc is concave towards a viewer at its centre. `radius` and `spacing` are plain fields: a
 * change recomputes the layout, and a `requestTransition` that assigns them animates the placements.
 *
 * On a flat workspace the placements are the node's offset from the centre of the work area (`radius * sin(angle)`
 * across), its depth `z`, its turn about Y and its scale; see `Placement.transform`.
 */
export class ArcLayout extends NativeLayout {
    /** Distance from the centre of the arc to each node, in scene units. Default 1200. */
    radius: number;
    /** Angle between neighbouring nodes, in radians. Default 0.35. */
    spacing: number;

    constructor(options: ArcOptions = {}) {
        super();
        this.radius = check("radius", options.radius ?? 1200, true);
        this.spacing = check("spacing", options.spacing ?? 0.35, false);
    }

    protected get kind(): number {
        return BuiltinKind.ARC;
    }

    protected get options(): OptionSpecs {
        return OPTIONS;
    }

    /** The child that is the focused node, or whose `content` is: a layout of Views follows the focus of the client behind each. */
    protected override focusedChild(context: LayoutContext): LayoutNode | undefined {
        const { focused, children } = context;
        if (focused === null || focused === undefined) {
            return undefined;
        }
        return children.find((node) => node === focused || contentOf(node) === focused);
    }
}
