// Controller state, input claims and the live list of controllers in an XR session.

import type { ControllerButton } from "gnoblin/bindings/trigger_types";
import { errorCodeOfRefusal } from "gnoblin/errors";
import { SceneError } from "gnoblin/scene/backend";
import { Transform3D } from "gnoblin/scene/transform";
import { Ray, Vector3 } from "gnoblin/scene/vector";
import type { NodeNumber, XRWorld } from "gnoblin/extensions/xr/types";

/** What a shell may claim on a controller, in the order the node lists it: the names `controller.claimed` accepts. */
export const CLAIMABLE_INPUTS: readonly string[] = ["trigger", "grip", "thumbstick", "primary", "secondary"];

/**
 * The longest settle time a chord may have, in milliseconds. The program holds a press back from the window for 150 ms
 * (`xr_input::CLAIM_HOLD`) to wait for a claim; the claim is made after the settle time and needs about as long again
 * to travel shell, compositor and program, so a settle time past 60 would claim after the hold.
 */
export const MAX_CHORD_SETTLE_MS = 60;

/** A trigger pulled to this much is down, and it stays down until it falls to `RELEASE`. */
const PRESS = 0.75;

const RELEASE = 0.25;

/** One tracked hand: a `controller` node the XR program writes. Read it; it cannot be written. */
export class XRController {
    readonly #world: XRWorld;
    readonly #node: NodeNumber;
    readonly #down = new Set<ControllerButton>();
    /** What each owner (a binding, a script) has claimed here; the node says the union. */
    readonly #claims = new Map<object, ReadonlySet<string>>();
    readonly #hand: string;

    /** @internal Made by `XRSession`. */
    constructor(world: XRWorld, node: NodeNumber) {
        this.#world = world;
        this.#node = node;
        // Read once: a hand does not change, and a controller that has gone still says which it was.
        this.#hand = (world.scene.get(node, "hand") as string | undefined) ?? "";
    }

