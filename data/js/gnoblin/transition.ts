// `gnoblin.requestTransition`: state changes the compositor draws as one coordinated animation.
//
//     gnoblin.requestTransition(() => {
//         client.opacity = 0.85;
//         workspace.layout = other;
//     }, { duration: 250, easing: "ease-out" });
//
// The callback runs once, now. While it runs every write to a node (the host's `set`) is staged, not queued: a read gives
// the staged value, and nothing reaches the tree until the callback and the layout and style passes after it have
// finished without an error. Then the staged writes are sent in the shell's one transaction of the frame, each with the
// duration and the easing's samples that the tree's timed write takes, so the compositor starts a track from the value it
// draws now to the staged one. A transition that fails leaves the tree as it was and puts back the script state that
// `snapshot` covers.
//
// Plain TypeScript over a small host (`TransitionHost`), so the staging and the ownership rules run without a compositor.
// What the native tween player cannot do is not faked here (`docs/js-transitions.md`, "Not built").

/** How an animation moves from its start to its end, as the Lua shell's `ease` table named them. */
export type ClientEasing = "linear" | "out" | "out-cubic" | "in-cubic" | "in-out";

export const EASINGS: Readonly<Record<ClientEasing, (t: number) => number>> = {
    linear: (t) => t,
    out: (t) => 1 - (1 - t) ** 2,
    "out-cubic": (t) => 1 - (1 - t) ** 3,
    "in-cubic": (t) => t ** 3,
    "in-out": (t) => (t < 0.5 ? 2 * t * t : 1 - (2 - 2 * t) ** 2 / 2),
};

/** The curve is sent as this many steps plus the start: 33 samples, as the Lua shell's `node:to` sends. */
export const CURVE_STEPS = 32;

/** The samples of `curve`: progress from 0 to 1 over `CURVE_STEPS` steps, which is what a timed write carries. */
export function curveSamples(curve: (t: number) => number): number[] {
    return Array.from({ length: CURVE_STEPS + 1 }, (_, i) => curve(i / CURVE_STEPS));
}

/** The CSS names of the spec, each the curve of `ClientEasing` nearest to it. */
const EASING_ALIASES: Readonly<Record<string, ClientEasing>> = {
    "ease-out": "out",
    "ease-in-out": "in-out",
    "ease-in": "in-cubic",
};

/** An easing a transition takes: a `ClientEasing`, or a CSS name of the spec. */
export type TransitionEasing = ClientEasing | "ease-out" | "ease-in-out" | "ease-in";

/** The milliseconds a transition takes when `duration` is not given. */
export const DEFAULT_DURATION_MS = 250;

export interface TransitionOptions {
    /** Milliseconds, 0 or more. Default 250. Zero commits the change at once (still as one transaction). */
    duration?: number;
    /** Default `"out"`, as `Client.animate`'s. */
    easing?: TransitionEasing;
}

/** What `finished` and `onFinished` deliver: `cancelled` is true when `cancel()` or a newer transition took every track. */
export interface TransitionResult {
    readonly cancelled: boolean;
}

/** Stops a callback before it runs. Calling it twice is harmless. */
export interface TransitionSubscription {
    unsubscribe(): void;
}

/** A value as it goes to the tree: a number, a flag, a text, a list of numbers or effect objects. */
type Value = unknown;

/** The timed write a staged value is sent as, when it has a curve of its own (`Client.animate` inside a callback). */
interface Curve {
    seconds: number;
    samples: number[];
}

interface Staged {
    node: number;
    key: string;
    value: Value;
    curve: Curve | null;
}

/** The host functions staging stands in front of. Codes are the tree's: `unknown-node`, `read-only`, `invalid-value` ... */
export interface StageBackend {
    get(node: number, key: string): Value;
    set(node: number, key: string, value: Value): string | undefined;
    animate(node: number, key: string, value: Value, seconds: number, samples: number[]): string | undefined;
    /** What `set` would answer, without queuing the write; with `timed` it also refuses a property the tree cannot animate. */
    check(node: number, key: string, value: Value, timed: boolean): string | undefined;
}

/** Makes the error a failed transition throws: `code` says why, `target` names the member or call, `message` the reason. */
export type MakeError = (code: string, target: string, message: string) => Error;

