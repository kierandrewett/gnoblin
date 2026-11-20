import type { Point, DragEvent, BindingClient } from "gnoblin-core/bindings/types";
import type { InputDevice } from "gnoblin-core/devices";
import type { ClaimTable } from "gnoblin-core/input/claims";
import type { InputHit, InputHost, InputWorld, SeatSettings } from "gnoblin-core/input/host";
import type { Registration } from "gnoblin-core/input/registration";
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
export declare class Hooks {
    #private;
    constructor(env: MatchEnv);
    /** Builds the event passed to a hook, including elapsed time and the current pointer position. */
    event(registration: Registration, client: BindingClient | null, position: Point, lastTime: number, extra?: Record<string, unknown>): {
        event: DragEvent;
        now: number;
    };
    /** Calls a hook, reporting a throw instead of raising it, so the next binding still has its turn. */
    call<R>(registration: Registration, hook: string, run: () => R): {
        ok: true;
        value: R;
    } | {
        ok: false;
    };
    /** Starts a binding's drag hook and returns its state, or null if it declines, throws or returns no state. */
    start(registration: Registration, event: DragEvent): object | null;
}
