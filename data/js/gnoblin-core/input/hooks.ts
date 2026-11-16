// Builds binding events and runs hooks for the key, pointer and gesture matchers of one context.

import type { Point, DragEvent, BindingClient } from "gnoblin-core/bindings/types";
import type { InputDevice } from "gnoblin-core/devices";
import type { ClaimTable } from "gnoblin-core/input/claims";
import type { InputHit, InputHost, InputWorld, SeatSettings } from "gnoblin-core/input/host";
import type { Registration } from "gnoblin-core/input/registration";

/** `deltaTime` never exceeds this, so a simulation that integrates it is not thrown by a stall. */
const MAX_DELTA_TIME = 0.1;

/** Services a matcher reads from its context while processing input. */
export interface MatchEnv {
    /** The context, which a hook's event names. */
    readonly context: object;
    /** The host that provides seat state and claim operations. */
    readonly host: InputHost;
    /** Client, layout and scheduling services used by the matchers. */
    readonly world: InputWorld;
    /** Looks up claims held by this context. */
    readonly claims: ClaimTable;
    /** The seat the context makes claims on. */
    seat(): number;
    /** Returns the current window-management settings for the seat. */
    settings(): SeatSettings;
    /** The pointer in the compositor's logical space. */
    pointer(): Point;
    /** Finds the topmost client at a logical pointer position, or null. */
    hitTest(position: Point): InputHit | null;
    /** Returns the client with keyboard focus, or null. */
    focusedClient(): BindingClient | null;
    /** The device this context holds that reports `name`, or null. */
    deviceNamed(name: string): InputDevice | null;
}

/** The hook runner of one context's matchers. */
export class Hooks {
    readonly #env: MatchEnv;

    constructor(env: MatchEnv) {
        this.#env = env;
    }

    /** Builds the event passed to a hook, including elapsed time and the current pointer position. */
    event(
        registration: Registration,
        client: BindingClient | null,
        position: Point,
        lastTime: number,
        extra: Record<string, unknown> = {},
    ): { event: DragEvent; now: number } {
        const now = this.#env.host.now();
        const deviceName = registration.device === "" ? null : registration.device;
        const device = deviceName === null ? null : this.#env.deviceNamed(deviceName);
        const base: Record<string, unknown> = {
            context: this.#env.context,
            device,
            trigger: registration.parsed.text,
            timestamp: now,
            deltaTime: Math.min(MAX_DELTA_TIME, Math.max(0, (now - lastTime) / 1000)),
            modifiers: new Set<string>(registration.parsed.modifiers),
            position,
            ...extra,
        };
        if (client !== null) {
            const b = client.bounds;
            base.client = client;
            base.localPosition = { x: position.x - b.x, y: position.y - b.y };
        }
        if (registration.layout !== null) {
            base.layout = registration.layout;
        }
        return { event: base as unknown as DragEvent, now };
    }

    /** Calls a hook, reporting a throw instead of raising it, so the next binding still has its turn. */
    call<R>(registration: Registration, hook: string, run: () => R): { ok: true; value: R } | { ok: false } {
        try {
            return { ok: true, value: run() };
        } catch (error) {
            this.#env.world.report(
                `${registration.binding.constructor.name} "${registration.binding.id}".${hook}`,
                error,
            );
            return { ok: false };
        }
    }

    /** Starts a binding's drag hook and returns its state, or null if it declines, throws or returns no state. */
    start(registration: Registration, event: DragEvent): object | null {
        const outcome = this.call(registration, "onStart", () => registration.binding.onStart(event));
        if (!outcome.ok || outcome.value === false) {
            return null;
        }
        if (typeof outcome.value !== "object" || outcome.value === null) {
            this.#env.world.report(
                `${registration.binding.constructor.name} "${registration.binding.id}".onStart`,
                new TypeError("onStart must return a state object, or false to decline"),
            );
            return null;
        }
        return outcome.value;
    }
}