const copy = (value: Value): Value => (Array.isArray(value) ? [...value] : value);

/**
 * The writes of one callback. Outside a callback it is the host: every call goes straight through, and a write tells
 * `onWrite` so that a transition that owned the property lets it go. Inside, a write is checked at once (so the setter
 * that made it throws, naming its own path) and kept until `end`.
 */
export class Staging {
    #backend: StageBackend;
    #entries: Map<string, Staged> | null = null;
    #error: MakeError;
    /** Told of each write made outside a callback, before it is queued. */
    onWrite: ((node: number, key: string) => void) | null = null;

    constructor(backend: StageBackend, error: MakeError) {
        this.#backend = backend;
        this.#error = error;
    }

    get active(): boolean {
        return this.#entries !== null;
    }

    begin(): void {
        this.#entries = new Map();
    }

    /** Stops staging and hands over what it kept, in the order the properties were first written. */
    end(): Staged[] {
        const kept = [...(this.#entries ?? []).values()];
        this.#entries = null;
        return kept;
    }

    /** Stops staging and drops what it kept. */
    discard(): void {
        this.#entries = null;
    }

    get(node: number, key: string): Value {
        const staged = this.#entries?.get(`${node}:${key}`);
        return staged === undefined ? this.#backend.get(node, key) : copy(staged.value);
    }

    set(node: number, key: string, value: Value): string | undefined {
        if (this.#entries === null) {
            this.onWrite?.(node, key);
            return this.#backend.set(node, key, value);
        }
        const code = this.#backend.check(node, key, value, false);
        if (code === undefined || code === null) {
            this.#keep({ node, key, value: copy(value), curve: null });
            return undefined;
        }
        return code;
    }

    animate(node: number, key: string, value: Value, seconds: number, samples: number[]): string | undefined {
        if (this.#entries === null) {
            this.onWrite?.(node, key);
            return this.#backend.animate(node, key, value, seconds, samples);
        }
        const code = this.#backend.check(node, key, value, true);
        if (code === undefined || code === null) {
            this.#keep({ node, key, value: copy(value), curve: { seconds, samples } });
            return undefined;
        }
        return code;
    }

    /** Queues a write of the transaction's own, which is not a script's: it does not tell `onWrite`. */
    land(node: number, key: string, value: Value, curve: Curve | null): string | undefined {
        return curve === null
            ? this.#backend.set(node, key, value)
            : this.#backend.animate(node, key, value, curve.seconds, curve.samples);
    }

    /** Throws for a call that has a side effect, while a callback is staging. Otherwise does nothing. */
    forbid(target: string): void {
        if (this.#entries !== null) {
            throw this.#error(
                "forbidden",
                target,
                "is not valid inside a requestTransition callback: it has a side effect, and a callback only stages property writes",
            );
        }
    }

    #keep(entry: Staged): void {
        this.#entries?.set(`${entry.node}:${entry.key}`, entry);
    }
}

/** What an engine needs of the shell. */
export interface TransitionHost {
    staging: Staging;
    /** The seconds an animation of `seconds` takes: 0 while the seat asks for reduced motion. */
    motion(seconds: number): number;
    /** Whether the tree takes a timed write of `key`. */
    animated(key: string): boolean;
    /** Whether a node has a place that is drawn: a size, and not hidden. A node that has none just appears. */
    placed(node: number): boolean;
    /** The name of a node in an error path: `Client#3`. */
    label(node: number): string;
    /** Runs `fn` `seconds` from now, at the first frame past it. */
    after(seconds: number, fn: () => void): { cancel(): boolean };
    /** Queues `task` after the calls already queued, under the listener budget. `site` names it in a report. */
    schedule(site: string, task: () => void): void;
    /** Takes what a callback can change that is not a tree write, and returns what puts it back. */
    snapshot(): () => void;
    /** Runs the style and layout passes once against the staged state, so their writes are staged too. */
    settle(): void;
    error: MakeError;
}

const PLACE_KEYS = ["x", "y", "width", "height"];

let counter = 0;

/**
 * One call of `requestTransition`. `finished` settles once, with `{ cancelled }`; `onFinished` runs a plain callback
 * once as a queued call, one added after the settle runs after the current call returns.
 *
 * What is not here, and why: `progress`, `play`, `pause`, `reverse` and `finish` need the native tween player to take an
 * external progress. It advances tracks by elapsed milliseconds and cannot be set to a value
 * (`crates/gnoblin-render/src/tween.rs`), so `autoplay: false` is refused.
 */
