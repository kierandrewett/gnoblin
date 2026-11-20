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
/**
 * Places the nodes on an arc: node `i` at the angle `(i - selected) * spacing`, where `selected` is the focused node's
 * index (0 when none is focused). Its position is `(sin(angle) * radius, 0, -cos(angle) * radius)` and it is turned
 * `-angle` about Y, so the arc is concave towards a viewer at its centre. `radius` and `spacing` are plain fields: a
 * change recomputes the layout, and a `requestTransition` that assigns them animates the placements.
 *
 * On a flat workspace the placements are the node's offset from the centre of the work area (`radius * sin(angle)`
 * across), its depth `z`, its turn about Y and its scale; see `Placement.transform`.
 */
export declare class ArcLayout extends NativeLayout {
    /** Distance from the centre of the arc to each node, in scene units. Default 1200. */
    radius: number;
    /** Angle between neighbouring nodes, in radians. Default 0.35. */
    spacing: number;
    constructor(options?: ArcOptions);
    protected get kind(): number;
    protected get options(): OptionSpecs;
    /** The child that is the focused node, or whose `content` is: a layout of Views follows the focus of the client behind each. */
    protected focusedChild(context: LayoutContext): LayoutNode | undefined;
}
