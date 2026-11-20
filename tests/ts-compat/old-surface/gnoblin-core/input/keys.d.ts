import type { ClaimEntry } from "gnoblin-core/input/registration";
import type { Hooks, MatchEnv } from "gnoblin-core/input/hooks";
/** Matches the presses of the key chords a context claims. */
export declare class KeyMatcher {
    #private;
    constructor(env: MatchEnv, hooks: Hooks);
    /** A press of `claim`: schedules the first binding that has an `onPress` and does not decline. */
    press(claim: ClaimEntry): void;
}