export class Transition {
    readonly id: number;
    /** The milliseconds the animation takes: the duration asked for, or 0 under reduced motion. */
    readonly duration: number;
    #host: TransitionHost;
    #state: "running" | "finished" | "cancelled" = "running";
    #callbacks: { run: (result: TransitionResult) => void; live: boolean }[] = [];
    #promise: Promise<TransitionResult> | null = null;
    #resolve: ((result: TransitionResult) => void) | null = null;
    #cancel: (transition: Transition) => void;
    #timer: { cancel(): boolean } | null = null;

    /** Only the engine makes transitions: a script receives one from `requestTransition`. */
    constructor(host: TransitionHost, duration: number, cancel: (transition: Transition) => void) {
        this.id = counter += 1;
        this.duration = duration;
        this.#host = host;
        this.#cancel = cancel;
    }

    /** True from the call until the transition finishes or is cancelled. */
    get running(): boolean {
        return this.#state === "running";
    }

    /** True once `cancel()` or a newer transition took every track. */
    get cancelled(): boolean {
        return this.#state === "cancelled";
    }

    /**
     * A promise for the end. It is made when first asked for, so a script that never reads it leaves no unhandled
     * promise behind. It resolves once, with `{ cancelled }`, and never rejects.
     */
    get finished(): Promise<TransitionResult> {
        if (this.#promise === null) {
            const { promise, resolve } = Promise.withResolvers<TransitionResult>();
            this.#promise = promise;
            this.#resolve = resolve;
            if (this.#state !== "running") {
                resolve({ cancelled: this.#state === "cancelled" });
            }
        }
        return this.#promise;
    }

    /** Calls `fn({ cancelled })` once, when the transition ends. The returned subscription stops it before it runs. */
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
            this.#cancel(this);
        }
    }

    /** Arms the timer that ends the transition `seconds` from now. Engine only. */
    arm(timer: { cancel(): boolean }): void {
        this.#timer = timer;
    }

