import type { LayoutSpec } from "gnoblin-std/generated/records";
import type { Layout } from "gnoblin-std/layout";
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
/**
 * What a workspace gives the library for `layout`. The stock floating layout is the library's own; any other layout, a
 * subclass of it included, is a foreign one the library asks through `owner`'s windows.
 */
export declare function bridgeLayout(layout: Layout, owner: () => Workspace | undefined): LayoutBridge;
