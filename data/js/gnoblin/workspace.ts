// What a workspace needs that does not touch the host: the stock layouts by name, the option checks, the
// rule that picks the workspace a new window joins, the snapshot diff that notices a change to a layout's
// own fields, and the check of what `compute` returned. The `Workspace` class itself is in the host module
// (`gnoblin/host`), because its `clients` collection is one of the host's live collections.
//
// This module is not re-exported from `gnoblin`: a config reaches workspaces through `Workspace` and
// `gnoblin.workspaces`, and the host and `defineConfig` import the helpers by name.

import { Layout } from "gnoblin/layout";
import type { Gaps, Placement, Rect } from "gnoblin/layout";
import { FloatingLayout } from "gnoblin/layouts/floating";
import { GridLayout } from "gnoblin/layouts/grid";
import { MasterStackLayout } from "gnoblin/layouts/master-stack";
import { MonocleLayout } from "gnoblin/layouts/monocle";
import { DwindleLayout } from "gnoblin/layouts/dwindle";
import { KioskLayout } from "gnoblin/layouts/kiosk";
import { transformFrom } from "gnoblin/spatial";
import { Carousel3dLayout, StripLayout } from "gnoblin/layouts/strip";

/** The names `new Workspace({ layout })` takes for a stock layout. */
export type StockLayoutName =
    "floating" | "master-stack" | "grid" | "monocle" | "dwindle" | "kiosk" | "strip" | "carousel-3d";

/** One factory per stock layout name, so that two workspaces never share a layout instance. */
const STOCK_LAYOUTS: Readonly<Record<StockLayoutName, () => Layout>> = {
    floating: () => new FloatingLayout(),
    "master-stack": () => new MasterStackLayout(),
    grid: () => new GridLayout(),
    monocle: () => new MonocleLayout(),
    dwindle: () => new DwindleLayout(),
    kiosk: () => new KioskLayout(),
    strip: () => new StripLayout(),
    "carousel-3d": () => new Carousel3dLayout(),
};

/** A stock layout by name, an instance of any Layout as it is, or a floating layout for `undefined`. */
export function resolveLayout(layout: StockLayoutName | Layout | undefined, where: string): Layout {
    if (layout === undefined) {
        return STOCK_LAYOUTS.floating();
    }
    if (layout instanceof Layout) {
        return layout;
    }
    if (typeof layout === "string" && Object.hasOwn(STOCK_LAYOUTS, layout)) {
        return STOCK_LAYOUTS[layout]();
    }
    throw new TypeError(
        `${where}.layout must be a Layout or one of ${Object.keys(STOCK_LAYOUTS).join(", ")}, got ${String(layout)}`,
    );
}

/** The most edits between a typo and a key for the key to be suggested. */
const MAX_SUGGESTION_DISTANCE = 2;

/** The Levenshtein distance of two strings: the fewest single-character edits that turn one into the other. */
export function editDistance(from: string, to: string): number {
    let previous = Array.from({ length: to.length + 1 }, (_, i) => i);
    for (let i = 1; i <= from.length; i++) {
        const row = [i];
        for (let j = 1; j <= to.length; j++) {
            const same = from[i - 1] === to[j - 1] ? 0 : 1;
            row.push(Math.min(previous[j] + 1, row[j - 1] + 1, previous[j - 1] + same));
        }
        previous = row;
    }
    return previous[to.length];
}

/**
 * The valid key nearest to `key` within two edits (the first one on a tie), or `undefined` when none is that close.
 * Case, dashes and underscores do not count as edits, so `window-management` is `windowManagement`.
 */
export function nearestKey(key: string, known: readonly string[]): string | undefined {
    const strip = (text: string) => text.toLowerCase().replaceAll(/[-_]/g, "");
    let best: string | undefined;
    let bestDistance = MAX_SUGGESTION_DISTANCE + 1;
    for (const candidate of known) {
        const distance = editDistance(strip(key), strip(candidate));
        if (distance < bestDistance) {
            best = candidate;
            bestDistance = distance;
        }
    }
    return best;
}

/**
 * Throws a TypeError for the first key of `object` that is not in `known`. The message names the place (`what`),
 * the key, the nearest valid key when one is within two edits, and every valid key. `hints` give a sentence for a
 * name that is not a typo but a different thing, such as the old `shortcuts` section; it wins over a nearest key.
 */
