// Types the XR module reads from the shell's clients, seat and scene.

import type { SceneBackend } from "gnoblin-std/scene/backend";

/** A node of the compositor tree, by its number. */
export type NodeNumber = number;

/** What the XR module reads of a client. The shell's `Client` has all of it. */
export interface XRClient {
    /** Whether the client still exists. */
    readonly alive: boolean;
    /** Whether the client content is secure and must not be shown. */
    readonly secure: boolean;
    /** Whether the client is minimised. */
    readonly minimised: boolean;
    /** The application the window belongs to: what an `accept` function chooses a window by. */
    readonly appId: string;
    /** The window title. */
    readonly title: string;
    /** Client width, in pixels. */
    width: number;
    /** Client height, in pixels. */
    height: number;
}

/** The node and name of the shell's XR seat. */
export interface XRSeat {
    /** Scene node number for the seat. */
    readonly node: NodeNumber;
    /** Seat name, such as `xr`. */
    readonly name: string;
}

/** What the module needs of the shell. The host module implements it. */
export interface XRWorld {
    /** Scene backend used to read and write node properties. */
    readonly scene: SceneBackend;
    /** Node classes are returned in parent-before-child order. */
    nodes(cls: string): NodeNumber[];
    /** The lowercase class of a node, or undefined once it is gone. */
    kind(node: NodeNumber): string | undefined;
    /** Lists the shell's seats. */
    seats(): XRSeat[];
    /** The client that holds the keyboard focus of the main seat. */
    focusedClient(): XRClient | null;
    /** Lists clients currently known to the shell. */
    clients(): XRClient[];
    /** The lowercase class of the node a client sits under, or undefined when it has none. */
    parentKind(client: XRClient): string | undefined;
    /** The `InputContext` of a seat. */
    inputContext(seat: NodeNumber): object | null;
    /** Whether the session declared XR, so the module imports before the headset's session has started. */
    declared(): boolean;
    /** Milliseconds on a clock that only goes forward. */
    now(): number;
    /** Reports an error from an XR callback or update. */
    report(site: string, error: unknown): void;
}
