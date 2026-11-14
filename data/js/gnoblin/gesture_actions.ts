// Touchpad gestures that run actions, ported from data/retired shell/gnoblin/shell/gestures.retired shell (`Gestures.actions`,
// `Gestures.direction`, `Gestures.on_swipe`, `Gestures.on_pinch`). Each case has its test in
// crates/gnoblin-js/tests/input.rs (the last seven cases of tests/retired shell/retired corpus gestures).
//
//   defineConfig({
//       gestureActions: {
//           Swipe4: { up: "window.toggle-maximise", left: "workspace.next", right: "workspace.previous" },
//           Pinch3: { in: "window.minimise", out: (event) => console.log(event.scale) },
//       },
//   });
//
// A gesture is bound like a chord (`Swipe3`, `Pinch4`: the gesture and its finger count) and Gnoblin claims it, so no
// client sees it. An action runs once, when the fingers lift, for the direction the gesture went; a cancelled gesture
// runs nothing. The five named actions are `gnoblin.gestures.actions`, and a config can give a function instead of a
// name. The stock shell binds one gesture, the three-finger workspace slide: a gesture bound at its trigger replaces it,
// so use four fingers to keep both.

import { GestureBinding } from "gnoblin/bindings";
import type { DragEvent } from "gnoblin/bindings";

/** The shortest swipe `onSwipe` acts on, in touchpad units. */
export const SWIPE_THRESHOLD = 48;
/** How far from 1 a pinch's scale must be for `onPinch` to act on it. */
export const PINCH_THRESHOLD = 0.12;

export type SwipeDirection = "left" | "right" | "up" | "down";
export type PinchDirection = "in" | "out";

/** The five actions the configuration names. `window.minimize` is accepted for `window.minimise`. */
export type GestureActionName =
    | "workspace.next"
    | "workspace.previous"
    | "window.close"
    | "window.minimise"
    | "window.minimize"
    | "window.toggle-maximise";

/** The five stock actions. Each takes nothing: it acts on the workspace shown under the pointer or on the focused window. */
export interface GestureActions {
    /** Shows the workspace after the shown one; nothing at the last. */
    workspaceNext(): void;
    workspacePrevious(): void;
    /** Asks the focused window to close; nothing without one. */
    windowClose(): void;
    /** Minimises the focused window, which hands the keyboard to the next one. */
    windowMinimise(): void;
    /** Maximises the focused window, or restores it when it is maximised. */
    windowToggleMaximise(): void;
}

/** What a gesture runs: the name of a stock action, or a function that gets the gesture's last event. */
export type GestureAction = GestureActionName | ((event: DragEvent) => void);

/** The map `onSwipe` takes. */
export type SwipeMap = { [way in SwipeDirection]?: GestureAction };
/** The map `onPinch` takes. */
export type PinchMap = { [way in PinchDirection]?: GestureAction };

const ACTION_NAMES: Readonly<Record<GestureActionName, keyof GestureActions>> = {
    "workspace.next": "workspaceNext",
    "workspace.previous": "workspacePrevious",
    "window.close": "windowClose",
    "window.minimise": "windowMinimise",
    "window.minimize": "windowMinimise",
    "window.toggle-maximise": "windowToggleMaximise",
};

/** The names a config may give an action. */
export const GESTURE_ACTION_NAMES: readonly GestureActionName[] = Object.keys(ACTION_NAMES) as GestureActionName[];

const SWIPE_WAYS: readonly string[] = ["left", "right", "up", "down"];
const PINCH_WAYS: readonly string[] = ["in", "out"];

/**
 * The way a gesture has gone: `left`, `right`, `up` or `down` by its longer axis, or `null` while it has moved less than
 * `threshold` (default `SWIPE_THRESHOLD`) touchpad units. A tie goes to the horizontal axis.
 */
export function gestureDirection(
    event: { readonly dx?: number; readonly dy?: number },
    threshold: number = SWIPE_THRESHOLD,
): SwipeDirection | null {
    const dx = event.dx ?? 0;
    const dy = event.dy ?? 0;
    if (Math.max(Math.abs(dx), Math.abs(dy)) < threshold) {
        return null;
    }
    if (Math.abs(dx) >= Math.abs(dy)) {
        return dx > 0 ? "right" : "left";
    }
    return dy > 0 ? "down" : "up";
}

/** Where a named action finds the stock actions when a binding has none of its own: the `gnoblin` global. */
function globalActions(): GestureActions {
    // `globalThis` has no typed `gnoblin`: the host sets it before any config runs.
    const scope = globalThis as { gnoblin?: { gestures?: { actions?: GestureActions } } };
    const actions = scope.gnoblin?.gestures?.actions;
    if (actions === undefined) {
        throw new Error("a named gesture action needs the gnoblin global to run on");
    }
    return actions;
}

/** Checks `map`'s keys against `ways` and its values against what an action may be; returns the actions by way. */
function checkMap(map: unknown, ways: readonly string[], what: string): Map<string, GestureAction> {
    if (map === null || typeof map !== "object" || Array.isArray(map)) {
        throw new TypeError(`${what}: needs a table of ${ways.join(", ")} to an action`);
    }
    const checked = new Map<string, GestureAction>();
    for (const [way, action] of Object.entries(map)) {
        if (!ways.includes(way)) {
            throw new TypeError(`${what}: "${way}" is not a way for this gesture to go; use ${ways.join(", ")}`);
        }
        if (typeof action === "function") {
            checked.set(way, action as GestureAction);
        } else if (typeof action === "string" && Object.hasOwn(ACTION_NAMES, action)) {
            checked.set(way, action as GestureActionName);
        } else {
            throw new TypeError(
                `${what}.${way}: an action is a function or one of ${GESTURE_ACTION_NAMES.join(", ")}, got ${String(action)}`,
            );
        }
    }
    return checked;
}

