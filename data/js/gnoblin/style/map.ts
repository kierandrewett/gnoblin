// What a resolved style becomes on the node: the effects the compositor's tree keeps (shadows, borders, a backdrop
// blur), the corner radius and the opacity.

import type { ComputedStyle, FilterValue } from "gnoblin/style/resolve";
import { MAX_GRADE } from "gnoblin/style/values";
import type { Color } from "gnoblin/style/values";

/** One `effect` of the tree's node `effects` list, with the schema's property names (effect.toml). */
export interface EffectWire {
    kind: "blur" | "shadow" | "border";
    downscale: number;
    passes: number;
    algorithm: string;
    noise: number;
    brightness: number;
    contrast: number;
    saturation: number;
    tint: string;
    offset_x: number;
    offset_y: number;
    blur_radius: number;
    spread: number;
    colour: string;
    width: number;
    offset: number;
    inset: boolean;
    frame: boolean;
}

/** The compositor's fixed backdrop blur radius (crates/gnoblin-compositor/src/ipc/tree_effects.rs, `BLUR_RADIUS`). */
export const BACKDROP_BLUR_RADIUS = 12;

/** The wire's documented defaults for the parameters a kind does not use. */
const BLANK: Omit<EffectWire, "kind"> = {
    downscale: 2,
    passes: 3,
    algorithm: "kawase",
    noise: 0,
    brightness: 1,
    contrast: 1,
    saturation: 1,
    tint: "",
    offset_x: 0,
    offset_y: 0,
    blur_radius: 12,
    spread: 0,
    colour: "#00000080",
    width: 0,
    offset: 0,
    inset: false,
    frame: false,
};

/** A colour as the wire spells it: `#rrggbbaa`, lower case, with a full alpha added to a six-digit colour. */
function wireColour(text: Color): string {
    let digits = text.slice(1).toLowerCase();
    if (digits.length <= 4) {
        digits = [...digits].map((digit) => digit + digit).join("");
    }
    return `#${digits.length === 6 ? `${digits}ff` : digits}`;
}

/** What a resolved style becomes on the node, and what of it nothing draws. */
export interface StyleMapping {
    /**
     * In paint order, first to last: the backdrop blur, the shadows from the last listed to the first, then the borders
     * in list order.
     */
    effects: EffectWire[];
    /** The node's `corner_radius`, in logical pixels: the one radius the style asks for, or 0. */
    cornerRadius: number;
    /** The properties of this style the tree cannot express, one text each, `<property>: <reason>`. */
    unapplied: string[];
}

/** The radius a style's `cornerRadius` makes on the node, and the reason when the node cannot say it exactly. */
function nodeRadius(radius: ComputedStyle["cornerRadius"]): { radius: number; note: string | null } {
    if (typeof radius === "number") {
        return { radius, note: null };
    }
    const corners = [radius.topLeft, radius.topRight, radius.bottomRight, radius.bottomLeft];
    const largest = Math.max(...corners);
    return {
        radius: largest,
        note: corners.every((corner) => corner === largest)
            ? null
            : `cornerRadius: a window has one radius for every corner, so the largest of the four (${largest}) is drawn`,
    };
}

/**
 * What a style becomes on the node: a node `corner_radius`, and the `effects` list. Outer `shadows` are `shadow`
 * effects that follow the window's whole frame (the titlebar included), `borders` are `border` effects, and the
 * `backdropFilter` is one backdrop blur whose saturation, brightness and contrast are the products of the chain's.
 * `background`, `filter` and inset shadows stay in `computedStyle`; each comes back as an `unapplied` text when the
 * style has a non-default value for it. Whether the active renderer draws a kind is the compositor's to say: what it
 * leaves out is in the node's `skipped` list (`Client#styleWarnings`), not here.
 */
export function mapStyle(computed: ComputedStyle): StyleMapping {
    const effects: EffectWire[] = [];
    const unapplied: string[] = [];
    const blurs = computed.backdropFilter.filter((f): f is Extract<FilterValue, { kind: "blur" }> => f.kind === "blur");
    const grade = (kind: FilterValue["kind"]): number =>
        computed.backdropFilter.reduce((acc, f) => (f.kind === kind && "amount" in f ? acc * f.amount : acc), 1);
    const [saturation, brightness, contrast] = [grade("saturate"), grade("brightness"), grade("contrast")];
    if (blurs.some((f) => f.radius > 0)) {
        if (blurs.some((f) => f.radius > 0 && f.radius !== BACKDROP_BLUR_RADIUS)) {
            unapplied.push(
                `backdropFilter: the compositor blurs at a fixed radius of ${BACKDROP_BLUR_RADIUS}, so a BlurFilter's radius only switches the blur on`,
            );
        }
        const clamped = (n: number): number => Math.min(MAX_GRADE, n);
        if ([saturation, brightness, contrast].some((n) => n > MAX_GRADE)) {
            unapplied.push(
                `backdropFilter: a combined saturate, brightness or contrast above ${MAX_GRADE} is drawn as ${MAX_GRADE}`,
            );
        }
        effects.push({
            ...BLANK,
            kind: "blur",
            saturation: clamped(saturation),
            brightness: clamped(brightness),
            contrast: clamped(contrast),
        });
    } else if ([saturation, brightness, contrast].some((n) => n !== 1)) {
        unapplied.push(
            "backdropFilter: saturate, brightness and contrast are drawn with the backdrop blur, and there is none in the chain",
        );
    }
    for (const shadow of [...computed.shadows].reverse()) {
        if (shadow.inset) {
            continue;
        }
        effects.push({
            ...BLANK,
            kind: "shadow",
            offset_x: shadow.x,
            offset_y: shadow.y,
            blur_radius: shadow.blur,
            spread: shadow.spread,
            colour: wireColour(shadow.color),
            // A window's shadow is the whole frame's, titlebar and all, not the content's alone.
            frame: true,
        });
    }
    if (computed.shadows.some((shadow) => shadow.inset)) {
        unapplied.push("shadows: an inset shadow has no effect kind");
    }
    for (const border of computed.borders) {
        effects.push({
            ...BLANK,
            kind: "border",
            width: border.width,
            offset: border.offset,
            inset: border.inset,
            colour: wireColour(border.color),
        });
    }
    const { radius, note } = nodeRadius(computed.cornerRadius);
    if (note !== null) {
        unapplied.push(note);
    }
    if (computed.background !== null) {
        unapplied.push("background: the tree has no background for a window");
    }
    if (computed.filter.length > 0) {
        unapplied.push("filter: no effect kind filters a window's own content");
    }
    return { effects, cornerRadius: radius, unapplied };
}
