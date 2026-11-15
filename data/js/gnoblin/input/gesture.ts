// Touchpad gestures: begin, update, end and cancel of a claimed swipe or pinch map to the same four hooks as a drag. A
// gesture starts on the best candidate whose direction has come true, and `progress` says how far it has gone its way.

import type { GestureTriggerParts } from "gnoblin/bindings/trigger_types";
import type { BindingClient, DragEvent } from "gnoblin/bindings/types";
import type { Hooks, MatchEnv } from "gnoblin/input/hooks";
import type { InputGestureMessage } from "gnoblin/input/messages";
import type { ClaimEntry, Registration } from "gnoblin/input/registration";

/** Touchpad units of centroid movement before a swipe has a direction. */
const SWIPE_DECIDE = 12;
/** How far a scale must move from 1 before a pinch has a direction. */
const PINCH_DECIDE = 0.1;
/** Touchpad units for a whole swipe (`progress` 1), and the scale distance for a whole pinch. */
const SWIPE_TRAVEL = 400;
const PINCH_TRAVEL = 0.5;
/** The progress at which a function bound to a gesture fires: the Lua shell's 48 units and 0.12 scale. */
export const SWIPE_COMMIT = 48 / SWIPE_TRAVEL;
export const PINCH_COMMIT = 0.12 / PINCH_TRAVEL;

interface GestureSample {
    dx: number;
    dy: number;
    scale: number;
    rotation: number;
}

/** One touchpad gesture from its begin to its end or cancel. */
interface GestureRun {
    readonly claim: ClaimEntry;
    readonly fingers: number;
    readonly client: BindingClient | null;
    readonly candidates: readonly Registration[];
    latest: GestureSample;
    lastTime: number;
    started: Registration | null;
    state: object | null;
    /** No candidate can start for this gesture any more. */
    dead: boolean;
    done: boolean;
    scheduled: boolean;
}

function clamp01(value: number): number {
    return Math.max(0, Math.min(1, value));
}

/** The way a swipe has gone by its longer axis, or null while it has moved less than `SWIPE_DECIDE`. */
function swipeDirection(sample: GestureSample): "left" | "right" | "up" | "down" | null {
    if (Math.max(Math.abs(sample.dx), Math.abs(sample.dy)) < SWIPE_DECIDE) {
        return null;
    }
    if (Math.abs(sample.dx) >= Math.abs(sample.dy)) {
        return sample.dx > 0 ? "right" : "left";
    }
    return sample.dy > 0 ? "down" : "up";
}

function pinchDirection(sample: GestureSample): "in" | "out" | null {
    if (Math.abs(sample.scale - 1) < PINCH_DECIDE) {
        return null;
    }
    return sample.scale < 1 ? "in" : "out";
}

/** How far a gesture has gone for `trigger`, from 0 to 1. A directed gesture counts only its own way. */
function progressOf(trigger: GestureTriggerParts, sample: GestureSample): number {
    if (trigger.gesture === "pinch") {
        const spread =
            trigger.direction === "in"
                ? 1 - sample.scale
                : trigger.direction === "out"
                  ? sample.scale - 1
                  : Math.abs(sample.scale - 1);
        return clamp01(spread / PINCH_TRAVEL);
    }
    switch (trigger.direction) {
        case "left":
            return clamp01(-sample.dx / SWIPE_TRAVEL);
        case "right":
            return clamp01(sample.dx / SWIPE_TRAVEL);
        case "up":
            return clamp01(-sample.dy / SWIPE_TRAVEL);
        case "down":
            return clamp01(sample.dy / SWIPE_TRAVEL);
        default:
            return clamp01(Math.hypot(sample.dx, sample.dy) / SWIPE_TRAVEL);
    }
}

/** The gestures in flight on one context, by claim. */
export class GestureMatcher {
    readonly #env: MatchEnv;
    readonly #hooks: Hooks;
    readonly #gestures = new Map<number, GestureRun>();

    constructor(env: MatchEnv, hooks: Hooks) {
        this.#env = env;
        this.#hooks = hooks;
    }

