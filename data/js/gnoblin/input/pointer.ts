// Matches claimed pointer chords. A press records its position and client; movement past the threshold starts a drag.
// Bindings are tried in precedence order. The active binding receives updates and then an end or cancel event.

import { PointerDragBinding } from "gnoblin/bindings/binding";
import type { BindingClient, DragEvent, Point } from "gnoblin/bindings/types";
import type { Hooks, MatchEnv } from "gnoblin/input/hooks";
import type { InputPointerMessage } from "gnoblin/input/messages";
import type { ClaimEntry, Registration } from "gnoblin/input/registration";

/** How far the pointer must go from the press before a drag starts, unless the binding says otherwise. */
const DRAG_THRESHOLD = 4;

/** One pointer chord from its start to its end or cancel. */
interface PointerPress {
    readonly claim: ClaimEntry;
    readonly origin: Point;
    readonly button: number;
    readonly client: BindingClient | null;
    readonly candidates: readonly Registration[];
    /** Where the compositor last said the pointer is. */
    at: Point;
    /** Where the last hook that ran saw it. */
    last: Point;
    lastTime: number;
    phase: "pressed" | "dragging" | "done";
    active: Registration | null;
    state: object | null;
    scheduled: boolean;
    /** The pointer went past the drag threshold and no binding took the drag: this press is not a click. */
    declined: boolean;
}

function distance(a: Point, b: Point): number {
    return Math.hypot(a.x - b.x, a.y - b.y);
}

/** The pointer drags in flight on one context, by claim. */
export class PointerMatcher {
    readonly #env: MatchEnv;
    readonly #hooks: Hooks;
    readonly #presses = new Map<number, PointerPress>();

    constructor(env: MatchEnv, hooks: Hooks) {
        this.#env = env;
        this.#hooks = hooks;
    }

    /** Records a press, schedules drag samples, or finishes the interaction for a chord message. */
    message(claim: ClaimEntry, message: InputPointerMessage): void {
        const at = { x: message.x, y: message.y };
        if (message.phase === "start") {
            this.#pointerPress(claim, at, message.button);
            return;
        }
        const press = this.#presses.get(claim.id);
        if (press === undefined) {
            return;
        }
        press.at = at;
        if (message.phase === "update") {
            this.#schedulePointer(press);
        } else if (message.phase === "end") {
            this.#pointerRelease(press);
        } else if (message.phase === "cancel") {
            this.#cancelPress(press);
        }
    }