export function rejectUnknown(
    object: object,
    known: readonly string[],
    what: string,
    hints: Readonly<Record<string, string>> = {},
): void {
    for (const key of Object.keys(object)) {
        if (!known.includes(key)) {
            const advice = adviceFor(key, known, hints);
            throw new TypeError(`${what} has no "${key}"${advice}; expected one of ${known.join(", ")}`);
        }
    }
}

/** The parenthesised advice for an unknown `key`: its hint, else the nearest valid key, else nothing. */
function adviceFor(key: string, known: readonly string[], hints: Readonly<Record<string, string>>): string {
    if (Object.hasOwn(hints, key)) {
        return ` (${hints[key]})`;
    }
    const nearest = nearestKey(key, known);
    return nearest === undefined ? "" : ` (did you mean "${nearest}"?)`;
}

/** The workspace's gaps as two finite numbers of at least zero; a side left out is zero. */
export function checkGaps(gaps: { inner?: number; outer?: number } | undefined, what = "gaps"): Gaps {
    if (gaps === undefined) {
        return { inner: 0, outer: 0 };
    }
    rejectUnknown(gaps, ["inner", "outer"], what);
    const read = (name: "inner" | "outer"): number => {
        const value = gaps[name] ?? 0;
        if (typeof value !== "number" || !Number.isFinite(value) || value < 0) {
            throw new RangeError(`${what}.${name} must be a finite number of at least 0, got ${String(value)}`);
        }
        return value;
    };
    return { inner: read("inner"), outer: read("outer") };
}

/** The options `new Workspace(...)` takes. Only `id` is required. */
export interface WorkspaceOptions {
    id: string;
    /** Shown to people; defaults to the id. */
    name?: string;
    /** A stock layout by name, or an instance of any Layout. Default `floating`. */
    layout?: StockLayoutName | Layout;
    gaps?: { inner?: number; outer?: number };
}

/** `WorkspaceOptions` after `checkWorkspaceOptions`: every field present and valid. */
export interface CheckedWorkspaceOptions {
    id: string;
    name: string;
    layout: Layout;
    gaps: Gaps;
}

/** The options of `new Workspace(...)`, checked, with the defaults filled in. */
export function checkWorkspaceOptions(options: WorkspaceOptions): CheckedWorkspaceOptions {
    if (options === null || typeof options !== "object") {
        throw new TypeError("new Workspace: the options must be an object with an id");
    }
    rejectUnknown(options, ["id", "name", "layout", "gaps"], "Workspace options");
    if (typeof options.id !== "string" || options.id === "") {
        throw new TypeError("Workspace options.id must be a non-empty string");
    }
    if (options.name !== undefined && typeof options.name !== "string") {
        throw new TypeError("Workspace options.name must be a string");
    }
    return {
        id: options.id,
        name: options.name ?? options.id,
        layout: resolveLayout(options.layout, "Workspace options"),
        gaps: checkGaps(options.gaps, "Workspace options.gaps"),
    };
}

/**
 * The workspace a window that has none joins: the one the primary display shows. For now that is the first
 * workspace flagged active, and when none is (a collection that has not shown any yet) the first workspace.
 * Every place that needs this answer asks here, so that the rule can change in one place.
 */
export function initialWorkspace<W extends { readonly active: boolean }>(workspaces: Iterable<W>): W | null {
    let first: W | null = null;
    for (const workspace of workspaces) {
        if (workspace.active) {
            return workspace;
        }
        first ??= workspace;
    }
    return first;
}

/** The own enumerable fields of a layout, by key. Private (`#x`) fields and prototype members are not fields. */
export function layoutFields(layout: object): Map<PropertyKey, unknown> {
    const fields = new Map<PropertyKey, unknown>();
    for (const key of Reflect.ownKeys(layout)) {
        if (Object.getOwnPropertyDescriptor(layout, key)?.enumerable) {
            fields.set(key, Reflect.get(layout, key));
        }
    }
    return fields;
}

/**
 * Whether two snapshots of a layout's fields differ: a field added, removed or given another value. The
 * comparison is shallow (`Object.is`), so a field edited in place, such as an array's element, is not seen;
 * assign a new array instead.
 */
export function fieldsChanged(
    before: ReadonlyMap<PropertyKey, unknown> | null,
    now: ReadonlyMap<PropertyKey, unknown>,
): boolean {
    if (before === null || before.size !== now.size) {
        return true;
    }
    for (const [key, value] of now) {
        if (!before.has(key) || !Object.is(before.get(key), value)) {
            return true;
        }
    }
    return false;
}

