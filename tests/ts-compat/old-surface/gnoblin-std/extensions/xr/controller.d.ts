import type { ControllerButton } from "gnoblin-core/bindings/trigger_types";
import { Transform3D } from "gnoblin-core/math/transform";
import { Ray } from "gnoblin-core/math/vector";
import type { NodeNumber, XRWorld } from "gnoblin-std/extensions/xr/types";
/** What a shell may claim on a controller, in the order the node lists it: the names `controller.claimed` accepts. */
export declare const CLAIMABLE_INPUTS: readonly string[];
/**
 * The longest settle time a chord may have, in milliseconds. The program holds a press back from the window for 150 ms
 * (`xr_input::CLAIM_HOLD`) to wait for a claim; the claim is made after the settle time and needs about as long again
 * to travel shell, compositor and program, so a settle time past 60 would claim after the hold.
 */
export declare const MAX_CHORD_SETTLE_MS = 60;
/** One tracked hand: a `controller` node the XR program writes. Read it; it cannot be written. */
export declare class XRController {
    #private;
    /** @internal Made by `XRSession`. */
    constructor(world: XRWorld, node: NodeNumber);
    /** The controller node number as a string. */
    get id(): string;
    /** Whether the controller node still exists. */
    get alive(): boolean;
    /** `left` or `right`. */
    get hand(): string;
    /** Whether the controller is tracked. Untracked controllers cannot start a drag and end any active drag. */
    get tracked(): boolean;
    /** The pose of the aim ray in the room's placement frame, in room units. */
    get pose(): Transform3D;
    /** The aim ray: from the pose's position along its negative z, as the program points it. */
    get ray(): Ray;
    /** How far the trigger is pulled, 0 to 1. */
    get trigger(): number;
    /** How far the grip is squeezed, 0 to 1. */
    get grip(): number;
    /** The current thumbstick position, with each axis from -1 to 1. */
    get thumbstick(): {
        x: number;
        y: number;
    };
    /** The buttons that are down, as of the last frame. */
    get buttons(): ReadonlySet<ControllerButton>;
    /** Inputs the shell withholds from the window under the ray, in `CLAIMABLE_INPUTS` order. */
    get claimed(): readonly string[];
    /**
     * Claims `inputs` for `owner`. Claimed inputs stop reaching the window under the ray. A new claim replaces that
     * owner's previous claim; other claims remain. Invalid names throw `TypeError`; a missing controller or refused
     * scene write throws `SceneError`.
     */
    claim(owner: object, inputs: readonly string[]): void;
    /** Drops `owner`'s claim. Other claims remain. */
    release(owner: object): void;
    /** @internal Reads the values once and settles which buttons are down (a pull is down at 0.75, up at 0.25). */
    update(): void;
}
/** A list the XR module keeps current, which a listener can follow. */
export declare class XRList<T extends {
    readonly id: string;
}> implements Iterable<T> {
    #private;
    constructor(report: (site: string, error: unknown) => void);
    /** Number of items in the list. */
    get size(): number;
    /** Finds an item by id, or returns undefined. */
    get(id: string): T | undefined;
    /** Returns the item at a zero-based index, or undefined. */
    at(index: number): T | undefined;
    /** Calls `onAdded(item)` and `onRemoved(item)` on `listener` for later changes. */
    addListener(listener: object): void;
    /** Stops notifications to this listener. */
    removeListener(listener: object): void;
    /** @internal */
    add(item: T): void;
    /** @internal */
    remove(item: T): void;
    /** Iterates over a snapshot of the current items. */
    [Symbol.iterator](): Iterator<T>;
}
