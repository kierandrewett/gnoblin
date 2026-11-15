// `gnoblin.requestTransition`: state changes the compositor draws as one coordinated animation.
//
//     gnoblin.requestTransition(() => {
//         client.opacity = 0.85;
//         workspace.layout = other;
//     }, { duration: 250, easing: "ease-out" });
//
// The callback runs once, now. While it runs every write to a node (the host's `set`) is staged, not queued: a read
// gives the staged value, and nothing reaches the tree until the callback and the layout and style passes after it
// have finished without an error. Then the staged writes are sent in the shell's one transaction of the frame, each
// with the duration and the easing's samples that the tree's timed write takes, so the compositor starts a track from
// the value it draws now to the staged one. A transition that fails leaves the tree as it was and puts back the
// script state that `snapshot` covers.
//
// Plain TypeScript over a small host (`TransitionHost`), so the staging and the ownership rules run without a
// compositor. What the native tween player cannot do is not faked here (`docs/js-transitions.md`, "Not built").

/**
 * How an animation moves from its start to its end: the CSS names the specification uses. Each is the CSS timing
 * function of that name (`ease-out` is `cubic-bezier(0, 0, 0.58, 1)`), so a script that knows CSS knows the curve.
 */
export type Easing = "linear" | "ease" | "ease-in" | "ease-out" | "ease-in-out";

/** The control points `(x1, y1, x2, y2)` of the CSS cubic Bezier each named easing is. */
const BEZIER_POINTS: Readonly<Record<Exclude<Easing, "linear">, readonly [number, number, number, number]>> = {
    ease: [0.25, 0.1, 0.25, 1],
    "ease-in": [0.42, 0, 1, 1],
    "ease-out": [0, 0, 0.58, 1],
    "ease-in-out": [0.42, 0, 0.58, 1],
};

/** The CSS timing function of a Bezier through (0, 0) and (1, 1): progress in time onto progress in value. */
function cubicBezier(x1: number, y1: number, x2: number, y2: number): (t: number) => number {
    // The curve is parametric: x(s) and y(s) are cubics in s. Solving x(s) = t gives s, and y(s) is the value.
    const coordinate = (a: number, b: number, s: number): number =>
        3 * a * (1 - s) ** 2 * s + 3 * b * (1 - s) * s ** 2 + s ** 3;
    const slope = (a: number, b: number, s: number): number =>
        3 * a * (1 - s) ** 2 + 6 * (b - a) * (1 - s) * s + 3 * (1 - b) * s ** 2;
    return (t) => {
        if (t <= 0) {
            return 0;
        }
        if (t >= 1) {
            return 1;
        }
        // Newton's method converges in a few steps on these monotonic curves; bisection backs it up where the
        // slope is nearly flat.
        let s = t;
        for (let i = 0; i < 8; i += 1) {
            const error = coordinate(x1, x2, s) - t;
            if (Math.abs(error) < 1e-7) {
                return coordinate(y1, y2, s);
            }
            const d = slope(x1, x2, s);
            if (Math.abs(d) < 1e-6) {
                break;
            }
            s -= error / d;
        }
        let low = 0;
        let high = 1;
        s = t;
        for (let i = 0; i < 40; i += 1) {
            const x = coordinate(x1, x2, s);
            if (Math.abs(x - t) < 1e-7) {
                break;
            }
            if (x < t) {
                low = s;
            } else {
                high = s;
            }
            s = (low + high) / 2;
        }
        return coordinate(y1, y2, s);
    };
}

/** The curve for each `Easing`: it maps progress from 0 to 1 onto an eased value that also runs from 0 to 1. */
export const EASINGS: Readonly<Record<Easing, (t: number) => number>> = {
    linear: (t) => t,
    ease: cubicBezier(...BEZIER_POINTS.ease),
    "ease-in": cubicBezier(...BEZIER_POINTS["ease-in"]),
    "ease-out": cubicBezier(...BEZIER_POINTS["ease-out"]),
    "ease-in-out": cubicBezier(...BEZIER_POINTS["ease-in-out"]),
};

/** The names of the easings, for an error message. */
export const EASING_NAMES = Object.keys(EASINGS).join(", ");

/** The easing an animation has when none is given. */
export const DEFAULT_EASING: Easing = "ease-out";

/** The curve is sent as this many steps plus the start: 33 samples, as the Lua shell's `node:to` sends. */
export const CURVE_STEPS = 32;

/** The samples of `curve`: progress from 0 to 1 over `CURVE_STEPS` steps, which is what a timed write carries. */
export function curveSamples(curve: (t: number) => number): number[] {
    return Array.from({ length: CURVE_STEPS + 1 }, (_, i) => curve(i / CURVE_STEPS));
}

/** The curve of an easing name, or undefined for a name that is not one of `Easing`. */
export function easingCurve(name: unknown): ((t: number) => number) | undefined {
    return typeof name === "string" && Object.hasOwn(EASINGS, name) ? EASINGS[name as Easing] : undefined;
}

/** The milliseconds a transition takes when `duration` is not given. */
export const DEFAULT_DURATION_MS = 250;

