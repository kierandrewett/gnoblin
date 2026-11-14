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

export type StockLayoutName = "floating" | "master-stack" | "grid" | "monocle";

const STOCK_LAYOUTS: Readonly<Record<StockLayoutName, () => Layout>> = {
    floating: () => new FloatingLayout(),
    "master-stack": () => new MasterStackLayout(),
    grid: () => new GridLayout(),
    monocle: () => new MonocleLayout(),
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

export function rejectUnknown(object: object, known: readonly string[], what: string): void {
    for (const key of Object.keys(object)) {
        if (!known.includes(key)) {
            throw new TypeError(`${what} has no "${key}"; expected one of ${known.join(", ")}`);
        }
    }
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

export interface WorkspaceOptions {
    id: string;
    /** Shown to people; defaults to the id. */
    name?: string;
    /** A stock layout by name, or an instance of any Layout. Default `floating`. */
    layout?: StockLayoutName | Layout;
    gaps?: { inner?: number; outer?: number };
}

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

export function rectsEqual(a: Rect | null, b: Rect | null): boolean {
    return a !== null && b !== null && a.x === b.x && a.y === b.y && a.width === b.width && a.height === b.height;
}

/**
 * What a layout's `compute` returned, as `[node, bounds]` pairs for the nodes that get a rectangle. A result
 * that is not an array, a placement for a node that is not one of `children`, or a rectangle that is not four
 * finite numbers with a size of at least zero is refused before anything is written, so a layout that
 * fails halfway leaves the workspace as it was. A placement without bounds leaves its node where it is.
 */
export function checkPlacements<N extends object>(
    placements: unknown,
    children: readonly N[],
    where: string,
): [N, Rect][] {
    if (!Array.isArray(placements)) {
        throw new TypeError(`${where}: compute must return an array of placements, got ${String(placements)}`);
    }
    const checked: [N, Rect][] = [];
    placements.forEach((placement: Placement<N> | null, i) => {
        if (placement === null || typeof placement !== "object" || !children.includes(placement.node)) {
            throw new TypeError(`${where}: placement ${i} is not for one of the workspace's clients`);
        }
        const bounds = placement.bounds;
        if (bounds === undefined) {
            return;
        }
        for (const key of ["x", "y", "width", "height"] as const) {
            const value = bounds?.[key];
            if (
                typeof value !== "number" ||
                !Number.isFinite(value) ||
                ((key === "width" || key === "height") && value < 0)
            ) {
                throw new TypeError(
                    `${where}: placement ${i} bounds.${key} must be a finite number${key === "width" || key === "height" ? " of at least 0" : ""}`,
                );
            }
        }
        checked.push([placement.node, { x: bounds.x, y: bounds.y, width: bounds.width, height: bounds.height }]);
    });
    return checked;
}