    /** One step of a claimed gesture: `begin` picks the candidates, `update` and `end` run the hooks, `cancel` stops it. */
    message(claim: ClaimEntry, message: InputGestureMessage): void {
        const sample: GestureSample = {
            dx: message.dx,
            dy: message.dy,
            scale: message.scale,
            rotation: message.rotation,
        };
        const run = this.#gestures.get(claim.id);
        switch (message.phase) {
            case "begin": {
                if (run !== undefined) {
                    this.#cancelGesture(run);
                }
                const client = this.#env.hitTest(this.#env.pointer())?.client ?? null;
                const fresh: GestureRun = {
                    claim,
                    fingers: message.fingers,
                    client,
                    candidates: this.#env.claims.candidates(claim, client),
                    latest: sample,
                    lastTime: this.#env.host.now(),
                    started: null,
                    state: null,
                    dead: false,
                    done: false,
                    scheduled: false,
                };
                this.#gestures.set(claim.id, fresh);
                this.#env.world.schedule(`gesture ${claim.specKey}`, () => this.#decide(fresh));
                break;
            }
            case "update": {
                if (run === undefined) {
                    return;
                }
                // Latest wins: an update that is still waiting its turn is read at its turn, so the sample it
                // will see is this one, and a second call is not queued behind it.
                run.latest = sample;
                if (!run.scheduled) {
                    run.scheduled = true;
                    this.#env.world.schedule(`gesture ${claim.specKey}`, () => {
                        run.scheduled = false;
                        this.#gestureStep(run);
                    });
                }
                break;
            }
            case "end": {
                if (run === undefined) {
                    return;
                }
                run.latest = sample;
                this.#gestures.delete(claim.id);
                this.#env.world.schedule(`gesture end ${claim.specKey}`, () => this.#gestureEnd(run));
                break;
            }
            case "cancel": {
                if (run !== undefined) {
                    this.#cancelGesture(run);
                }
                break;
            }
        }
    }

    #gestureEvent(run: GestureRun, registration: Registration, phase: string): DragEvent {
        const trigger = registration.parsed as GestureTriggerParts;
        const sample = run.latest;
        const { event, now } = this.#hooks.event(registration, run.client, this.#env.pointer(), run.lastTime, {
            phase,
            fingers: run.fingers,
            dx: sample.dx,
            dy: sample.dy,
            scale: sample.scale,
            rotation: sample.rotation,
            progress: progressOf(trigger, sample),
        });
        run.lastTime = now;
        return event;
    }

    /** Starts the gesture on the best candidate whose direction has come true, once there is one. */
    #decide(run: GestureRun): void {
        if (run.started !== null || run.dead || run.done) {
            return;
        }
        for (const registration of run.candidates) {
            const parsed = registration.parsed as GestureTriggerParts;
            if (!this.#env.claims.has(registration)) {
                continue;
            }
            if (parsed.direction !== null) {
                const way = parsed.gesture === "swipe" ? swipeDirection(run.latest) : pinchDirection(run.latest);
                if (way === null) {
                    // Not decided yet. A better candidate may still match, so the rest wait.
                    return;
                }
                if (way !== parsed.direction) {
                    continue;
                }
            }
            const state = this.#hooks.start(registration, this.#gestureEvent(run, registration, "begin"));
            if (state !== null) {
                run.started = registration;
                run.state = state;
                return;
            }
        }
        // Everyone who could take it has declined, or can never match the way it has gone.
        const undecided = run.candidates.some((registration) => {
            const parsed = registration.parsed as GestureTriggerParts;
            return (
                parsed.direction !== null &&
                (parsed.gesture === "swipe" ? swipeDirection(run.latest) : pinchDirection(run.latest)) === null
            );
        });
        run.dead = !undecided;
    }

    #gestureStep(run: GestureRun): void {
        if (run.done) {
            return;
        }
        this.#decide(run);
        if (run.started !== null) {
            const registration = run.started;
            const state = run.state as object;
            const event = this.#gestureEvent(run, registration, "update");
            this.#hooks.call(registration, "onUpdate", () => registration.binding.onUpdate(event, state));
        }
    }

    #gestureEnd(run: GestureRun): void {
        if (run.done) {
            return;
        }
        this.#decide(run);
        run.done = true;
        if (run.started !== null) {
            const registration = run.started;
            const state = run.state as object;
            const event = this.#gestureEvent(run, registration, "end");
            this.#hooks.call(registration, "onEnd", () => registration.binding.onEnd(event, state));
        }
    }

    #cancelGesture(run: GestureRun): void {
        this.#gestures.delete(run.claim.id);
        if (run.done) {
            return;
        }
        run.done = true;
        const registration = run.started;
        if (registration === null) {
            return;
        }
        const state = run.state as object;
        this.#env.world.schedule(
            `${registration.binding.constructor.name} "${registration.binding.id}".onCancel`,
            () => {
                const event = this.#gestureEvent(run, registration, "cancel");
                this.#hooks.call(registration, "onCancel", () => registration.binding.onCancel(event, state));
            },
        );
    }

    /** A client died: the gesture on it is cancelled once. */
    cancelOn(client: BindingClient): void {
        for (const run of [...this.#gestures.values()]) {
            if (run.client === client) {
                this.#cancelGesture(run);
            }
        }
    }

    /** A binding was removed: the gesture it runs ends here, so nothing could finish it. */
    cancelBy(registration: Registration): void {
        for (const run of [...this.#gestures.values()]) {
            if (run.started === registration) {
                this.#cancelGesture(run);
            }
        }
    }

    /** Cancels every gesture in flight. */
    cancelAll(): void {
        for (const run of [...this.#gestures.values()]) {
            this.#cancelGesture(run);
        }
    }
}
