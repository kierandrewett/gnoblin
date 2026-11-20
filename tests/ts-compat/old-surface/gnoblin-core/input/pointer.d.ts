import type { BindingClient } from "gnoblin-core/bindings/types";
import type { Hooks, MatchEnv } from "gnoblin-core/input/hooks";
import type { InputPointerMessage } from "gnoblin-core/input/messages";
import type { ClaimEntry, Registration } from "gnoblin-core/input/registration";
/** The pointer drags in flight on one context, by claim. */
export declare class PointerMatcher {
    #private;
    constructor(env: MatchEnv, hooks: Hooks);
    /** Records a press, schedules drag samples, or finishes the interaction for a chord message. */
    message(claim: ClaimEntry, message: InputPointerMessage): void;
    /** A client died: a press on it is dropped, and a drag on it is cancelled. */
    cancelOn(client: BindingClient): void;
    /** A binding was removed: the drag it runs ends here, so nothing could finish it. */
    cancelBy(registration: Registration): void;
    /** Cancels every drag in flight. */
    cancelAll(): void;
}
