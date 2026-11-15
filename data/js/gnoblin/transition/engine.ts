// `TransitionEngine`: runs the callback once, stages its writes, sends them in the shell's one transaction of the
// frame with the duration and the easing's samples, and owns the rule that a node has one transition at a time. A
// transition that fails leaves the tree as it was and puts back the script state that `snapshot` covers.

import { curveSamples, DEFAULT_EASING, EASING_NAMES, easingCurve, EASINGS } from "gnoblin/transition/easing";
import { copy } from "gnoblin/transition/staging";
import type { Staged, Value } from "gnoblin/transition/staging";
import { Transition } from "gnoblin/transition/transition";
import type { TransitionControl } from "gnoblin/transition/transition";
import { DEFAULT_DURATION_MS } from "gnoblin/transition/types";
import type { TrackEndedEvent, TransitionHost, TransitionOptions } from "gnoblin/transition/types";

const PLACE_KEYS = ["x", "y", "width", "height"];

/**
 * A track a transition started: where its property starts and ends, so `cancel` can land it and a rollback restore
 * it.
 */
interface Owned {
    node: number;
    key: string;
    value: Value;
    from: Value;
}

/** One transition with tracks running, as the engine keeps it. */
interface Active {
    /** The transition itself, set once it is made: its control, which the tracks' writes go through, is made first. */
    transition: Transition | null;
    /** The compositor's `tween` node its tracks joined. */
    tween: number;
    /** The tracks that have not ended, by `node:key`. */
    owned: Map<string, Owned>;
    /** Every property it wrote with a track, by `node:key`, so a rollback can put the start values back. */
    all: Map<string, Owned>;
    /** The properties a newer write took from it: they are the newer write's now. */
    released: Set<string>;
    /** Whether any track ended other than by reaching its target. */
    cancelled: boolean;
    /** Whether `reverse()` is the last word on its direction. */
    reversed: boolean;
}

/** The checked options: the milliseconds of the animation, its delay, the samples of its easing and whether it plays. */
interface Timing {
    duration: number;
    delay: number;
    samples: number[];
    autoplay: boolean;
}

function checkOptions(options: TransitionOptions | undefined, site: string): Timing {
    if (options === undefined) {
        return {
            duration: DEFAULT_DURATION_MS,
            delay: 0,
            samples: curveSamples(EASINGS[DEFAULT_EASING]),
            autoplay: true,
        };
    }
    if (options === null || typeof options !== "object") {
        throw new TypeError(`${site}: options must be an object`);
    }
    const known = ["duration", "easing", "delay", "autoplay"];
    for (const key of Object.keys(options)) {
        if (!known.includes(key)) {
            throw new TypeError(`${site}: unknown option "${key}" (known: ${known.join(", ")})`);
        }
    }
    const duration = options.duration ?? DEFAULT_DURATION_MS;
    if (typeof duration !== "number" || !Number.isFinite(duration) || duration < 0) {
        throw new TypeError(`${site}: duration must be a finite number of milliseconds of at least 0`);
    }
    const delay = options.delay ?? 0;
    if (typeof delay !== "number" || !Number.isFinite(delay) || delay < 0) {
        throw new TypeError(`${site}: delay must be a finite number of milliseconds of at least 0`);
    }
    const autoplay = options.autoplay ?? true;
    if (typeof autoplay !== "boolean") {
        throw new TypeError(`${site}: autoplay must be true or false`);
    }
    const easing = easingCurve(options.easing ?? DEFAULT_EASING);
    if (easing === undefined) {
        throw new TypeError(`${site}: easing must be one of ${EASING_NAMES}`);
    }
    return { duration, delay, samples: curveSamples(easing), autoplay };
}

function isThenable(value: unknown): boolean {
    return (
        value !== null &&
        (typeof value === "object" || typeof value === "function") &&
        "then" in value &&
        typeof value.then === "function"
    );
}

/** Runs callbacks as transitions over a host, and keeps which transition owns which property. */
export class TransitionEngine {
    #host: TransitionHost;
    /** The transition whose track runs on each `node:key`. */
    #owners = new Map<string, Active>();
    /** The transitions with tracks running, by the `tween` node their tracks joined. */
    #active = new Map<number, Active>();

    constructor(host: TransitionHost) {
        this.#host = host;
        // A plain write ends the compositor's track on its property, so the transition no longer owns it.
        host.staging.onWrite = (node, key) => this.#release(`${node}:${key}`);
    }

    /** How many tracks transitions own now, for a test: `running` transitions and their properties. */
    get owned(): number {
        return this.#owners.size;
    }

    /** How many transitions have tracks running. */
    get running(): number {
        return this.#active.size;
    }

    /** Ends the generation's hold on the compositor: the `tween` nodes of transitions still running go. */
    release(): void {
        for (const active of this.#active.values()) {
            this.#host.dropTween(active.tween);
        }
        this.#active.clear();
        this.#owners.clear();
    }

    request(callback: () => void, options?: TransitionOptions): Transition {
        const site = "gnoblin.requestTransition";
        if (typeof callback !== "function") {
            throw new TypeError(`${site}: the first argument must be a function that stages the changes`);
        }
        const host = this.#host;
        if (host.staging.active) {
            throw host.error(
                "busy",
                site,
                "was called inside a requestTransition callback; stage every change in the outer callback instead",
            );
        }
        const timing = checkOptions(options, site);
        const restore = host.snapshot();
        host.staging.begin();
        let entries: Staged[];
        try {
            const returned: unknown = callback();
            if (isThenable(returned)) {
                throw new TypeError(
                    `${site}: the callback returned a Promise; it runs once and synchronously, so it cannot be async or await`,
                );
            }
            host.settle();
            entries = host.staging.end();
        } catch (error) {
            host.staging.discard();
            restore();
            throw error;
        }
        return this.#commit(entries, timing);
    }

