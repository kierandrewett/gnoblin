/** A colour as `#rgb`, `#rgba`, `#rrggbb` or `#rrggbbaa`. */
export type Color = string;
/** One radius per corner, in pixels. */
export interface CornerRadii {
    /** Radius of the top-left corner in logical pixels. */
    topLeft: number;
    /** Radius of the top-right corner in logical pixels. */
    topRight: number;
    /** Radius of the bottom-right corner in logical pixels. */
    bottomRight: number;
    /** Radius of the bottom-left corner in logical pixels. */
    bottomLeft: number;
}
/** How many entries a style list holds, and how many effects a window's node carries. */
export declare const MAX_STYLE_LIST = 8;
/** The largest `amount` a saturate, brightness or contrast filter takes. */
export declare const MAX_GRADE = 2;
/** The renderer's limits on a shader (crates/gnoblin-render/src/effect.rs): the source in bytes and the numbers it reads. */
export declare const MAX_SHADER_SOURCE_BYTES: number;
export declare const MAX_SHADER_PARAMS = 8;
/** Throws a RangeError whose text starts with the property path, such as `Client#42.style.opacity`. */
export declare function fail(path: string, message: string): never;
/** Checks `value` for the property at `path`, and returns it (or its frozen, normalised form). */
export type Checker<T = unknown> = (value: unknown, path: string) => T;
/** Any finite number. */
export declare const FINITE: Checker<number>;
/** A finite number that is not below zero. */
export declare const NON_NEGATIVE: Checker<number>;
/** A finite radius from zero to the largest one. */
export declare const RADIUS: Checker<number>;
/** A finite spread, within the largest radius either way. */
export declare const SPREAD: Checker<number>;
/** A finite filter amount from zero to the largest. */
export declare const GRADE: Checker<number>;
/** A finite number from zero to one. */
export declare const UNIT: Checker<number>;
/** A colour: a CSS hex string. */
export declare const colour: Checker<Color>;
/** A shader's WGSL source: text that is not blank, of at most 16 KiB. Whether it is valid WGSL is the compositor's to say. */
export declare const shaderSource: Checker<string>;
/** The numbers a shader reads as `p`: at most 8, each finite. Returns a copy. */
export declare const shaderParams: Checker<number[]>;
/** A boolean. */
export declare const flag: Checker<boolean>;
/** A corner radius: one number for every corner, or a value per corner. */
export declare const cornerRadius: Checker<number | Readonly<CornerRadii>>;
/** A background: a colour, or `null` for none. */
export declare const background: Checker<Color | null>;
