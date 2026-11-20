import type { Border, BorderLike, Shadow, ShadowLike } from "gnoblin-std/style/border_shadow";
import type { Filter, FilterInit } from "gnoblin-std/style/filters";
import type { StyleList, StyleProperty, StyleScope } from "gnoblin-std/style/lists";
import type { Color, CornerRadii } from "gnoblin-std/style/values";
/**
 * A style holds only values set on its scope; unset properties inherit from lower scopes. `undefined` unsets a
 * property, while `null` is a value for `background`.
 */
export declare class Style {
    constructor(init?: object);
    /** From 0 (invisible) to 1. Written to the window node's `opacity`. */
    get opacity(): number | undefined;
    /** Sets opacity; `undefined` restores inheritance. */
    set opacity(value: number | undefined);
    /** The chain applied to the window's own content. No renderer draws it yet. */
    get filter(): StyleList<Filter, FilterInit>;
    /** Replaces the content filter chain; `undefined` restores inheritance. */
    set filter(value: FilterInit[] | undefined);
    /** The chain applied to what is behind the window. A blur in it is the one the compositor draws. */
    get backdropFilter(): StyleList<Filter, FilterInit>;
    /** Replaces the backdrop filter chain; `undefined` restores inheritance. */
    set backdropFilter(value: FilterInit[] | undefined);
    /** Takes the named properties back to inheriting. An unknown name is a `TypeError`. */
    unset(...names: StyleProperty[]): void;
    /** Takes every property back to inheriting. */
    reset(): void;
    /** What this scope sets, as plain data: a property it inherits is absent. */
    toJSON(): Record<string, unknown>;
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
export declare class ClientStyle extends Style {
    constructor(init?: ClientStyleInit);
    /** A number, or one radius per corner. Not drawn by any renderer yet. */
    get cornerRadius(): number | Readonly<CornerRadii> | undefined;
    /** Sets the corner radii; `undefined` restores inheritance. */
    set cornerRadius(value: number | CornerRadii | undefined);
    /** A colour or `null` (no background). Not drawn by any renderer yet. */
    get background(): Color | null | undefined;
    /** Sets the background colour, or clears it with `null`; `undefined` restores inheritance. */
    set background(value: Color | null | undefined);
    /** Not drawn by any renderer yet. */
    get borders(): StyleList<Border, BorderLike>;
    /** Replaces the border list; `undefined` restores inheritance. */
    set borders(value: BorderLike[] | undefined);
    /** Behind the window. The first entry is on top. */
    get shadows(): StyleList<Shadow, ShadowLike>;
    /** Replaces the shadow list; `undefined` restores inheritance. */
    set shadows(value: ShadowLike[] | undefined);
}
/** Ties a style to its scope for error paths and inherited lists. */
export declare function bindStyle(style: Style, scope: StyleScope): void;
/** Validates a style literal under `label` and returns the resulting temporary style. */
export declare function checkStyleInit(init: unknown, label: string): ClientStyle;
/** Replaces `target` with the values set by `source`; invalid literals leave `target` unchanged. */
export declare function replaceStyle(target: ClientStyle, source: unknown): void;
