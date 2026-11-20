import { XRExtension } from "gnoblin-std/extensions/xr/session";
import type { XROptions } from "gnoblin-std/extensions/xr/session";
import type { XRWorld } from "gnoblin-std/extensions/xr/types";
/**
 * The shell's side of XR, one per shell. It follows the seat: while the program has a session it builds the room, keeps
 * the panels current and runs the drags; when the session ends it takes all of it down.
 */
export declare class XRRuntime {
    #private;
    /** Options for the stock room and its client filter. */
    options: XROptions;
    constructor(world: XRWorld);
    /** The live XR extension, or null when no session exists. */
    get extension(): XRExtension | null;
    /** Receives session start and end notifications. */
    addListener(listener: object): void;
    /** Stops receiving session lifecycle notifications. */
    removeListener(listener: object): void;
    /** Whether `gnoblin-std/extensions/xr` may be imported now: the session declared XR, or the program has a session. */
    available(): boolean;
    /** Runs once a frame: follows the seat, then keeps the room and the drags current. */
    frame(): void;
    /**
     * Cancels each active drag once and ends the session's hold. The shell calls this when a generation ends.
     */
    release(): void;
}