/** The options a transition takes. Every field is optional; the default of each is stated on the field. */
export interface TransitionOptions {
    /** Milliseconds, 0 or more. Default 250. Zero commits the change at once (still as one transaction). */
    duration?: number;
    /** One of the CSS names of `Easing`. Default `"ease-out"`, as `Client.animate`'s. */
    easing?: Easing;
    /** Milliseconds, 0 or more, the compositor holds the start state before the animation begins. Default 0. */
    delay?: number;
    /**
     * Default true. False stages the transition with its tracks held at the start: `progress` scrubs them, `play()`
     * and `reverse()` run them, `pause()` holds them.
     */
    autoplay?: boolean;
}

/**
 * What `finished` and `onFinished` deliver: `cancelled` is true when `cancel()` or a newer transition took every
 * track.
 */
export interface TransitionResult {
    readonly cancelled: boolean;
}

/** Stops a callback before it runs. Calling it twice is harmless. */
export interface TransitionSubscription {
    unsubscribe(): void;
}

/** A value as it goes to the tree: a number, a flag, a text, a list of numbers or effect objects. */
type Value = unknown;

/**
 * The timed write a staged value is sent as, when it has a curve of its own (`Client.animate` inside a callback). A
 * transition adds the `delay` in milliseconds before it starts and the `tween` node whose play state drives it.
 */
interface Curve {
    duration: number;
    samples: number[];
    delay?: number;
    tween?: number;
}

interface Staged {
    node: number;
    key: string;
    value: Value;
    curve: Curve | null;
}

/**
 * The host functions staging stands in front of. Codes are the tree's: `unknown-node`, `read-only`, `invalid-value`
 * ...
 */
export interface StageBackend {
    get(node: number, key: string): Value;
    set(node: number, key: string, value: Value): string | undefined;
    /** A timed write: `delay` milliseconds before it starts (0 for none), joining the `tween` node (0 for none). */
    animate(
        node: number,
        key: string,
        value: Value,
        duration: number,
        samples: number[],
        delay?: number,
        tween?: number,
    ): string | undefined;
    /**
     * What `set` would answer, without queuing the write; with `timed` it also refuses a property the tree cannot
     * animate.
     */
    check(node: number, key: string, value: Value, timed: boolean): string | undefined;
}

/**
 * Makes the error a failed transition throws: `code` says why, `target` names the member or call, `message` the
 * reason.
 */
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

    animate(node: number, key: string, value: Value, duration: number, samples: number[]): string | undefined {
        if (this.#entries === null) {
            this.onWrite?.(node, key);
            return this.#backend.animate(node, key, value, duration, samples);
        }
        const code = this.#backend.check(node, key, value, true);
        if (code === undefined || code === null) {
            this.#keep({ node, key, value: copy(value), curve: { duration, samples } });
            return undefined;
        }
        return code;
    }

    /** Queues a write of the transaction's own, which is not a script's: it does not tell `onWrite`. */
    land(node: number, key: string, value: Value, curve: Curve | null): string | undefined {
        return curve === null
            ? this.#backend.set(node, key, value)
            : this.#backend.animate(
                  node,
                  key,
                  value,
                  curve.duration,
                  curve.samples,
                  curve.delay ?? 0,
                  curve.tween ?? 0,
              );
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
    /** The milliseconds an animation of `duration` takes: 0 while the seat asks for reduced motion. */
    motion(duration: number): number;
    /** Whether the tree takes a timed write of `key`. */
    animated(key: string): boolean;
    /** Whether a node has a place that is drawn: a size, and not hidden. A node that has none just appears. */
    placed(node: number): boolean;
    /** The name of a node in an error path: `Client#3`. */
    label(node: number): string;
    /**
     * Makes the compositor's `tween` node, the play state its tracks join (`progress`, `paused`, `reverse`), and
     * returns its id. Throws when the shell can make no more nodes.
     */
    makeTween(): number;
    /** Destroys a `tween` node made by `makeTween`. */
    dropTween(tween: number): void;
    /** Queues `task` after the calls already queued, under the listener budget. `site` names it in a report. */
    schedule(site: string, task: () => void): void;
    /** Takes what a callback can change that is not a tree write, and returns what puts it back. */
    snapshot(): () => void;
    /** Runs the style and layout passes once against the staged state, so their writes are staged too. */
    settle(): void;
    error: MakeError;
}

/** The compositor's word that a timed write ended (`track_ended`), as the host hands it over. */
export interface TrackEndedEvent {
    /** The node the write was to. */
    node: number;
    /** The property's tree key: `x`, `opacity`, `rotation`. */
    property: string;
    /** True when the track reached its target; false for every other end. */
    completed: boolean;
    /** The `tween` node the write joined; 0 for none. */
    tween: number;
}

const PLACE_KEYS = ["x", "y", "width", "height"];

let counter = 0;

/**
 * What a transition asks of the compositor's tracks: the engine's side of `progress`, `play`, `pause`, `reverse` and
 * `cancel`.
 */
interface TransitionControl {
    seek(progress: number): void;
    pause(paused: boolean): void;
    reverse(reverse: boolean): void;
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
    readonly id: number;
    /** The milliseconds the animation takes: the duration asked for, or 0 under reduced motion. */
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

    toString(): string {
        return `Transition#${this.id}`;
    }
}

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
            throw host.error("refused", "gnoblin.requestTransition", `the host refused ${refused.join(", ")}`);
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