    /** Settles the transition. Engine only: a second settle does nothing. */
    settle(cancelled: boolean): void {
        if (this.#state !== "running") {
            return;
        }
        this.#state = cancelled ? "cancelled" : "finished";
        this.#timer?.cancel();
        this.#timer = null;
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

    toString(): string {
        return `Transition#${this.id}`;
    }
}

/** A track a transition started: where its property ends, so `cancel` can land it. */
interface Owned {
    node: number;
    key: string;
    value: Value;
}

/** The checked options: the seconds of the animation and the samples of its easing. */
interface Timing {
    ms: number;
    samples: number[];
}

function easingOf(name: unknown): ((t: number) => number) | undefined {
    if (typeof name !== "string") {
        return undefined;
    }
    if (Object.hasOwn(EASINGS, name)) {
        return EASINGS[name as ClientEasing];
    }
    return Object.hasOwn(EASING_ALIASES, name) ? EASINGS[EASING_ALIASES[name]] : undefined;
}

function checkOptions(options: TransitionOptions | undefined, site: string): Timing {
    if (options === undefined) {
        return { ms: DEFAULT_DURATION_MS, samples: curveSamples(EASINGS.out) };
    }
    if (options === null || typeof options !== "object") {
        throw new TypeError(`${site}: options must be an object`);
    }
    const known = ["duration", "easing"];
    const extra = options as Record<string, unknown>;
    for (const key of Object.keys(extra)) {
        const refused =
            (key === "autoplay" && extra.autoplay !== true && extra.autoplay !== undefined) ||
            (key === "delay" && extra.delay !== 0 && extra.delay !== undefined);
        if (refused) {
            throw new TypeError(
                `${site}: ${key} is not supported: the compositor's tween player has no ${
                    key === "delay" ? "delay on a timed write" : "external progress to scrub"
                } (docs/js-transitions.md, "Not built")`,
            );
        }
        if (!known.includes(key) && key !== "autoplay" && key !== "delay") {
            throw new TypeError(`${site}: unknown option "${key}" (known: ${known.join(", ")})`);
        }
    }
    const duration = options.duration ?? DEFAULT_DURATION_MS;
    if (typeof duration !== "number" || !Number.isFinite(duration) || duration < 0) {
        throw new TypeError(`${site}: duration must be a finite number of milliseconds of at least 0`);
    }
    const easing = easingOf(options.easing ?? "out");
    if (easing === undefined) {
        const names = [...Object.keys(EASINGS), ...Object.keys(EASING_ALIASES)];
        throw new TypeError(`${site}: easing must be one of ${names.join(", ")}`);
    }
    return { ms: duration, samples: curveSamples(easing) };
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
    /** The transition whose track runs on each `node:key`, and where that track ends. */
    #owners = new Map<string, { transition: Transition; owned: Owned }>();

    constructor(host: TransitionHost) {
        this.#host = host;
        // A plain write ends the compositor's track on its property, so the transition no longer owns it.
        host.staging.onWrite = (node, key) => this.#release(`${node}:${key}`);
    }

    /** How many tracks a transition owns now, for a test: `running` transitions and their properties. */
    get owned(): number {
        return this.#owners.size;
    }

    request(callback: () => void, options?: TransitionOptions): Transition {
        const site = "gnoblin.requestTransition";
        if (typeof callback !== "function") {
            throw new TypeError(`${site}: the first argument must be a function that stages the changes`);
        }
        const host = this.#host;
        if (host.staging.active) {
            throw host.error(
                "nested",
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

    #commit(entries: Staged[], timing: Timing): Transition {
        const host = this.#host;
        const seconds = host.motion(timing.ms / 1000);
        const transition = new Transition(host, seconds * 1000, (stopped) => this.#cancel(stopped));
        const refused: string[] = [];
        // A node that was not drawn (no size, or hidden) has no place to tween from: its place is written at once.
        const appears = new Map<number, boolean>();
        let longest = 0;
        let tracks = 0;
        for (const entry of entries) {
            const curve = entry.curve ?? (seconds > 0 ? { seconds, samples: timing.samples } : null);
            let appearing = appears.get(entry.node);
            if (appearing === undefined) {
                appearing = !host.placed(entry.node);
                appears.set(entry.node, appearing);
            }
            const timed =
                curve !== null &&
                curve.seconds > 0 &&
                host.animated(entry.key) &&
                !(appearing && PLACE_KEYS.includes(entry.key));
            const id = `${entry.node}:${entry.key}`;
            this.#release(id);
            const sent = timed ? curve : null;
            const code = host.staging.land(entry.node, entry.key, entry.value, sent);
            if (code !== undefined && code !== null && code !== "unknown-node") {
                refused.push(`${host.label(entry.node)}.${entry.key} (${code})`);
                continue;
            }
            if (sent !== null) {
                this.#owners.set(id, { transition, owned: { node: entry.node, key: entry.key, value: entry.value } });
                longest = Math.max(longest, sent.seconds);
                tracks += 1;
            }
        }
        if (refused.length > 0) {
            throw host.error("refused", "gnoblin.requestTransition", `the host refused ${refused.join(", ")}`);
        }
        if (tracks === 0) {
            host.schedule(`Transition#${transition.id}`, () => transition.settle(false));
        } else {
            transition.arm(
                host.after(longest, () => {
                    this.#forget(transition);
                    transition.settle(false);
                }),
            );
        }
        return transition;
    }

    /** A property written again: the transition that owned it no longer does, and one that has no track left is over. */
    #release(id: string): void {
        const held = this.#owners.get(id);
        if (held === undefined) {
            return;
        }
        this.#owners.delete(id);
        if (![...this.#owners.values()].some((other) => other.transition === held.transition)) {
            held.transition.settle(true);
        }
    }

    #forget(transition: Transition): void {
        for (const [id, held] of this.#owners) {
            if (held.transition === transition) {
                this.#owners.delete(id);
            }
        }
    }

    #cancel(transition: Transition): void {
        for (const [id, held] of [...this.#owners]) {
            if (held.transition !== transition) {
                continue;
            }
            this.#owners.delete(id);
            // The instant write lands the end value and ends the track the compositor runs on the property.
            this.#host.staging.land(held.owned.node, held.owned.key, held.owned.value, null);
        }
        transition.settle(true);
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
