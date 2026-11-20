import type { Client } from "gnoblin-core/host/client";
import type { Output } from "gnoblin-core/host/output";
import type { HostValue, NodeId, Scalar } from "gnoblin-core/types";
/** A native failure code, as an error naming `target`. */
export declare function refuse(code: string, target: string, what: string): never;
/** What listeners were last told about a client, to raise an event only for a real change. */
export interface Seen {
    /** Last title reported by the compositor. */
    title: string;
    /** Last application id reported by the compositor. */
    appId: string;
    /** Last decoration mode reported by the compositor. */
    decoration: string;
    /** Whether the window was last reported as focused. */
    focused: boolean;
    /** Last horizontal position in logical pixels. */
    x: number;
    /** Last vertical position in logical pixels. */
    y: number;
    /** Last width in logical pixels. */
    width: number;
    /** Last height in logical pixels. */
    height: number;
    /** The node of the window this one is a dialog of, 0 for none. */
    parent: number;
}
/** State shared by native-backed object instances: identity, liveness, events, and last published values. */
export interface NodeState {
    /** Native node identity. */
    node: NodeId;
    /** Which kind of object this is the state of. */
    kind: "client" | "output";
    /** Whether the native object still exists. */
    alive: boolean;
    /** The last values of a destroyed node, which stay readable. */
    frozen: Record<string, HostValue | undefined> | null;
    /** Objects listening for changes to this native object. */
    listeners: object[];
    /** Whether the initial ready event has been delivered. */
    readied: boolean;
    /** Last client values reported to listeners, or null before they are recorded. */
    seen: Seen | null;
}
/** Returns native state for a managed object. Throws when the value is not a shell-created object. */
export declare function stateOf(object: object): NodeState;
/** Reads a scalar property from the frozen snapshot or the current host mirror. */
export declare function read<T extends Scalar>(object: object, key: string): T;
/** Freezes the requested properties and marks an object dead before removal events are delivered. */
export declare function freeze(object: object, keys: string[]): void;
/** What the compositor last published for the properties that raise events. */
export declare function publishedValues(node: NodeId): Seen;
/** Calls a native client method. Throws for a dead client or host refusal. */
export declare function callMethod(client: Client, method: string): void;
/** Whether `value` is a `Client` made by the compositor (a brand check, so a module below `Client` can ask). */
export declare function isClient(value: unknown): value is Client;
/** Whether `value` is an `Output` made by the compositor. */
export declare function isOutput(value: unknown): value is Output;
/** The kind of value a client property takes. */
type PropertyKind = "number" | "flag" | "vector" | "text";
/**
 * Writes one property of a client: the queued write, or the error that names the member. Throws for a dead client, a
 * value of the wrong shape, or a host refusal.
 */
export declare function writeProperty(client: Client, key: string, value: unknown, kind: PropertyKind): void;
export {};
