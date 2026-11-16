// The handle returned by `gnoblin.requestTransition`: controls its tracks and reports how they ended.

import type { TransitionHost, TransitionResult, TransitionSubscription } from "gnoblin-std/transition/types";

let counter = 0;

/**
 * The controls the transition engine uses to operate the compositor tracks.
 */
export interface TransitionControl {
    /** Sets the tracks to a fraction of their duration, from 0 to 1. */
    seek(progress: number): void;
    /** Holds or resumes the tracks. */
    pause(paused: boolean): void;
    /** Sets whether the tracks run towards the start or target. */
    reverse(reverse: boolean): void;
    /** Ends every owned track at its target value. */
    cancel(): void;
}

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
export class Transition {
    /** A unique identifier for this transition. */
    readonly id: number;
    /** The duration in milliseconds, or 0 when reduced motion is active. */
    readonly duration: number;
    #host: TransitionHost;
    #state: "running" | "finished" | "cancelled" = "running";
    #callbacks: { run: (result: TransitionResult) => void; live: boolean }[] = [];
    #promise: Promise<TransitionResult> | null = null;
    #resolve: ((result: TransitionResult) => void) | null = null;
    #control: TransitionControl | null;
    #progress = 0;
    #paused: boolean;
    #reversed = false;

    /** Only the engine makes transitions: a script receives one from `requestTransition`. */
    constructor(host: TransitionHost, duration: number, autoplay: boolean, control: TransitionControl | null) {
        this.id = counter += 1;
        this.duration = duration;
        this.#host = host;
        this.#paused = !autoplay;
        this.#control = control;
    }

    /** True from the call until the transition finishes or is cancelled. */
    get running(): boolean {
        return this.#state === "running";
    }

    /** True once `cancel()` or a newer transition took every track, or it ran back to its start. */
    get cancelled(): boolean {
        return this.#state === "cancelled";
    }

    /** Whether the tracks are held: `autoplay: false` until `play()` or `reverse()`, and after `pause()`. */
    get paused(): boolean {
        return this.#paused;
    }

    /** Whether the tracks run back to their start. */
    get reversed(): boolean {
        return this.#reversed;
    }

    /**
     * Where the tracks were last put, from 0 (the start) to 1 (the target), before the easing. Writing it moves every
     * track to that point of its duration at once and draws it; it does not play or pause. Reading it gives what was
     * last written, 0 before that: the compositor does not report a running transition's place.
     */
    get progress(): number {
        return this.#progress;
    }

    /** Sets the track position from 0 (start) to 1 (target), without playing or pausing. */
    set progress(value: number) {
        if (typeof value !== "number" || !Number.isFinite(value) || value < 0 || value > 1) {
            throw new RangeError(`Transition.progress: expects a number from 0 to 1, not ${String(value)}`);
        }
        this.#host.staging.forbid("Transition.progress");
        if (this.#state !== "running") {
            return;
        }
        this.#progress = value;
        this.#control?.seek(value);
    }

    /** Lets the tracks run on towards the target, from where they are. */
    play(): void {
        this.#host.staging.forbid("Transition.play()");
        if (this.#state !== "running") {
            return;
        }
        this.#paused = false;
        this.#reversed = false;
        this.#control?.reverse(false);
        this.#control?.pause(false);
    }

    /** Holds the tracks where they are. */
    pause(): void {
        this.#host.staging.forbid("Transition.pause()");
        if (this.#state !== "running") {
            return;
        }
        this.#paused = true;
        this.#control?.pause(true);
    }

    /**
     * Runs the tracks back towards their start from where they are. When they get there the transition settles
     * `{ cancelled: true }` and the start values are back in the tree.
     */
    reverse(): void {
        this.#host.staging.forbid("Transition.reverse()");
        if (this.#state !== "running") {
            return;
        }
        this.#paused = false;
        this.#reversed = true;
        this.#control?.reverse(true);
        this.#control?.pause(false);
    }

    /**
     * A promise for the end. It is made when first asked for, so a script that never reads it leaves no unhandled
     * promise behind. It resolves once, with `{ cancelled }`, and never rejects.
     */
    get finished(): Promise<TransitionResult> {
        const existing = this.#promise;
        if (existing !== null) {
            return existing;
        }
        // ES2023 has no `withResolvers`; the Promise executor synchronously captures its resolver here.
        let resolve!: (value: TransitionResult | PromiseLike<TransitionResult>) => void;
        const promise = new Promise<TransitionResult>((settle) => {
            resolve = settle;
        });
        this.#promise = promise;
        this.#resolve = resolve;
        if (this.#state !== "running") {
            resolve({ cancelled: this.#state === "cancelled" });
        }
        return promise;
    }

    /** Calls `fn` once when the transition ends; unsubscribe to skip a callback that has not run yet. */
    onFinished(fn: (result: TransitionResult) => void): TransitionSubscription {
        if (typeof fn !== "function") {
            throw new TypeError("Transition.onFinished: expects a function");
        }
        const entry = { run: fn, live: true };
        if (this.#state === "running") {
            this.#callbacks.push(entry);
        } else {
            this.#deliver(entry);
        }
        return {
            unsubscribe: () => {
                entry.live = false;
            },
        };
    }

    /**
     * Stops the transition: every property it still owns is written at its end value at once, which ends the
     * compositor's track, and `finished` settles with `{ cancelled: true }`. The target state was committed when the
     * transition began, so cancelling puts what is drawn there and does not go back. Nothing happens once it ended.
     */
    cancel(): void {
        if (this.#state === "running") {
            this.#control?.cancel();
            this.settle(true);
        }
    }

    /** Settles the transition. Engine only: a second settle does nothing. */
    settle(cancelled: boolean): void {
        if (this.#state !== "running") {
            return;
        }
        this.#state = cancelled ? "cancelled" : "finished";
        const result: TransitionResult = Object.freeze({ cancelled });
        this.#resolve?.(result);
        for (const entry of this.#callbacks.splice(0)) {
            this.#deliver(entry, result);
        }
    }

    #deliver(entry: { run: (result: TransitionResult) => void; live: boolean }, result?: TransitionResult): void {
        const delivered = result ?? { cancelled: this.#state === "cancelled" };
        this.#host.schedule(`Transition#${this.id}.onFinished`, () => {
            if (entry.live) {
                entry.live = false;
                entry.run(delivered);
            }
        });
    }

    /** Returns this transition's diagnostic label. */
    toString(): string {
        return `Transition#${this.id}`;
    }
}