    #pointerPress(claim: ClaimEntry, origin: Point, button: number): void {
        const previous = this.#presses.get(claim.id);
        if (previous !== undefined) {
            // A second start before the end: the first one's end was lost. Finish it as cancelled.
            this.#cancelPress(previous);
        }
        const client = this.#env.hitTest(origin)?.client ?? null;
        const press: PointerPress = {
            claim,
            origin,
            button,
            client,
            candidates: this.#env.claims.candidates(claim, client),
            at: origin,
            last: origin,
            lastTime: this.#env.host.now(),
            phase: "pressed",
            active: null,
            state: null,
            scheduled: false,
            declined: false,
        };
        this.#presses.set(claim.id, press);
    }

    /** Schedules a sample of the drag unless the pointer has not moved or one already waits (latest wins). */
    #schedulePointer(press: PointerPress): void {
        if (press.phase === "done" || press.scheduled || (press.at.x === press.last.x && press.at.y === press.last.y)) {
            return;
        }
        press.scheduled = true;
        this.#env.world.schedule(`pointer ${press.claim.specKey}`, () => {
            press.scheduled = false;
            this.#advance(press);
        });
    }

    /** Builds a drag event with the position, distance from the press, button and work area. */
    #pointerEvent(registration: Registration, press: PointerPress, position: Point): { event: DragEvent; now: number } {
        return this.#hooks.event(registration, press.client, position, press.lastTime, {
            x: position.x,
            y: position.y,
            dx: position.x - press.origin.x,
            dy: position.y - press.origin.y,
            button: press.button,
            workArea: this.#env.world.workArea(press.client ?? undefined) ?? undefined,
            dragConstraint: this.#env.host.dragConstraint(this.#env.seat()),
            edgeTiling: this.#env.settings().edgeTiling,
        });
    }

    #pointerRelease(press: PointerPress): void {
        this.#presses.delete(press.claim.id);
        this.#env.world.schedule(`pointer end ${press.claim.specKey}`, () => {
            if (press.phase === "done") {
                return;
            }
            // The end may arrive in the frame of the update that starts the drag: start it now.
            this.#advance(press);
            if (press.phase === "dragging" && press.active !== null) {
                const registration = press.active;
                const state = press.state as object;
                press.phase = "done";
                const { event } = this.#pointerEvent(registration, press, press.at);
                this.#hooks.call(registration, "onEnd", () => registration.binding.onEnd(event, state));
                return;
            }
            const wasPressed = press.phase === "pressed" && !press.declined;
            press.phase = "done";
            if (wasPressed) {
                this.#click(press);
            }
        });
    }

    /** An end with no drag: the best click binding that does not decline has it. */
    #click(press: PointerPress): void {
        for (const registration of press.candidates) {
            const parsed = registration.parsed;
            if (parsed.kind !== "pointer" || parsed.action !== "click" || !this.#env.claims.has(registration)) {
                continue;
            }
            if (typeof registration.binding.onPress !== "function") {
                continue;
            }
            const { event } = this.#pointerEvent(registration, press, press.at);
            const outcome = this.#hooks.call(registration, "onPress", () => registration.binding.onPress?.(event));
            if (!outcome.ok || outcome.value !== false) {
                return;
            }
        }
    }

    /** Takes the drag as far as the pointer's position allows: starts it past the threshold, else updates it. */
    #advance(press: PointerPress): void {
        if (press.phase === "done") {
            return;
        }
        const at = press.at;
        if (press.phase === "pressed" && !press.declined) {
            this.#tryStart(press, at);
        }
        if (press.phase === "dragging" && press.active !== null && (at.x !== press.last.x || at.y !== press.last.y)) {
            const registration = press.active;
            const state = press.state as object;
            const { event, now } = this.#pointerEvent(registration, press, at);
            press.last = at;
            press.lastTime = now;
            const outcome = this.#hooks.call(registration, "onUpdate", () =>
                registration.binding.onUpdate(event, state),
            );
            if (!outcome.ok) {
                // A binding that throws has lost the drag: it is told once, and no other drag is touched.
                this.#cancelPress(press);
            }
        }
    }

    #tryStart(press: PointerPress, at: Point): void {
        const moved = distance(press.origin, at);
        for (const registration of press.candidates) {
            const parsed = registration.parsed;
            if (parsed.kind !== "pointer" || parsed.action !== "drag" || !this.#env.claims.has(registration)) {
                continue;
            }
            const threshold =
                registration.binding instanceof PointerDragBinding ? registration.binding.threshold : DRAG_THRESHOLD;
            if (moved < threshold) {
                // A better binding is still waiting for the pointer to go far enough: the others wait too.
                return;
            }
            // The start event says where the press was, not where the pointer has got to, so a binding that
            // measures from `position` measures from the press. The update that follows has the current one.
            const { event } = this.#pointerEvent(registration, press, press.origin);
            const state = this.#hooks.start(registration, event);
            if (state !== null) {
                press.phase = "dragging";
                press.active = registration;
                press.state = state;
                press.last = press.origin;
                return;
            }
        }
        // Every candidate that was due has declined, so this is not a drag. The press stays one: its end is
        // still swallowed (the compositor withheld the press from the client), but a pointer that has gone this
        // far is not a click either.
        if (moved >= DRAG_THRESHOLD) {
            press.declined = true;
        }
    }

    #cancelPress(press: PointerPress): void {
        if (this.#presses.get(press.claim.id) === press) {
            this.#presses.delete(press.claim.id);
        }
        const was = press.phase;
        press.phase = "done";
        if (was !== "dragging" || press.active === null) {
            return;
        }
        const registration = press.active;
        const state = press.state as object;
        this.#env.world.schedule(
            `${registration.binding.constructor.name} "${registration.binding.id}".onCancel`,
            () => {
                const { event } = this.#pointerEvent(registration, press, press.at);
                this.#hooks.call(registration, "onCancel", () => registration.binding.onCancel(event, state));
            },
        );
    }

    /** A client died: a press on it is dropped, and a drag on it is cancelled. */
    cancelOn(client: BindingClient): void {
        for (const press of [...this.#presses.values()]) {
            if (press.client === client) {
                this.#cancelPress(press);
            }
        }
    }

    /** A binding was removed: the drag it runs ends here, so nothing could finish it. */
    cancelBy(registration: Registration): void {
        for (const press of [...this.#presses.values()]) {
            if (press.active === registration) {
                this.#cancelPress(press);
            }
        }
    }

    /** Cancels every drag in flight. */
    cancelAll(): void {
        for (const press of [...this.#presses.values()]) {
            this.#cancelPress(press);
        }
    }
}
