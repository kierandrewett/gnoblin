// The handle returned by `gnoblin.requestTransition`: controls its tracks and reports how they ended. The tracks, their owners and the
// end are the library's (`gnoblin-std/generated`'s `Transition`); this class is the script's spelling of one.

import { ControlKind, TransitionKind } from "gnoblin-std/generated/constants";
import { Transition as Native } from "gnoblin-std/generated/classes";
import type { TransitionInfo } from "gnoblin-std/generated/records";
import { stdFailure } from "gnoblin-std/native_error";
import type { TransitionHost, TransitionResult, TransitionSubscription } from "gnoblin-std/transition/types";

/** The people who ask to be told when a transition ends. */
interface Waiter {
    run: (result: TransitionResult) => void;
    live: boolean;
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
    #native: Native;
    #host: TransitionHost;
    #state: "running" | "finished" | "cancelled" = "running";
    #waiters: Waiter[] = [];
    #promise: Promise<TransitionResult> | null = null;
    #resolve: ((result: TransitionResult) => void) | null = null;
    /** What the transition was when it ended: the library forgets an ended one after a while. */
    #last: TransitionInfo;

    /** Only the engine makes transitions: a script receives one from `requestTransition`. */
    constructor(host: TransitionHost, native: Native) {
        this.#host = host;
        this.#native = native;
        this.#last = native.info();
        this.id = this.#last.id;
        this.duration = this.#last.durationMs;
        host.staging.track(1);
        // The library says when it ends: its tracks all ended, a newer write took the last of them, or it was cancelled.
        native.finished((event) => {
            this.settle(event.flag !== 0);
        });
    }

    /** What the library says the transition is, while it still does. */
    #info(): TransitionInfo {
        if (this.#state === "running") {
            this.#last = this.#native.info();
        }
        return this.#last;
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
        return this.#info().paused !== 0;
    }

    /** Whether the tracks run back to their start. */
    get reversed(): boolean {
        return this.#info().reversed !== 0;
    }

    /**
     * Where the tracks were last put, from 0 (the start) to 1 (the target), before the easing. Writing it moves every
     * track to that point of its duration at once and draws it; it does not play or pause. Reading it gives what was
     * last written, 0 before that: the compositor does not report a running transition's place.
     */
    get progress(): number {
        return this.#info().progress;
    }

    /** Sets the track position from 0 (start) to 1 (target), without playing or pausing. */
    set progress(value: number) {
        if (typeof value !== "number" || !Number.isFinite(value) || value < 0 || value > 1) {
            throw new RangeError(`Transition.progress: expects a number from 0 to 1, not ${String(value)}`);
        }
        this.#control(ControlKind.SEEK, value, "Transition.progress");
    }

    /** Lets the tracks run on towards the target, from where they are. */
    play(): void {
        this.#control(ControlKind.PLAY, 0, "Transition.play()");
    }

    /** Holds the tracks where they are. */
    pause(): void {
        this.#control(ControlKind.PAUSE, 0, "Transition.pause()");
    }

    /**
     * Runs the tracks back towards their start from where they are. When they get there the transition settles
     * `{ cancelled: true }` and the start values are back in the tree.
     */
    reverse(): void {
        this.#control(ControlKind.REVERSE, 0, "Transition.reverse()");
    }

    /** A control: refused inside a `requestTransition` callback, a no-op once the transition ended. */
    #control(op: number, value: number, target: string): void {
        this.#host.staging.forbid(target);
        if (this.#state !== "running") {
            return;
        }
        try {
            this.#native.control(op, value);
        } catch (error) {
            throw stdFailure(error, target);
        }
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
        const entry: Waiter = { run: fn, live: true };
        if (this.#state === "running") {
            this.#waiters.push(entry);
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
        this.#control(ControlKind.CANCEL, 0, "Transition.cancel()");
        this.settle(true);
    }

    /** Settles the transition. The library's word and `cancel` only: a second settle does nothing. */
    settle(cancelled: boolean): void {
        if (this.#state !== "running") {
            return;
        }
        this.#last = { ...this.#info(), state: cancelled ? TransitionKind.CANCELLED : TransitionKind.FINISHED };
        this.#state = cancelled ? "cancelled" : "finished";
        this.#host.staging.track(-1);
        const result: TransitionResult = Object.freeze({ cancelled });
        this.#resolve?.(result);
        for (const entry of this.#waiters.splice(0)) {
            this.#deliver(entry, result);
        }
    }

    #deliver(entry: Waiter, result?: TransitionResult): void {
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
