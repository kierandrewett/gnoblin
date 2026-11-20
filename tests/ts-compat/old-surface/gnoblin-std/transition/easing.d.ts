import { EasingKind } from "gnoblin-std/generated/constants";
/**
 * How an animation moves from its start to its end. Each name uses the CSS timing function with the same name; for
 * example, `ease-out` is `cubic-bezier(0, 0, 0.58, 1)`.
 */
export type Easing = "linear" | "ease" | "ease-in" | "ease-out" | "ease-in-out";
/** The names of the easings, for an error message. */
export declare const EASING_NAMES: string;
/** The easing an animation has when none is given. */
export declare const DEFAULT_EASING: Easing;
/** The library's number of an easing name, or undefined for a name that is not one of `Easing`. */
export declare function easingKind(name: unknown): EasingKind | undefined;