/** Whether two rectangles are the same. A null side is never equal, so a missing area always counts as changed. */
export function rectsEqual(a: Rect | null, b: Rect | null): boolean {
    return a !== null && b !== null && a.x === b.x && a.y === b.y && a.width === b.width && a.height === b.height;
}

/** A placement after the checks, with every default filled in. */
export interface CheckedPlacement<N extends object> {
    node: N;
    /** `null` leaves the node where it is. */
    bounds: Rect | null;
    visible: boolean;
    tiled: boolean;
    z: number;
    rotation: readonly [number, number, number];
    scale: readonly [number, number, number];
    /** How far a `transform` moves the node from the centre of its bounds, in pixels, or null when it gives no position. */
    offset: { x: number; y: number } | null;
}

/** The rotation of a placement that gives none: no rotation about any axis, in radians. */
export const NO_ROTATION = [0, 0, 0] as const;
/** The scale of a placement that gives none: 1 along every axis. */
export const UNIT_SCALE = [1, 1, 1] as const;

function checkVector(value: unknown, fallback: readonly [number, number, number], what: string) {
    if (value === undefined) {
        return fallback;
    }
    if (
        !Array.isArray(value) ||
        value.length !== 3 ||
        value.some((n) => typeof n !== "number" || !Number.isFinite(n))
    ) {
        throw new TypeError(`${what} must be three finite numbers`);
    }
    return [value[0], value[1], value[2]] as const;
}

/**
 * `bounds` as a copy, after a check that it is four finite numbers with a width and a height of at least zero.
 * A script can pass `null` here, which the `?.` turns into the same error as a missing number.
 */
function checkBounds(bounds: Rect, at: string): Rect {
    for (const key of ["x", "y", "width", "height"] as const) {
        const value = bounds?.[key];
        const isSize = key === "width" || key === "height";
        if (typeof value !== "number" || !Number.isFinite(value) || (isSize && value < 0)) {
            throw new TypeError(`${at} bounds.${key} must be a finite number${isSize ? " of at least 0" : ""}`);
        }
    }
    const { x, y, width, height } = bounds;
    return { x, y, width, height };
}

/**
 * What a layout's `compute` returned, with the defaults filled in, for every placement. A result that is not an
 * array, a placement for a node that is not one of `children`, a rectangle that is not four finite numbers with a
 * size of at least zero, or a flag, depth or vector of the wrong type is refused before anything is written, so a
 * layout that fails halfway leaves the workspace as it was. A placement without bounds leaves its node where it is.
 */
export function checkPlacements<N extends object>(
    placements: unknown,
    children: readonly N[],
    where: string,
): CheckedPlacement<N>[] {
    if (!Array.isArray(placements)) {
        throw new TypeError(`${where}: compute must return an array of placements, got ${String(placements)}`);
    }
    return placements.map((placement: Placement<N> | null, i) => {
        if (placement === null || typeof placement !== "object" || !children.includes(placement.node)) {
            throw new TypeError(`${where}: placement ${i} is not for one of the workspace's clients`);
        }
        const at = `${where}: placement ${i}`;
        const bounds = placement.bounds === undefined ? null : checkBounds(placement.bounds, at);
        for (const flag of ["visible", "tiled"] as const) {
            if (placement[flag] !== undefined && typeof placement[flag] !== "boolean") {
                throw new TypeError(`${at} ${flag} must be true or false`);
            }
        }
        if (placement.z !== undefined && (typeof placement.z !== "number" || !Number.isFinite(placement.z))) {
            throw new TypeError(`${at} z must be a finite number`);
        }
        const moved = placement.transform === undefined ? null : transformFrom(`${at} transform`, placement.transform);
        return {
            node: placement.node,
            bounds,
            visible: placement.visible ?? true,
            tiled: placement.tiled ?? false,
            z: placement.z ?? moved?.position?.[2] ?? 0,
            rotation:
                placement.rotation === undefined
                    ? (moved?.rotation ?? NO_ROTATION)
                    : checkVector(placement.rotation, NO_ROTATION, `${at} rotation`),
            scale:
                placement.scale === undefined
                    ? (moved?.scale ?? UNIT_SCALE)
                    : checkVector(placement.scale, UNIT_SCALE, `${at} scale`),
            offset: moved?.position == null ? null : { x: moved.position[0], y: moved.position[1] },
        };
    });
}
