// How a matcher runs a binding's hook: the event it hands over, the call that reports a throw instead of raising it, and the
// start that asks a binding to take a drag. The key, pointer and gesture matchers share them, and read the context
// they act for only through `MatchEnv`.

import type { Point, DragEvent, BindingClient } from "gnoblin/bindings/types";
import type { InputDevice } from "gnoblin/devices";
import type { ClaimTable } from "gnoblin/input/claims";
import type { InputHit, InputHost, InputWorld, SeatSettings } from "gnoblin/input/host";
import type { Registration } from "gnoblin/input/registration";

/** `deltaTime` never exceeds this, so a simulation that integrates it is not thrown by a stall. */
const MAX_DELTA_TIME = 0.1;

/** What a matcher reads of the context it works for. The context builds one and hands it to each matcher. */
export interface MatchEnv {
    /** The context, which a hook's event names. */
    readonly context: object;
    readonly host: InputHost;
    readonly world: InputWorld;
    readonly claims: ClaimTable;
    /** The seat the context makes claims on. */
    seat(): number;
    settings(): SeatSettings;
    /** The pointer in the compositor's logical space. */
    pointer(): Point;
    hitTest(position: Point): InputHit | null;
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

    /** The event of a hook: when it runs, how long since the last one, where the pointer is, and what the registration knows of the client, device and layout. */
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
