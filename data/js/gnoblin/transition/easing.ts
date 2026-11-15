// The named easings of `gnoblin.requestTransition` and the samples of their curves. Each is the CSS timing function
// of that name, so a script that knows CSS knows the curve.

/**
 * How an animation moves from its start to its end: the CSS names the specification uses. Each is the CSS timing
 * function of that name (`ease-out` is `cubic-bezier(0, 0, 0.58, 1)`), so a script that knows CSS knows the curve.
 */
export type Easing = "linear" | "ease" | "ease-in" | "ease-out" | "ease-in-out";

/** The control points `(x1, y1, x2, y2)` of the CSS cubic Bezier each named easing is. */
const BEZIER_POINTS: Readonly<Record<Exclude<Easing, "linear">, readonly [number, number, number, number]>> = {
    ease: [0.25, 0.1, 0.25, 1],
    "ease-in": [0.42, 0, 1, 1],
    "ease-out": [0, 0, 0.58, 1],
    "ease-in-out": [0.42, 0, 0.58, 1],
};

/** The CSS timing function of a Bezier through (0, 0) and (1, 1): progress in time onto progress in value. */
function cubicBezier(x1: number, y1: number, x2: number, y2: number): (t: number) => number {
    // The curve is parametric: x(s) and y(s) are cubics in s. Solving x(s) = t gives s, and y(s) is the value.
    const coordinate = (a: number, b: number, s: number): number =>
        3 * a * (1 - s) ** 2 * s + 3 * b * (1 - s) * s ** 2 + s ** 3;
    const slope = (a: number, b: number, s: number): number =>
        3 * a * (1 - s) ** 2 + 6 * (b - a) * (1 - s) * s + 3 * (1 - b) * s ** 2;
    return (t) => {
        if (t <= 0) {
            return 0;
        }
        if (t >= 1) {
            return 1;
        }
        // Newton's method converges in a few steps on these monotonic curves; bisection backs it up where the
        // slope is nearly flat.
        let s = t;
        for (let i = 0; i < 8; i += 1) {
            const error = coordinate(x1, x2, s) - t;
            if (Math.abs(error) < 1e-7) {
                return coordinate(y1, y2, s);
            }
            const d = slope(x1, x2, s);
            if (Math.abs(d) < 1e-6) {
                break;
            }
            s -= error / d;
        }
        let low = 0;
        let high = 1;
        s = t;
        for (let i = 0; i < 40; i += 1) {
            const x = coordinate(x1, x2, s);
            if (Math.abs(x - t) < 1e-7) {
                break;
            }
            if (x < t) {
                low = s;
            } else {
                high = s;
            }
            s = (low + high) / 2;
        }
        return coordinate(y1, y2, s);
    };
}

/** The curve for each `Easing`: it maps progress from 0 to 1 onto an eased value that also runs from 0 to 1. */
export const EASINGS: Readonly<Record<Easing, (t: number) => number>> = {
    linear: (t) => t,
    ease: cubicBezier(...BEZIER_POINTS.ease),
    "ease-in": cubicBezier(...BEZIER_POINTS["ease-in"]),
    "ease-out": cubicBezier(...BEZIER_POINTS["ease-out"]),
    "ease-in-out": cubicBezier(...BEZIER_POINTS["ease-in-out"]),
};

/** The names of the easings, for an error message. */
export const EASING_NAMES = Object.keys(EASINGS).join(", ");

/** The easing an animation has when none is given. */
export const DEFAULT_EASING: Easing = "ease-out";

/** The curve is sent as this many steps plus the start: 33 samples, as the Lua shell's `node:to` sends. */
export const CURVE_STEPS = 32;

/** The samples of `curve`: progress from 0 to 1 over `CURVE_STEPS` steps, which is what a timed write carries. */
export function curveSamples(curve: (t: number) => number): number[] {
    return Array.from({ length: CURVE_STEPS + 1 }, (_, i) => curve(i / CURVE_STEPS));
}

/** The curve of an easing name, or undefined for a name that is not one of `Easing`. */
export function easingCurve(name: unknown): ((t: number) => number) | undefined {
    return typeof name === "string" && Object.hasOwn(EASINGS, name) ? EASINGS[name as Easing] : undefined;
}
