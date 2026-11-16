// Styles store per-scope overrides that inherit through the client and workspace layers.
// The resolver combines those layers; the mapper turns the result into node properties and effects.
// Read `docs/js-style.md` for usage.
//
// A scope holds only what was set on it: a property that was never assigned, or was unset, is `undefined` and is
// inherited. `null` is a value (`background = null`). A list assigned to a scope replaces the inherited list whole.
//
// Changes are seen two ways, and no Proxy is involved. A field of an element or a scalar of a style is an accessor
// that checks the value, throws with the property path when it is bad, leaves the old value, and bumps a counter the
// host reads each frame. An array a script changes in place (`push`, `splice`, `style.shadows[0] = {...}`) cannot be
// intercepted, so `syncStyleLists` compares each array with the contents it had last time, once per frame, checks
// the new contents then, and puts the old ones back with a reported error when they are bad.
//
// Reading a list that a scope does not override returns a copy of the inherited list whose elements know they are
// copies. The first write to one of them (or the first in-place change of the array, seen by the frame comparison)
// makes that copy the scope's own list, so the write never reaches the scope the list came from.

import type { Border, BorderLike, Shadow, ShadowLike } from "gnoblin-std/style/border_shadow";
import { assignFields } from "gnoblin-std/style/element";
import type { Filter, FilterInit } from "gnoblin-std/style/filters";
import { getList, pathOf, setList, setScalar, storeOf, unsetOne } from "gnoblin-std/style/lists";
import type { StyleList, StyleProperty, StyleScope } from "gnoblin-std/style/lists";
import { styleState } from "gnoblin-std/style/state";
import type { Color, CornerRadii } from "gnoblin-std/style/values";

/** The properties every `Style` has, and the ones a `ClientStyle` adds. */
const STYLE_FIELDS: readonly string[] = ["opacity", "filter", "backdropFilter"];

const CLIENT_FIELDS: readonly string[] = [...STYLE_FIELDS, "cornerRadius", "borders", "shadows", "background"];

/**
 * A style holds only values set on its scope; unset properties inherit from lower scopes. `undefined` unsets a
 * property, while `null` is a value for `background`.
 */
export class Style {
    constructor(init?: object) {
        styleState.stores.set(this, {
            label: this.constructor.name,
            scope: null,
            scalars: new Map(),
            lists: new Map(),
            seen: new Map(),
            views: new Map(),
        });
        assignFields(this, init, this instanceof ClientStyle ? CLIENT_FIELDS : STYLE_FIELDS, this.constructor.name);
    }

    /** From 0 (invisible) to 1. Written to the window node's `opacity`. */
    get opacity(): number | undefined {
        return storeOf(this).scalars.get("opacity") as number | undefined;
    }

    /** Sets opacity; `undefined` restores inheritance. */
    set opacity(value: number | undefined) {
        setScalar(this, "opacity", value);
    }

    /** The chain applied to the window's own content. No renderer draws it yet. */
    get filter(): StyleList<Filter, FilterInit> {
        return getList(this, "filter") as StyleList<Filter, FilterInit>;
    }

    /** Replaces the content filter chain; `undefined` restores inheritance. */
    set filter(value: FilterInit[] | undefined) {
        setList(this, "filter", value);
    }

    /** The chain applied to what is behind the window. A blur in it is the one the compositor draws. */
    get backdropFilter(): StyleList<Filter, FilterInit> {
        return getList(this, "backdropFilter") as StyleList<Filter, FilterInit>;
    }

    /** Replaces the backdrop filter chain; `undefined` restores inheritance. */
    set backdropFilter(value: FilterInit[] | undefined) {
        setList(this, "backdropFilter", value);
    }

    /** Takes the named properties back to inheriting. An unknown name is a `TypeError`. */
    unset(...names: StyleProperty[]): void {
        const store = storeOf(this);
        const known = this instanceof ClientStyle ? CLIENT_FIELDS : STYLE_FIELDS;
        for (const name of names) {
            if (!known.includes(name)) {
                throw new TypeError(
                    `${pathOf(store, "unset")}: no property "${String(name)}" (known: ${known.join(", ")})`,
                );
            }
        }
        for (const name of names) {
            unsetOne(store, name);
        }
    }

