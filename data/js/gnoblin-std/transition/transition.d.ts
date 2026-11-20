import { Transition as Native } from "gnoblin-std/generated/classes";
import type { TransitionHost, TransitionResult, TransitionSubscription } from "gnoblin-std/transition/types";
/**
 * One call of `requestTransition`. `finished` settles once, with `{ cancelled }`; `onFinished` runs a plain callback
 * once as a queued call, one added after the settle runs after the current call returns.
 *
 * The compositor plays the tracks and says when they end (`track_ended`), so a transition ends when its tracks do and
 * not when the shell's clock says so. With `autoplay: false` the tracks start held at the start value: `progress`
 * moves them to a point of their duration, `play()` lets them run on, `reverse()` runs them back, `pause()` holds them.
 * The target state is committed when the transition is requested (the layout, the styles and the tree all hold it),
 * and what is drawn is the compositor's interpolation; a transition that runs back to its start puts the start values
 * back and settles `{ cancelled: true }`.
 */
export declare class Transition {
    #private;
    /** A unique identifier for this transition. */
    readonly id: number;
    /** The duration in milliseconds, or 0 when reduced motion is active. */
    readonly duration: number;
    /** Only the engine makes transitions: a script receives one from `requestTransition`. */
    constructor(host: TransitionHost, native: Native);
    /** True from the call until the transition finishes or is cancelled. */
    get running(): boolean;
    /** True once `cancel()` or a newer transition took every track, or it ran back to its start. */
    get cancelled(): boolean;
    /** Whether the tracks are held: `autoplay: false` until `play()` or `reverse()`, and after `pause()`. */
    get paused(): boolean;
    /** Whether the tracks run back to their start. */
    get reversed(): boolean;
    /**
     * Where the tracks were last put, from 0 (the start) to 1 (the target), before the easing. Writing it moves every
     * track to that point of its duration at once and draws it; it does not play or pause. Reading it gives what was
     * last written, 0 before that: the compositor does not report a running transition's place.
     */
    get progress(): number;
    /** Sets the track position from 0 (start) to 1 (target), without playing or pausing. */
    set progress(value: number);
    /** Lets the tracks run on towards the target, from where they are. */
    play(): void;
    /** Holds the tracks where they are. */
    pause(): void;
    /**
     * Runs the tracks back towards their start from where they are. When they get there the transition settles
     * `{ cancelled: true }` and the start values are back in the tree.
     */
    reverse(): void;
    /**
     * A promise for the end. It is made when first asked for, so a script that never reads it leaves no unhandled
     * promise behind. It resolves once, with `{ cancelled }`, and never rejects.
     */
    get finished(): Promise<TransitionResult>;
    /** Calls `fn` once when the transition ends; unsubscribe to skip a callback that has not run yet. */
    onFinished(fn: (result: TransitionResult) => void): TransitionSubscription;
    /**
     * Stops the transition: every property it still owns is written at its end value at once, which ends the
     * compositor's track, and `finished` settles with `{ cancelled: true }`. The target state was committed when the
     * transition began, so cancelling puts what is drawn there and does not go back. Nothing happens once it ended.
     */
    cancel(): void;
    /** Settles the transition. The library's word and `cancel` only: a second settle does nothing. */
    settle(cancelled: boolean): void;
    /** Returns this transition's diagnostic label. */
    toString(): string;
}
