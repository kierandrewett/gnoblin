import { Layout } from "gnoblin-std/layout";
import type { Gaps } from "gnoblin-std/layout";
import type { Rect } from "gnoblin-core/types";
/** Stock layout names accepted by `new Workspace({ layout })`. */
export type StockLayoutName = "floating" | "master-stack" | "grid" | "monocle" | "dwindle" | "kiosk" | "strip" | "carousel-3d";
/** A stock layout by name, an instance of any Layout as it is, or a floating layout for `undefined`. */
export declare function resolveLayout(layout: StockLayoutName | Layout | undefined, where: string): Layout;
/** The Levenshtein distance of two strings: the fewest single-character edits that turn one into the other. */
export declare function editDistance(from: string, to: string): number;
/**
 * The valid key nearest to `key` within two edits (the first one on a tie), or `undefined` when none is that close.
 * Case, dashes and underscores do not count as edits, so `window-management` is `windowManagement`.
 */
export declare function nearestKey(key: string, known: readonly string[]): string | undefined;
/**
 * Throws a TypeError for the first key of `object` that is not in `known`. The message names the place (`what`),
 * the key, the nearest valid key when one is within two edits, and every valid key. `hints` give a sentence for a
 * name that is not a typo but a different thing, such as the old `shortcuts` section; it wins over a nearest key.
 */
export declare function rejectUnknown(object: object, known: readonly string[], what: string, hints?: Readonly<Record<string, string>>): void;
/** The workspace's gaps as two finite numbers of at least zero; a side left out is zero. */
export declare function checkGaps(gaps: {
    inner?: number;
    outer?: number;
} | undefined, what?: string): Gaps;
/** The options `new Workspace(...)` takes. Only `id` is required. */
export interface WorkspaceOptions {
    /** Unique workspace identifier. */
    id: string;
    /** Shown to people; defaults to the id. */
    name?: string;
    /** A stock layout name or a layout instance. Defaults to `floating`. */
    layout?: StockLayoutName | Layout;
    /** Inner and outer spacing in logical pixels. Each defaults to zero. */
    gaps?: {
        inner?: number;
        outer?: number;
    };
}
/** `WorkspaceOptions` after `checkWorkspaceOptions`: every field present and valid. */
export interface CheckedWorkspaceOptions {
    /** Unique workspace identifier. */
    id: string;
    /** Display name, defaulting to the identifier. */
    name: string;
    /** Resolved layout instance. */
    layout: Layout;
    /** Checked inner and outer spacing in logical pixels. */
    gaps: Gaps;
}
/** The options of `new Workspace(...)`, checked, with the defaults filled in. */
export declare function checkWorkspaceOptions(options: WorkspaceOptions): CheckedWorkspaceOptions;
/** Chooses the workspace a new window joins: the active workspace, or the first if none is active. */
export declare function initialWorkspace<W extends {
    readonly active: boolean;
}>(workspaces: Iterable<W>): W | null;
/** The own enumerable fields of a layout, by key. Private (`#x`) fields and prototype members are not fields. */
export declare function layoutFields(layout: object): Map<PropertyKey, unknown>;
/**
 * Reports whether layout fields were added, removed or changed between snapshots. Comparison is shallow (`Object.is`),
 * so edits inside an existing value such as an array are not detected; assign a new array instead.
 */
export declare function fieldsChanged(before: ReadonlyMap<PropertyKey, unknown> | null, now: ReadonlyMap<PropertyKey, unknown>): boolean;
/** Whether two rectangles are the same. A null side is never equal, so a missing area always counts as changed. */
export declare function rectsEqual(a: Rect | null, b: Rect | null): boolean;
/** A placement after the checks, with every default filled in. */
export interface CheckedPlacement<N extends object> {
    /** Layout node receiving this placement. */
    node: N;
    /** `null` leaves the node where it is. */
    bounds: Rect | null;
    /** Whether the node is visible. Defaults to `true`. */
    visible: boolean;
    /** Whether the node is managed by the layout. Defaults to `false`. */
    tiled: boolean;
    /** Stacking depth. Defaults to zero. */
    z: number;
    /** Rotation in radians around the x, y and z axes. */
    rotation: readonly [number, number, number];
    /** Scale on the x, y and z axes. */
    scale: readonly [number, number, number];
    /** Offset from the bounds centre in logical pixels, or `null` when the transform gives no position. */
    offset: {
        x: number;
        y: number;
    } | null;
}
/** The rotation of a placement that gives none: no rotation about any axis, in radians. */
export declare const NO_ROTATION: readonly [0, 0, 0];
/** The scale of a placement that gives none: 1 along every axis. */
export declare const UNIT_SCALE: readonly [1, 1, 1];
/**
 * What a layout's `compute` returned, with the defaults filled in, for every placement. A result that is not an
 * array, a placement for a node that is not one of `children`, a rectangle that is not four finite numbers with a
 * size of at least zero, or a flag, depth or vector of the wrong type is refused before anything is written, so a
 * layout that fails halfway leaves the workspace as it was. A placement without bounds leaves its node where it is.
 */
export declare function checkPlacements<N extends object>(placements: unknown, children: readonly N[], where: string): CheckedPlacement<N>[];