    /** Takes every property back to inheriting. */
    reset(): void {
        this.unset(
            ...(this instanceof ClientStyle ? CLIENT_FIELDS : STYLE_FIELDS).map((name) => name as StyleProperty),
        );
    }

    /** What this scope sets, as plain data: a property it inherits is absent. */
    toJSON(): Record<string, unknown> {
        const store = storeOf(this);
        const out: Record<string, unknown> = {};
        for (const [name, value] of store.scalars) {
            out[name] = value;
        }
        for (const [name, list] of store.lists) {
            out[name] = list.map((element) => element.toJSON());
        }
        return out;
    }
}

/** A `ClientStyle` as a literal: any property may be left out. */
export interface ClientStyleInit {
    /** Opacity from 0 (invisible) to 1. */
    opacity?: number;
    /** One logical-pixel radius for all corners, or one per corner. */
    cornerRadius?: number | CornerRadii;
    /** Background colour, or `null` for no background. */
    background?: Color | null;
    /** Replaces the borders list. */
    borders?: BorderLike[];
    /** Replaces the shadows list. */
    shadows?: ShadowLike[];
    /** Filters applied to the window's own content. */
    filter?: FilterInit[];
    /** Filters applied to the area behind the window. */
    backdropFilter?: FilterInit[];
}

/** The style of a client, or the defaults a scope of clients gives. */
export class ClientStyle extends Style {
    constructor(init?: ClientStyleInit) {
        super(init);
    }

    /** A number, or one radius per corner. Not drawn by any renderer yet. */
    get cornerRadius(): number | Readonly<CornerRadii> | undefined {
        return storeOf(this).scalars.get("cornerRadius") as number | Readonly<CornerRadii> | undefined;
    }

    /** Sets the corner radii; `undefined` restores inheritance. */
    set cornerRadius(value: number | CornerRadii | undefined) {
        setScalar(this, "cornerRadius", value);
    }

    /** A colour or `null` (no background). Not drawn by any renderer yet. */
    get background(): Color | null | undefined {
        return storeOf(this).scalars.get("background") as Color | null | undefined;
    }

    /** Sets the background colour, or clears it with `null`; `undefined` restores inheritance. */
    set background(value: Color | null | undefined) {
        setScalar(this, "background", value);
    }

    /** Not drawn by any renderer yet. */
    get borders(): StyleList<Border, BorderLike> {
        return getList(this, "borders") as StyleList<Border, BorderLike>;
    }

    /** Replaces the border list; `undefined` restores inheritance. */
    set borders(value: BorderLike[] | undefined) {
        setList(this, "borders", value);
    }

    /** Behind the window. The first entry is on top. */
    get shadows(): StyleList<Shadow, ShadowLike> {
        return getList(this, "shadows") as StyleList<Shadow, ShadowLike>;
    }

    /** Replaces the shadow list; `undefined` restores inheritance. */
    set shadows(value: ShadowLike[] | undefined) {
        setList(this, "shadows", value);
    }
}

/** Ties a style to its scope for error paths and inherited lists. */
export function bindStyle(style: Style, scope: StyleScope): void {
    storeOf(style).scope = scope;
}

/** Validates a style literal under `label` and returns the resulting temporary style. */
export function checkStyleInit(init: unknown, label: string): ClientStyle {
    const style = new ClientStyle();
    storeOf(style).label = label;
    assignFields(style, init, CLIENT_FIELDS, label);
    return style;
}

/** Replaces `target` with the values set by `source`; invalid literals leave `target` unchanged. */
export function replaceStyle(target: ClientStyle, source: unknown): void {
    const staged = source instanceof Style ? source : checkStyleInit(source, storeOf(target).scope?.path() ?? "style");
    const from = storeOf(staged);
    target.reset();
    for (const [name, value] of from.scalars) {
        setScalar(target, name, value);
    }
    for (const [name, list] of from.lists) {
        setList(target, name, list);
    }
}
