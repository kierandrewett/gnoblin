// The values a style holds, their limits, and the checkers that accept one or throw with the property path.

/** A colour as `#rgb`, `#rgba`, `#rrggbb` or `#rrggbbaa`. */
export type Color = string;

/** One radius per corner, in pixels. */
export interface CornerRadii {
    topLeft: number;
    topRight: number;
    bottomRight: number;
    bottomLeft: number;
}

/** How many entries a style list holds, and how many effects a window's node carries. */
export const MAX_STYLE_LIST = 8;

/** The renderer's own limits (crates/gnoblin-render/src/effect.rs): a blur or shadow radius, a spread, a grade. */
const MAX_RADIUS = 100;

/** The largest `amount` a saturate, brightness or contrast filter takes. */
export const MAX_GRADE = 2;

/** The text form of a colour: 3, 4, 6 or 8 hex digits after the `#`. */
const COLOUR = /^#(?:[0-9a-f]{3,4}|[0-9a-f]{6}|[0-9a-f]{8})$/i;

/** Throws a RangeError whose text starts with the property path, such as `Client#42.style.opacity`. */
export function fail(path: string, message: string): never {
    throw new RangeError(`${path}: ${message}`);
}

/** Checks `value` for the property at `path`, and returns it (or its frozen, normalised form). */
export type Checker<T = unknown> = (value: unknown, path: string) => T;

/** A checker for a finite number from `min` to `max`, both included. */
function finiteIn(min: number, max: number): Checker<number> {
    return (value, path) => {
        if (typeof value !== "number" || !Number.isFinite(value) || value < min || value > max) {
            if (min === -Infinity && max === Infinity) {
                return fail(path, "must be a finite number");
            }
            return fail(
                path,
                max === Infinity ? `must be finite and >= ${min}` : `must be finite and between ${min} and ${max}`,
            );
        }
        return value;
    };
}

/** Any finite number. */
export const FINITE = finiteIn(-Infinity, Infinity);

/** A finite number that is not below zero. */
export const NON_NEGATIVE = finiteIn(0, Infinity);

/** A finite radius from zero to the largest one. */
export const RADIUS = finiteIn(0, MAX_RADIUS);

/** A finite spread, within the largest radius either way. */
export const SPREAD = finiteIn(-MAX_RADIUS, MAX_RADIUS);

/** A finite filter amount from zero to the largest. */
export const GRADE = finiteIn(0, MAX_GRADE);

/** A finite number from zero to one. */
export const UNIT = finiteIn(0, 1);

/** A colour: a CSS hex string. */
export const colour: Checker<Color> = (value, path) => {
    if (typeof value !== "string" || !COLOUR.test(value)) {
        return fail(path, "must be a #rgb, #rgba, #rrggbb or #rrggbbaa colour");
    }
    return value;
};

/** A boolean. */
export const flag: Checker<boolean> = (value, path) => {
    if (typeof value !== "boolean") {
        return fail(path, "must be true or false");
    }
    return value;
};

const RADII_KEYS: readonly (keyof CornerRadii)[] = ["topLeft", "topRight", "bottomRight", "bottomLeft"];

/** A corner radius: one number for every corner, or a value per corner. */
export const cornerRadius: Checker<number | Readonly<CornerRadii>> = (value, path) => {
    if (typeof value === "number") {
        return NON_NEGATIVE(value, path);
    }
    if (value !== null && typeof value === "object" && !Array.isArray(value)) {
        const given = value as Record<string, unknown>;
        for (const key of Object.keys(given)) {
            if (!(RADII_KEYS as readonly string[]).includes(key)) {
                throw new TypeError(`${path}: no corner "${key}" (known: ${RADII_KEYS.join(", ")})`);
            }
        }
        const radii = {} as CornerRadii;
        for (const key of RADII_KEYS) {
            radii[key] = NON_NEGATIVE(given[key], `${path}.${key}`);
        }
        return Object.freeze(radii);
    }
    return fail(path, "must be a finite number >= 0 or { topLeft, topRight, bottomRight, bottomLeft }");
};

/** A background: a colour, or `null` for none. */
export const background: Checker<Color | null> = (value, path) => (value === null ? null : colour(value, path));