    /**
     * The compositor's word that a timed write ended. A track that is not one a transition still owns is ignored: a
     * newer write took the property, or the transition was cancelled, and the transition has already settled for it.
     */
    trackEnded(event: TrackEndedEvent): void {
        const active = this.#active.get(event.tween);
        if (active === undefined) {
            return;
        }
        const id = `${event.node}:${event.property}`;
        if (!active.owned.delete(id)) {
            return;
        }
        this.#owners.delete(id);
        if (!event.completed) {
            active.cancelled = true;
        }
        if (active.owned.size === 0) {
            this.#finish(active);
        }
    }

    #commit(entries: Staged[], timing: Timing): Transition {
        const host = this.#host;
        const duration = host.motion(timing.duration);
        // Under reduced motion nothing waits either.
        const delay = duration > 0 ? timing.delay : 0;
        // A node that was not drawn (no size, or hidden) has no place to tween from: its place is written at once.
        const appears = new Map<number, boolean>();
        const plan = entries.map((entry) => {
            const curve = entry.curve ?? (duration > 0 ? { duration, samples: timing.samples } : null);
            let appearing = appears.get(entry.node);
            if (appearing === undefined) {
                appearing = !host.placed(entry.node);
                appears.set(entry.node, appearing);
            }
            const timed =
                curve !== null &&
                curve.duration > 0 &&
                host.animated(entry.key) &&
                !(appearing && PLACE_KEYS.includes(entry.key));
            return { entry, curve: timed ? curve : null };
        });
        const tracks = plan.filter((item) => item.curve !== null).length;
        // The tracks join a `tween` node of the compositor, so that its word on how each ended names this transition.
        const tween = tracks > 0 ? host.makeTween() : null;
        const active: Active | null =
            tween === null
                ? null
                : {
                      transition: null,
                      tween,
                      owned: new Map(),
                      all: new Map(),
                      released: new Set(),
                      cancelled: false,
                      reversed: false,
                  };
        const control: TransitionControl | null =
            active === null
                ? null
                : {
                      seek: (progress) => host.staging.land(active.tween, "progress", progress, null),
                      pause: (paused) => host.staging.land(active.tween, "paused", paused, null),
                      reverse: (reverse) => {
                          active.reversed = reverse;
                          host.staging.land(active.tween, "reverse", reverse, null);
                      },
                      cancel: () => this.#cancel(active),
                  };
        const transition = new Transition(host, duration, timing.autoplay, control);
        if (active !== null && tween !== null) {
            active.transition = transition;
            if (!timing.autoplay) {
                host.staging.land(tween, "paused", true, null);
            }
        }
        const refused: string[] = [];
        for (const { entry, curve } of plan) {
            const id = `${entry.node}:${entry.key}`;
            this.#release(id);
            const from = copy(host.staging.get(entry.node, entry.key));
            const sent = curve === null || tween === null ? null : { ...curve, delay, tween };
            const code = host.staging.land(entry.node, entry.key, entry.value, sent);
            if (code !== undefined && code !== null && code !== "unknown-node") {
                refused.push(`${host.label(entry.node)}.${entry.key} (${code})`);
                continue;
            }
            if (sent !== null && active !== null) {
                const owned = { node: entry.node, key: entry.key, value: entry.value, from };
                active.owned.set(id, owned);
                active.all.set(id, owned);
                this.#owners.set(id, active);
            }
        }
        if (refused.length > 0) {
            if (active !== null) {
                this.#forget(active);
            }
            throw host.error("denied", "gnoblin.requestTransition", `the host refused ${refused.join(", ")}`);
        }
        if (active === null) {
            host.schedule(`Transition#${transition.id}`, () => transition.settle(false));
        } else if (active.owned.size === 0) {
            // Every track was refused as for an unknown node: there is nothing for the compositor to end.
            this.#finish(active);
        } else {
            this.#active.set(active.tween, active);
        }
        return transition;
    }

    /**
     * A property written again: the transition that owned it no longer does, and one that has no track left is over.
     */
    #release(id: string): void {
        const held = this.#owners.get(id);
        if (held === undefined) {
            return;
        }
        this.#owners.delete(id);
        held.owned.delete(id);
        held.released.add(id);
        if (held.owned.size === 0) {
            // A transition that lost its last property to a newer write is over, cancelled; one that lost some goes on.
            held.cancelled = true;
            this.#finish(held);
        }
    }

    /**
     * A transition whose tracks have all ended: it settles, `{ cancelled: true }` when any did not reach its target.
     * One that ran back to its start has the start values put back first (a property a newer write took stays the
     * newer write's).
     */
    #finish(active: Active): void {
        this.#active.delete(active.tween);
        if (active.reversed && active.cancelled) {
            for (const [id, owned] of active.all) {
                if (!active.released.has(id)) {
                    this.#host.staging.land(owned.node, owned.key, owned.from, null);
                }
            }
        }
        this.#host.dropTween(active.tween);
        active.transition?.settle(active.cancelled);
    }

    /** A transition that failed to commit forgets what it took, and its `tween` node goes. */
    #forget(active: Active): void {
        for (const id of active.owned.keys()) {
            this.#owners.delete(id);
        }
        this.#active.delete(active.tween);
        this.#host.dropTween(active.tween);
    }

    #cancel(active: Active): void {
        for (const [id, owned] of [...active.owned]) {
            active.owned.delete(id);
            this.#owners.delete(id);
            // The instant write lands the end value and ends the track the compositor runs on the property.
            this.#host.staging.land(owned.node, owned.key, owned.value, null);
        }
        this.#active.delete(active.tween);
        this.#host.dropTween(active.tween);
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
