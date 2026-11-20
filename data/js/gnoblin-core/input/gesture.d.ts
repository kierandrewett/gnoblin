import type { BindingClient } from "gnoblin-core/bindings/types";
import type { Hooks, MatchEnv } from "gnoblin-core/input/hooks";
import type { InputGestureMessage } from "gnoblin-core/input/messages";
import type { ClaimEntry, Registration } from "gnoblin-core/input/registration";
/** Minimum progress for a swipe action to run when the gesture ends. */
export declare const SWIPE_COMMIT: number;
/** Minimum progress for a pinch action to run when the gesture ends. */
export declare const PINCH_COMMIT: number;
/** The gestures in flight on one context, by claim. */
export declare class GestureMatcher {
    #private;
    constructor(env: MatchEnv, hooks: Hooks);
    /**
     * One step of a claimed gesture: `begin` picks the candidates, `update` and `end` run the hooks, `cancel` stops
     * it.
     */
    message(claim: ClaimEntry, message: InputGestureMessage): void;
    /** A client died: the gesture on it is cancelled once. */
    cancelOn(client: BindingClient): void;
    /** A binding was removed: the gesture it runs ends here, so nothing could finish it. */
    cancelBy(registration: Registration): void;
    /** Cancels every gesture in flight. */
    cancelAll(): void;
}
