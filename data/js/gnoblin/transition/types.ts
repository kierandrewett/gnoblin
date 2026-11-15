// The shapes of `gnoblin.requestTransition`: what a script passes and gets back, and the small host (`TransitionHost`)
// the engine runs on, so that it works without a compositor.

import type { Easing } from "gnoblin/transition/easing";
import type { MakeError, Staging } from "gnoblin/transition/staging";

/** The milliseconds a transition takes when `duration` is not given. */
export const DEFAULT_DURATION_MS = 250;

/** The options a transition takes. Every field is optional; the default of each is stated on the field. */
export interface TransitionOptions {
    /** Milliseconds, 0 or more. Default 250. Zero commits the change at once (still as one transaction). */
    duration?: number;
    /** One of the CSS names of `Easing`. Default `"ease-out"`, as `Client.animate`'s. */
    easing?: Easing;
    /** Milliseconds, 0 or more, the compositor holds the start state before the animation begins. Default 0. */
    delay?: number;
    /**
     * Default true. False stages the transition with its tracks held at the start: `progress` scrubs them, `play()`
     * and `reverse()` run them, `pause()` holds them.
     */
    autoplay?: boolean;
}

/**
 * What `finished` and `onFinished` deliver: `cancelled` is true when `cancel()` or a newer transition took every
 * track.
 */
export interface TransitionResult {
    readonly cancelled: boolean;
}

/** Stops a callback before it runs. Calling it twice is harmless. */
export interface TransitionSubscription {
    unsubscribe(): void;
}

/** What an engine needs of the shell. */
export interface TransitionHost {
    staging: Staging;
    /** The milliseconds an animation of `duration` takes: 0 while the seat asks for reduced motion. */
    motion(duration: number): number;
    /** Whether the tree takes a timed write of `key`. */
    animated(key: string): boolean;
    /** Whether a node has a place that is drawn: a size, and not hidden. A node that has none just appears. */
    placed(node: number): boolean;
    /** The name of a node in an error path: `Client#3`. */
    label(node: number): string;
    /**
     * Makes the compositor's `tween` node, the play state its tracks join (`progress`, `paused`, `reverse`), and
     * returns its id. Throws when the shell can make no more nodes.
     */
    makeTween(): number;
    /** Destroys a `tween` node made by `makeTween`. */
    dropTween(tween: number): void;
    /** Queues `task` after the calls already queued, under the listener budget. `site` names it in a report. */
    schedule(site: string, task: () => void): void;
    /** Takes what a callback can change that is not a tree write, and returns what puts it back. */
    snapshot(): () => void;
    /** Runs the style and layout passes once against the staged state, so their writes are staged too. */
    settle(): void;
    error: MakeError;
}

/** The compositor's word that a timed write ended (`track_ended`), as the host hands it over. */
export interface TrackEndedEvent {
    /** The node the write was to. */
    node: number;
    /** The property's tree key: `x`, `opacity`, `rotation`. */
    property: string;
    /** True when the track reached its target; false for every other end. */
    completed: boolean;
    /** The `tween` node the write joined; 0 for none. */
    tween: number;
}
