// What every compositor-made object (a `Client`, an `Output`) keeps privately: its node, whether it is alive, the
// values it froze when it died, and the last values its listeners were told. Reads go through the mirror, or the
// frozen copy once the node is gone; writes are checked here so the error names the member.

import { errorCodeOfRefusal, GnoblinError } from "gnoblin/errors";
import { host } from "gnoblin/host/bridge";
import type { Client } from "gnoblin/host/client";
import { ctx } from "gnoblin/host/context";
import type { Output } from "gnoblin/host/output";
import { rotationFrom, scaleFrom } from "gnoblin/spatial";
import type { Triple } from "gnoblin/spatial";
import type { HostValue, NodeId, Scalar } from "gnoblin/types";

/** A native failure code, as an error naming `target`. */
export function refuse(code: string, target: string, what: string): never {
    switch (code) {
        case "unknown-node":
            throw new GnoblinError("not_found", target, `the ${what} is gone`);
        case "read-only":
            throw new GnoblinError("read_only", target, "this property cannot be written");
        case "invalid-value":
            throw new TypeError(`${target}: the value has the wrong type for this property`);
        default:
            throw new GnoblinError(errorCodeOfRefusal(code), target, `the host refused the write (${code})`);
    }
}

/** What listeners were last told about a client, to raise an event only for a real change. */
export interface Seen {
    title: string;
    appId: string;
    decoration: string;
    focused: boolean;
    x: number;
    y: number;
    width: number;
    height: number;
    /** The node of the window this one is a dialog of, 0 for none. */
    parent: number;
}

/** State shared by native-backed object instances: identity, liveness, events, and last published values. */
export interface NodeState {
    node: NodeId;
    /** Which kind of object this is the state of. */
    kind: "client" | "output";
    alive: boolean;
    /** The last values of a destroyed node, which stay readable. */
    frozen: Record<string, HostValue | undefined> | null;
    listeners: object[];
    readied: boolean;
    seen: Seen | null;
    /** The script's own flag: a floating client is left out of its workspace's tiling. */
    floating: boolean;
}

/** Returns native state for a managed object. Throws when the value is not a shell-created object. */
export function stateOf(object: object): NodeState {
    const state = ctx.states.get(object);
    if (!state) {
        throw new TypeError("not an object the compositor made");
    }
    return state;
}

/** Reads a scalar property from the frozen snapshot or the current host mirror. */
export function read<T extends Scalar>(object: object, key: string): T {
    const state = stateOf(object);
    return (state.frozen ? state.frozen[key] : host.get(state.node, key)) as T;
}

/** Freezes the requested properties and marks an object dead before removal events are delivered. */
export function freeze(object: object, keys: string[]): void {
    const state = stateOf(object);
    const frozen: Record<string, HostValue | undefined> = {};
    for (const key of keys) {
        frozen[key] = host.get(state.node, key);
    }
    state.frozen = frozen;
    state.alive = false;
}

/** What the compositor last published for the properties that raise events. */
export function publishedValues(node: NodeId): Seen {
    return {
        title: host.published(node, "title") as string,
        appId: host.published(node, "app_id") as string,
        focused: host.published(node, "focused") as boolean,
        x: host.published(node, "x") as number,
        y: host.published(node, "y") as number,
        width: host.published(node, "width") as number,
        height: host.published(node, "height") as number,
        parent: host.published(node, "parent_window") as number,
        decoration: host.published(node, "decoration") as string,
    };
}

/** Calls a native client method. Throws for a dead client or host refusal. */
export function callMethod(client: Client, method: string): void {
    const state = stateOf(client);
    const target = `Client#${host.label(state.node)}.${method}()`;
    if (!state.alive) {
        throw new GnoblinError("not_found", target, "the client is gone");
    }
    const code = host.call(state.node, method);
    if (code != null) {
        refuse(code, target, "client");
    }
}

/** Whether `value` is a `Client` made by the compositor (a brand check, so a module below `Client` can ask). */
export function isClient(value: unknown): value is Client {
    return typeof value === "object" && value !== null && ctx.states.get(value)?.kind === "client";
}

/** Whether `value` is an `Output` made by the compositor. */
export function isOutput(value: unknown): value is Output {
    return typeof value === "object" && value !== null && ctx.states.get(value)?.kind === "output";
}

/** The kind of value a client property takes. */
type PropertyKind = "number" | "flag" | "vector";

/**
 * Writes one property of a client: the queued write, or the error that names the member. Throws for a dead client, a
 * value of the wrong shape, or a host refusal.
 */
export function writeProperty(client: Client, key: string, value: unknown, kind: PropertyKind): void {
    const state = stateOf(client);
    const target = `Client#${host.label(state.node)}.${key}`;
    if (!state.alive) {
        throw new GnoblinError("not_found", target, "the client is gone");
    }
    let sent: HostValue;
    if (kind === "number") {
        if (typeof value !== "number" || !Number.isFinite(value)) {
            throw new TypeError(`${target}: must be a finite number`);
        }
        sent = value;
    } else if (kind === "flag") {
        if (typeof value !== "boolean") {
            throw new TypeError(`${target}: must be true or false`);
        }
        sent = value;
    } else {
        let parts: Triple;
        try {
            parts = key === "scale" ? scaleFrom(target, value) : rotationFrom(target, value);
        } catch (error) {
            // A list of the wrong size or content keeps the message the shell always gave for it.
            throw Array.isArray(value) ? new TypeError(`${target}: must be three finite numbers`) : error;
        }
        sent = [parts[0], parts[1], parts[2]];
    }
    const code = host.set(state.node, key, sent);
    if (code != null) {
        refuse(code, target, "client");
    }
}
