// The generated TypeScript binding of `libgnoblin-std`, and the few spellings that are not in the table.
//
// `gnoblin-std/generated` is written by `cargo xtask gen-sdks` from the library's function table: a class per object of the table
// (`Collection`, `Workspace`, `Client`, `Transition`, `Style`, `Scene`), every record with its conversions, every constant group, the
// typed class of every built-in layout and binding. They call `__gnoblinStd`, which `libgnoblin-runtime-js` installs. This file is the
// ergonomic layer over them: the roots of the model, the tile rounding of the tiling layouts, and a workspace made in one call.
// Nothing in it is behaviour of the desktop; that is in the library.
import { Collection, Workspace } from "gnoblin-std/generated/classes";
import type { Layout } from "gnoblin-std/generated/records";
import { LayoutSpec, WorkspaceSpec } from "gnoblin-std/generated/records";
import type { LayoutContext, Placement, Rect } from "gnoblin-std/generated/records";
import { CollectionKind, PlacementFlags } from "gnoblin-std/generated/constants";
import type { Init } from "gnoblin-std/generated/native";

export * from "gnoblin-std/generated";

/** Every client of the compositor, in the order they appeared. */
export function clients(): Collection {
    return Collection.root(CollectionKind.CLIENTS) as Collection;
}

/** Every connected output. */
export function outputs(): Collection {
    return Collection.root(CollectionKind.OUTPUTS) as Collection;
}

/** The workspaces the shell added, in order. */
export function workspaces(): Collection {
    return Collection.root(CollectionKind.WORKSPACES) as Collection;
}

/** The error a floor of float arithmetic is allowed: the right edge of 619.2 + 404.8 is 1023.9999999999999. */
const EDGE_EPSILON = 1e-6;

/**
 * A tiled placement of `node` at whole pixels. Each of the four edges is floored on its own, and the size is the distance between two
 * floored edges, so a master of 607.2 and a stack of 404.8 that starts at 619.2 are 607 and 404 wide, and the stack still ends at 1023
 * in an area that ends at 1024. This is the rule of the Rust SDK's `Placement::tile` and of the library's tiling layouts.
 */
export function tile(node: number, x: number, y: number, width: number, height: number): Init<Placement> {
    const left = Math.floor(x + EDGE_EPSILON);
    const top = Math.floor(y + EDGE_EPSILON);
    return {
        flags: PlacementFlags.BOUNDS | PlacementFlags.TILED,
        node,
        bounds: {
            x: left,
            y: top,
            width: Math.floor(x + width + EDGE_EPSILON) - left,
            height: Math.floor(y + height + EDGE_EPSILON) - top,
        },
    };
}

/** The workspace's area less the outer gap on every side: where a tiling layout works. */
export function insetArea(cx: LayoutContext): Rect {
    const gap = cx.gapOuter;
    return { x: cx.area.x + gap, y: cx.area.y + gap, width: cx.area.width - 2 * gap, height: cx.area.height - 2 * gap };
}

/** The options of {@link createWorkspace}. */
export interface WorkspaceOptions {
    name?: string;
    /** A layout of your own (an object with `compute`), or nothing for the library's floating layout. */
    layout?: Layout;
    /** The gap between tiles, then between the tiles and the edge of the area. */
    gaps?: [number, number];
}

/** Makes a workspace, gives it its layout and adds it to the list. */
export function createWorkspace(id: string, options: WorkspaceOptions = {}): Workspace {
    const [inner, outer] = options.gaps ?? [0, 0];
    const spec: Init<WorkspaceSpec> = {
        id,
        name: options.name ?? "",
        gapInner: inner,
        gapOuter: outer,
        layout: options.layout === undefined ? null : { ...LayoutSpec.foreign(), vtable: options.layout },
    };
    const workspace = Workspace.new(spec) as Workspace;
    workspace.add();
    return workspace;
}
