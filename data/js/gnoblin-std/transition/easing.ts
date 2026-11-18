// The names of the easing curves of `gnoblin.requestTransition`. The curves are the library's (`easing_sample`); a name is
// its number here and nothing else.

import { EasingKind } from "gnoblin-std/generated/constants";

/**
 * How an animation moves from its start to its end. Each name uses the CSS timing function with the same name; for
 * example, `ease-out` is `cubic-bezier(0, 0, 0.58, 1)`.
 */
export type Easing = "linear" | "ease" | "ease-in" | "ease-out" | "ease-in-out";

/** The library's number for each name. */
const KINDS: Readonly<Record<Easing, EasingKind>> = {
    linear: EasingKind.LINEAR,
    ease: EasingKind.EASE,
    "ease-in": EasingKind.EASE_IN,
    "ease-out": EasingKind.EASE_OUT,
    "ease-in-out": EasingKind.EASE_IN_OUT,
};

/** The names of the easings, for an error message. */
export const EASING_NAMES = Object.keys(KINDS).join(", ");

/** The easing an animation has when none is given. */
export const DEFAULT_EASING: Easing = "ease-out";

/** The library's number of an easing name, or undefined for a name that is not one of `Easing`. */
export function easingKind(name: unknown): EasingKind | undefined {
    return typeof name === "string" && Object.hasOwn(KINDS, name) ? KINDS[name as Easing] : undefined;
}
