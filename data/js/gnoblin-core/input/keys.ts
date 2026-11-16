// Key chords: a press of a claimed chord runs the highest-precedence binding that does not decline.

import type { ClaimEntry } from "gnoblin-core/input/registration";
import type { Hooks, MatchEnv } from "gnoblin-core/input/hooks";

/** Matches the presses of the key chords a context claims. */
export class KeyMatcher {
    readonly #env: MatchEnv;
    readonly #hooks: Hooks;

    constructor(env: MatchEnv, hooks: Hooks) {
        this.#env = env;
        this.#hooks = hooks;
    }

    /** A press of `claim`: schedules the first binding that has an `onPress` and does not decline. */
    press(claim: ClaimEntry): void {
        const client = this.#env.focusedClient();
        const candidates = this.#env.claims
            .candidates(claim, client)
            .filter((r) => typeof r.binding.onPress === "function");
        if (candidates.length === 0) {
            return;
        }
        this.#env.world.schedule(`key ${claim.specKey}`, () => {
            const at = this.#env.pointer();
            for (const registration of candidates) {
                if (!this.#env.claims.has(registration)) {
                    continue;
                }
                const { event } = this.#hooks.event(registration, client, at, this.#env.host.now());
                const outcome = this.#hooks.call(registration, "onPress", () => registration.binding.onPress?.(event));
                // A throw is that binding's answer: it handled the press, badly. Only a `false` declines.
                if (!outcome.ok || outcome.value !== false) {
                    return;
                }
            }
        });
    }
}