    /** The controller node number as a string. */
    get id(): string {
        return String(this.#node);
    }

    /** Whether the controller node still exists. */
    get alive(): boolean {
        return this.#world.kind(this.#node) === "controller";
    }

    #number(key: string): number {
        return (this.#world.scene.get(this.#node, key) as number | undefined) ?? 0;
    }

    /** `left` or `right`. */
    get hand(): string {
        return this.#hand;
    }

    /** Whether the controller is tracked. Untracked controllers cannot start a drag and end any active drag. */
    get tracked(): boolean {
        return (this.#world.scene.get(this.#node, "tracked") as boolean | undefined) === true;
    }

    /** The pose of the aim ray in the room's placement frame, in room units. */
    get pose(): Transform3D {
        const scene = this.#world.scene;
        const rotation = (scene.get(this.#node, "rotation") as number[] | undefined) ?? [0, 0, 0];
        return new Transform3D(new Vector3(this.#number("x"), this.#number("y"), this.#number("z")), rotation);
    }

    /** The aim ray: from the pose's position along its negative z, as the program points it. */
    get ray(): Ray {
        const pose = this.pose;
        return new Ray(pose.position, pose.transformDirection(new Vector3(0, 0, -1)));
    }

    /** How far the trigger is pulled, 0 to 1. */
    get trigger(): number {
        return this.#number("trigger");
    }

    /** How far the grip is squeezed, 0 to 1. */
    get grip(): number {
        return this.#number("grip");
    }

    /** The current thumbstick position, with each axis from -1 to 1. */
    get thumbstick(): { x: number; y: number } {
        return { x: this.#number("thumbstick_x"), y: this.#number("thumbstick_y") };
    }

    /** The buttons that are down, as of the last frame. */
    get buttons(): ReadonlySet<ControllerButton> {
        return this.#down;
    }

    /** Inputs the shell withholds from the window under the ray, in `CLAIMABLE_INPUTS` order. */
    get claimed(): readonly string[] {
        return (this.#world.scene.get(this.#node, "claimed") as string[] | undefined) ?? [];
    }

    /**
     * Claims `inputs` for `owner`. Claimed inputs stop reaching the window under the ray. A new claim replaces that
     * owner's previous claim; other claims remain. Invalid names throw `TypeError`; a missing controller or refused
     * scene write throws `SceneError`.
     */
    claim(owner: object, inputs: readonly string[]): void {
        const target = `Controller#${this.id}.claimed`;
        if (typeof owner !== "object" || owner === null) {
            throw new TypeError(`${target}: a claim needs an owner object, got ${String(owner)}`);
        }
        if (!Array.isArray(inputs)) {
            throw new TypeError(`${target}: claim a list of input names such as ["grip"], got ${String(inputs)}`);
        }
        for (const name of inputs) {
            if (typeof name !== "string") {
                throw new TypeError(`${target}: an input name is a string, got ${String(name)}`);
            }
            if (name !== name.toLowerCase()) {
                throw new TypeError(
                    `${target}: input names are lowercase; write "${name.toLowerCase()}", not "${name}"`,
                );
            }
            if (!CLAIMABLE_INPUTS.includes(name)) {
                throw new TypeError(`${target}: "${name}" is not an input; use ${CLAIMABLE_INPUTS.join(", ")}`);
            }
        }
        if (!this.alive) {
            throw new SceneError("not_found", target, "the controller is gone");
        }
        const before = this.#claims.get(owner);
        if (inputs.length === 0) {
            this.#claims.delete(owner);
        } else {
            this.#claims.set(owner, new Set(inputs));
        }
        try {
            this.#flush(target);
        } catch (error) {
            if (before === undefined) this.#claims.delete(owner);
            else this.#claims.set(owner, before);
            throw error;
        }
    }

    /** Drops `owner`'s claim. Other claims remain. */
    release(owner: object): void {
        if (!this.#claims.delete(owner) || !this.alive) {
            return;
        }
        this.#flush(`Controller#${this.id}.claimed`);
    }

    /** Writes the union of the claims, once, when it differs from what the node says. */
    #flush(target: string): void {
        const union = CLAIMABLE_INPUTS.filter((name) => [...this.#claims.values()].some((held) => held.has(name)));
        const now = this.claimed;
        if (now.length === union.length && now.every((name, at) => name === union[at])) {
            return;
        }
        const code = this.#world.scene.set(this.#node, "claimed", union);
        if (code !== undefined) {
            throw new SceneError(errorCodeOfRefusal(code), target, `the shell refused the write (${code})`);
        }
    }

    /** @internal Reads the values once and settles which buttons are down (a pull is down at 0.75, up at 0.25). */
    update(): void {
        const scene = this.#world.scene;
        const flag = (key: string): boolean => (scene.get(this.#node, key) as boolean | undefined) === true;
        const analog: [ControllerButton, number][] = [
            ["Trigger", this.trigger],
            ["Grip", this.grip],
        ];
        for (const [button, value] of analog) {
            if (this.#down.has(button)) {
                if (value <= RELEASE) this.#down.delete(button);
            } else if (value >= PRESS) {
                this.#down.add(button);
            }
        }
        const digital: [ControllerButton, boolean][] = [
            ["Thumbstick", flag("thumbstick_click")],
            ["Primary", flag("primary")],
            ["Secondary", flag("secondary")],
        ];
        for (const [button, down] of digital) {
            if (down) this.#down.add(button);
            else this.#down.delete(button);
        }
    }
}

/** A list the XR module keeps current, which a listener can follow. */
export class XRList<T extends { readonly id: string }> implements Iterable<T> {
    readonly #items: T[] = [];
    readonly #listeners: object[] = [];
    readonly #report: (site: string, error: unknown) => void;

    constructor(report: (site: string, error: unknown) => void) {
        this.#report = report;
    }

    /** Number of items in the list. */
    get size(): number {
        return this.#items.length;
    }

    /** Finds an item by id, or returns undefined. */
    get(id: string): T | undefined {
        return this.#items.find((item) => item.id === id);
    }

    /** Returns the item at a zero-based index, or undefined. */
    at(index: number): T | undefined {
        return this.#items[index];
    }

    /** Calls `onAdded(item)` and `onRemoved(item)` on `listener` for later changes. */
    addListener(listener: object): void {
        if (typeof listener !== "object" || listener === null) {
            throw new TypeError("addListener: the listener must be an object");
        }
        this.#listeners.push(listener);
    }

    /** Stops notifications to this listener. */
    removeListener(listener: object): void {
        const at = this.#listeners.indexOf(listener);
        if (at >= 0) this.#listeners.splice(at, 1);
    }

    /** @internal */
    add(item: T): void {
        this.#items.push(item);
        this.#tell("onAdded", item);
    }

    /** @internal */
    remove(item: T): void {
        const at = this.#items.indexOf(item);
        if (at >= 0) {
            this.#items.splice(at, 1);
            this.#tell("onRemoved", item);
        }
    }

    #tell(method: "onAdded" | "onRemoved", item: T): void {
        for (const listener of [...this.#listeners]) {
            const handler = Reflect.get(listener, method);
            if (typeof handler === "function") {
                try {
                    handler.call(listener, item);
                } catch (error) {
                    this.#report(`XRList.${method}`, error);
                }
            }
        }
    }

    /** Iterates over a snapshot of the current items. */
    [Symbol.iterator](): Iterator<T> {
        return [...this.#items][Symbol.iterator]();
    }
}
