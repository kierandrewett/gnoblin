import { Collection, Workspace } from "gnoblin-std/emitted/classes";
import type { Layout } from "gnoblin-std/emitted/records";
import type { LayoutContext, Placement, Rect } from "gnoblin-std/emitted/records";
import type { Init } from "gnoblin-std/emitted/native";
export * from "gnoblin-std/emitted";
/** Every client of the compositor, in the order they appeared. */
export declare function clients(): Collection;
/** Every connected output. */
export declare function outputs(): Collection;
/** The workspaces the shell added, in order. */
export declare function workspaces(): Collection;
/**
 * A tiled placement of `node` at whole pixels. Each of the four edges is floored on its own, and the size is the distance between two
 * floored edges, so a master of 607.2 and a stack of 404.8 that starts at 619.2 are 607 and 404 wide, and the stack still ends at 1023
 * in an area that ends at 1024. This is the rule of the Rust SDK's `Placement::tile` and of the library's tiling layouts.
 */
export declare function tile(node: number, x: number, y: number, width: number, height: number): Init<Placement>;
/** The workspace's area less the outer gap on every side: where a tiling layout works. */
export declare function insetArea(cx: LayoutContext): Rect;
/** The options of {@link createWorkspace}. */
export interface WorkspaceOptions {
    name?: string;
    /** A layout of your own (an object with `compute`), or nothing for the library's floating layout. */
    layout?: Layout;
    /** The gap between tiles, then between the tiles and the edge of the area. */
    gaps?: [number, number];
}
/** Makes a workspace, gives it its layout and adds it to the list. */
export declare function createWorkspace(id: string, options?: WorkspaceOptions): Workspace;