export interface GestureActionOptions {
    /** The binding's id. Default a generated one. */
    id?: string;
    /** The trigger the binding is made for, `Swipe3` to `Pinch5`. `InputContext.bind` sets it when it is not this. */
    trigger?: string;
    /**
     * The least movement that counts: touchpad units for a swipe (default 48), the distance of the scale from 1 for a
     * pinch (default 0.12).
     */
    threshold?: number;
    /** What a named action runs. Default the `gnoblin` global's `gestures.actions`. */
    actions?: GestureActions;
}

let serial = 0;

function nextId(): string {
    serial += 1;
    return `gesture-actions-${serial}`;
}

abstract class GestureActionBinding extends GestureBinding {
    readonly threshold: number;
    readonly #map: Map<string, GestureAction>;
    readonly #actions: GestureActions | undefined;

    constructor(map: Map<string, GestureAction>, trigger: string, fallback: number, options: GestureActionOptions) {
        super({ id: options.id ?? nextId(), trigger: options.trigger ?? trigger });
        const threshold = options.threshold ?? fallback;
        if (typeof threshold !== "number" || !Number.isFinite(threshold) || threshold < 0) {
            throw new RangeError(`a gesture threshold is a finite number of at least 0, got ${String(threshold)}`);
        }
        this.threshold = threshold;
        this.#map = map;
        this.#actions = options.actions;
    }

    /** Which way the gesture went, or `null` when it did not go far enough to count. */
    protected abstract way(event: DragEvent): string | null;

    onStart(): object {
        return {};
    }

    override onEnd(event: DragEvent): void {
        const way = this.way(event);
        const action = way === null ? undefined : this.#map.get(way);
        if (action === undefined) {
            return;
        }
        if (typeof action === "function") {
            action(event);
        } else {
            (this.#actions ?? globalActions())[ACTION_NAMES[action]]();
        }
    }
}

/** What `onSwipe` makes: a binding that acts when a swipe ends, for the direction it went. */
export class SwipeActionBinding extends GestureActionBinding {
    constructor(map: SwipeMap, options: GestureActionOptions = {}) {
        super(checkMap(map, SWIPE_WAYS, "onSwipe"), "Swipe3", SWIPE_THRESHOLD, options);
    }

    protected way(event: DragEvent): string | null {
        return gestureDirection(event, this.threshold);
    }
}

/** What `onPinch` makes: a binding that acts when a pinch ends, `out` for a spread and `in` for a gather. */
export class PinchActionBinding extends GestureActionBinding {
    constructor(map: PinchMap, options: GestureActionOptions = {}) {
        super(checkMap(map, PINCH_WAYS, "onPinch"), "Pinch2", PINCH_THRESHOLD, options);
    }

    protected way(event: DragEvent): string | null {
        const scale = event.scale ?? 1;
        if (scale >= 1 + this.threshold) {
            return "out";
        }
        return scale <= 1 - this.threshold ? "in" : null;
    }
}

/**
 * A binding for `InputContext.bind` that runs `map[direction]` when a swipe ends, if the swipe went that way (see
 * `gestureDirection`). `threshold` is the shortest swipe, default 48 units. A cancelled swipe runs nothing.
 *
 *     gnoblin.input.primary.bind("Swipe4", onSwipe({ up: "window.toggle-maximise" }));
 */
export function onSwipe(map: SwipeMap, threshold?: number, options: GestureActionOptions = {}): SwipeActionBinding {
    return new SwipeActionBinding(map, threshold === undefined ? options : { ...options, threshold });
}

/**
 * A binding that runs `map.out` or `map.in` when a pinch ends: `out` when the contacts spread to at least 1 + `threshold`
 * of their start (default 0.12), `in` when they gathered to at most 1 - threshold.
 */
export function onPinch(map: PinchMap, threshold?: number, options: GestureActionOptions = {}): PinchActionBinding {
    return new PinchActionBinding(map, threshold === undefined ? options : { ...options, threshold });
}

/** The gesture helpers of the `gnoblin` global: `gnoblin.gestures`. */
export interface GestureTools {
    /** The five stock actions. */
    readonly actions: GestureActions;
    direction: typeof gestureDirection;
    onSwipe(map: SwipeMap, threshold?: number): SwipeActionBinding;
    onPinch(map: PinchMap, threshold?: number): PinchActionBinding;
}

/** What `defineConfig({ gestureActions })` takes for one gesture: a swipe's four directions or a pinch's two. */
export type GestureActionConfig = SwipeMap & PinchMap;

/**
 * The binding a `gestureActions` entry stands for: a swipe for `Swipe<n>`, a pinch for `Pinch<n>`. The trigger names
 * the gesture and its finger count and no direction: the map holds the directions.
 */
export function gestureActionBinding(
    trigger: string,
    map: GestureActionConfig,
    gesture: "swipe" | "pinch",
): GestureBinding {
    return gesture === "swipe" ? new SwipeActionBinding(map, { trigger }) : new PinchActionBinding(map, { trigger });
}
