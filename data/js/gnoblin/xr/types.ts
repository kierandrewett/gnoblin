// What the XR module reads of the shell: a client, the XR seat and the world (`XRWorld`) the host implements. Types
// only, so the room, the controllers and the tests run over a fake world.

import type { SceneBackend } from "gnoblin/scene/backend";

/** A node of the compositor tree, by its number. */
export type NodeNumber = number;

/** What the XR module reads of a client. The shell's `Client` has all of it. */
export interface XRClient {
    readonly alive: boolean;
    readonly secure: boolean;
    readonly minimised: boolean;
    /** The application the window belongs to: what an `accept` function chooses a window by. */
    readonly appId: string;
    readonly title: string;
    width: number;
    height: number;
}

/** The node and name of the shell's XR seat. */
export interface XRSeat {
    readonly node: NodeNumber;
    readonly name: string;
}

/** What the module needs of the shell. The host module implements it. */
export interface XRWorld {
    readonly scene: SceneBackend;
    /** Every node of a tree class (`Controller`, `Node`), parents before children. */
    nodes(cls: string): NodeNumber[];
    /** The lowercase class of a node, or undefined once it is gone. */
    kind(node: NodeNumber): string | undefined;
    seats(): XRSeat[];
    /** The client that holds the keyboard focus of the main seat. */
    focusedClient(): XRClient | null;
    clients(): XRClient[];
    /** The lowercase class of the node a client sits under, or undefined when it has none. */
    parentKind(client: XRClient): string | undefined;
    /** The `InputContext` of a seat. */
    inputContext(seat: NodeNumber): object | null;
    /** Whether the session declared XR, so the module imports before the headset's session has started. */
    declared(): boolean;
    /** Milliseconds on a clock that only goes forward. */
    now(): number;
    report(site: string, error: unknown): void;
}
