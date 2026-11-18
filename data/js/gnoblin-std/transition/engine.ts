// Runs transition callbacks over the library's staging. The library holds the tracks, who owns each property and the end of a
// transition (`transition_begin`, `transition_commit`); this class runs the shell's callback between the two, restores the script
// state a failed callback changed, and checks the options as a script writes them.
//
// A failed callback leaves the tree unchanged: the library drops what it staged, and `snapshot` puts back what is not a tree write.

import { easingKind, DEFAULT_EASING, EASING_NAMES } from "gnoblin-std/transition/easing";
import type { TransitionOptions as NativeOptions } from "gnoblin-std/generated/records";
import type { Init } from "gnoblin-std/generated/native";
import { Transition } from "gnoblin-std/transition/transition";
import { DEFAULT_DURATION_MS } from "gnoblin-std/transition/types";
import type { TransitionHost, TransitionOptions } from "gnoblin-std/transition/types";

/**
 * The options a script gave, as the library takes them. What the library checks (a finite duration and delay of at least 0) it
 * checks; what only a script can get wrong (an option that does not exist, a value of another type) is refused here.
 */
function optionsOf(options: TransitionOptions | undefined, site: string): Init<NativeOptions> {
    const given = options ?? {};
    if (options !== undefined) {
        if (options === null || typeof options !== "object") {
            throw new TypeError(`${site}: options must be an object`);
        }
        const known = ["duration", "easing", "delay", "autoplay"];
        for (const key of Object.keys(options)) {
            if (!known.includes(key)) {
                throw new TypeError(`${site}: unknown option "${key}" (known: ${known.join(", ")})`);
            }
        }
    }
    const duration = given.duration ?? DEFAULT_DURATION_MS;
    if (typeof duration !== "number") {
        throw new TypeError(`${site}: duration must be a finite number of milliseconds of at least 0`);
    }
    const delay = given.delay ?? 0;
    if (typeof delay !== "number") {
        throw new TypeError(`${site}: delay must be a finite number of milliseconds of at least 0`);
    }
    const autoplay = given.autoplay ?? true;
    if (typeof autoplay !== "boolean") {
        throw new TypeError(`${site}: autoplay must be true or false`);
    }
    const easing = easingKind(given.easing ?? DEFAULT_EASING);
    if (easing === undefined) {
        throw new TypeError(`${site}: easing must be one of ${EASING_NAMES}`);
    }
    return { durationMs: duration, delayMs: delay, easing, autoplay: autoplay ? 1 : 0 };
}

function isThenable(value: unknown): boolean {
    return (
        value !== null &&
        (typeof value === "object" || typeof value === "function") &&
        "then" in value &&
        typeof value.then === "function"
    );
}

/** Runs callbacks as transitions over a host. */
export class TransitionEngine {
    #host: TransitionHost;

    constructor(host: TransitionHost) {
        this.#host = host;
    }

    /** Runs `callback` once and stages its property writes as a transition. */
    request(callback: () => void, options?: TransitionOptions): Transition {
        const site = "gnoblin.requestTransition";
        if (typeof callback !== "function") {
            throw new TypeError(`${site}: the first argument must be a function that stages the changes`);
        }
        const host = this.#host;
        const native = optionsOf(options, site);
        // The library refuses a call made inside a callback, with its words, before anything else happens.
        host.staging.begin(native);
        let restore = (): void => {};
        try {
            restore = host.snapshot();
            const returned: unknown = callback();
            if (isThenable(returned)) {
                throw new TypeError(
                    `${site}: the callback returned a Promise; it runs once and synchronously, so it cannot be async or await`,
                );
            }
            host.settle();
        } catch (error) {
            host.staging.discard();
            restore();
            throw error;
        }
        // The library runs the passes of the shell's layouts at the commit: a layout that fails fails the transition as a callback does.
        let committed;
        try {
            committed = host.staging.commit();
        } catch (error) {
            host.staging.discard();
            restore();
            throw error;
        }
        return new Transition(host, committed);
    }
}

/**
 * The shorthand of one object: `object.set(props, options)` is one `requestTransition` that assigns each property.
 * `label` names the object in an error path. A key that is not a property with a setter is refused before anything is
 * written. Without `options` the change is atomic and immediate.
 */
export function setProperties(
    engine: TransitionEngine,
    object: object,
    label: string,
    props: Record<string, unknown>,
    options: TransitionOptions | undefined,
): Transition {
    if (props === null || typeof props !== "object" || Array.isArray(props)) {
        throw new TypeError(`${label}.set: expects an object of properties`);
    }
    for (const key of Object.keys(props)) {
        let found: PropertyDescriptor | undefined;
        for (let at: object | null = object; at !== null && found === undefined; at = Object.getPrototypeOf(at)) {
            found = Object.getOwnPropertyDescriptor(at, key);
        }
        if (found === undefined || typeof found.set !== "function") {
            throw new TypeError(`${label}.set: ${key} is not a writable property`);
        }
    }
    return engine.request(
        () => {
            for (const [key, value] of Object.entries(props)) {
                (object as Record<string, unknown>)[key] = value;
            }
        },
        options ?? { duration: 0 },
    );
}
