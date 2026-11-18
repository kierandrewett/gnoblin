// The options and results of `gnoblin.requestTransition`, plus what the transition layer needs of the shell.

import type { Easing } from "gnoblin-std/transition/easing";
import type { Staging } from "gnoblin-std/transition/staging";

/** The milliseconds a transition takes when `duration` is not given. */
export const DEFAULT_DURATION_MS = 250;

/** The options a transition takes. Every field is optional; the default of each is stated on the field. */
export interface TransitionOptions {
    /** Milliseconds, 0 or more. Default 250. Zero commits the change at once (still as one transaction). */
    duration?: number;
    /** One of the CSS names of `Easing`. Default `"ease-out"`. */
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
    /** True when the transition did not reach its target. */
    readonly cancelled: boolean;
}

/** Stops a callback before it runs. Calling it twice is harmless. */
export interface TransitionSubscription {
    /** Stops the callback before it runs. Calling it twice is harmless. */
    unsubscribe(): void;
}

/** What the transition layer needs of the shell. */
export interface TransitionHost {
    /** Stages the engine's reads and writes during a transition callback, in the library (`gnoblin-std/transition/staging`). */
    staging: Staging;
    /** Queues `task` after the calls already queued, under the listener budget. `site` names it in a report. */
    schedule(site: string, task: () => void): void;
    /** Takes what a callback can change that is not a tree write, and returns what puts it back. */
    snapshot(): () => void;
    /** Runs the style and layout passes once against the staged state, so their writes are staged too. */
    settle(): void;
}
