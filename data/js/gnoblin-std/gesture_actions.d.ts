import { GestureBinding } from "gnoblin-core/bindings/binding";
import type { DragEvent } from "gnoblin-core/bindings/types";
/** The shortest swipe `onSwipe` acts on, in touchpad units. */
export declare const SWIPE_THRESHOLD = 48;
/** How far from 1 a pinch's scale must be for `onPinch` to act on it. */
export declare const PINCH_THRESHOLD = 0.12;
/** A swipe's direction, by the longer axis of the gesture. See `gestureDirection`. */
export type SwipeDirection = "left" | "right" | "up" | "down";
/**
 * A pinch's direction: `in` when the fingers come together, `out` when they spread. The scale must move past the
 * threshold from 1 for either to count.
 */
export type PinchDirection = "in" | "out";
/** The five actions the configuration names. `window.minimize` is accepted for `window.minimise`. */
export type GestureActionName = "workspace.next" | "workspace.previous" | "window.close" | "window.minimise" | "window.minimize" | "window.toggle-maximise";
/**
 * The five stock actions. Each takes nothing and acts on the workspace under the pointer or on the focused window.
 */
export interface GestureActions {
    /** Shows the workspace after the shown one; nothing at the last. */
    workspaceNext(): void;
    /** Shows the workspace before the shown one; nothing at the first. */
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
export type SwipeMap = {
    [way in SwipeDirection]?: GestureAction;
};
/** The map `onPinch` takes. */
export type PinchMap = {
    [way in PinchDirection]?: GestureAction;
};
/** The names a config may give an action. */
export declare const GESTURE_ACTION_NAMES: readonly GestureActionName[];
/**
 * The way a gesture has gone: `left`, `right`, `up` or `down` by its longer axis, or `null` while it has moved less
 * than `threshold` (default `SWIPE_THRESHOLD`) touchpad units. A tie goes to the horizontal axis. The library decides.
 */
export declare function gestureDirection(event: {
    readonly dx?: number;
    readonly dy?: number;
}, threshold?: number): SwipeDirection | null;
/** Options shared by `onSwipe`, `onPinch` and `gestureActionBinding`. Every field is optional. */
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
/** The part the swipe and pinch bindings share: it runs the action for the way the gesture went, when it ends. */
declare abstract class GestureActionBinding extends GestureBinding {
    #private;
    readonly threshold: number;
    constructor(map: Map<string, GestureAction>, trigger: string, fallback: number, options: GestureActionOptions);
    /** Which way the gesture went, or `null` when it did not go far enough to count. */
    protected abstract way(event: DragEvent): string | null;
    /** Nothing is kept while the gesture runs: the way is read once, from the event that ends it. */
    onStart(): object;
    onEnd(event: DragEvent): void;
}
/** What `onSwipe` makes: a binding that acts when a swipe ends, for the direction it went. */
export declare class SwipeActionBinding extends GestureActionBinding {
    constructor(map: SwipeMap, options?: GestureActionOptions);
    protected way(event: DragEvent): string | null;
}
/** What `onPinch` makes: a binding that acts when a pinch ends, `out` for a spread and `in` for a gather. */
export declare class PinchActionBinding extends GestureActionBinding {
    constructor(map: PinchMap, options?: GestureActionOptions);
    protected way(event: DragEvent): string | null;
}
/**
 * A binding for `InputContext.bind` that runs `map[direction]` when a swipe ends, if the swipe went that way (see
 * `gestureDirection`). `threshold` is the shortest swipe, default 48 units. A cancelled swipe runs nothing.
 *
 *     gnoblin.input.primary.bind("Swipe4", onSwipe({ up: "window.toggle-maximise" }));
 */
export declare function onSwipe(map: SwipeMap, threshold?: number, options?: GestureActionOptions): SwipeActionBinding;
/**
 * A binding that runs `map.out` or `map.in` when a pinch ends: `out` when the contacts spread to at least
 * 1 + `threshold` of their start (default 0.12), `in` when they gathered to at most 1 - threshold.
 */
export declare function onPinch(map: PinchMap, threshold?: number, options?: GestureActionOptions): PinchActionBinding;
/** The gesture helpers of the `gnoblin` global: `gnoblin.gestures`. */
export interface GestureTools {
    /** The five stock actions. */
    readonly actions: GestureActions;
    /** Gets the direction of a swipe event. */
    direction: typeof gestureDirection;
    /** Creates a binding that runs an action when a swipe ends. */
    onSwipe(map: SwipeMap, threshold?: number): SwipeActionBinding;
    /** Creates a binding that runs an action when a pinch ends. */
    onPinch(map: PinchMap, threshold?: number): PinchActionBinding;
}
export {};
