import type { Client } from "gnoblin-core/host/client";
import type { CollectionState } from "gnoblin-core/host/collection_state";
import type { WorkspaceSnapshot } from "gnoblin-core/host/contract";
import type { Output } from "gnoblin-core/host/output";
import type { InputLayout } from "gnoblin-core/input/host";
import type { NodeId, Rect } from "gnoblin-core/types";
/** What changed on a window that a package may need to follow, besides the events the object model raises itself. */
export interface ClientChange {
    /** Who draws the window's frame changed. */
    decoration: boolean;
    /** The window moved by this much since the listeners were last told; null when it did not move. */
    moved: {
        dx: number;
        dy: number;
    } | null;
    /** The window became a dialog of another one, after the shell opened. */
    parent: boolean;
}
/** A pointer edge on a node that is not a window, in the node's own coordinates. */
export interface NodePointer {
    /** Horizontal position. */
    x: number;
    /** Vertical position. */
    y: number;
    /** Button number, or 0 when the edge is not for a button. */
    button: number;
}
/** The points at which a package takes part in the object model. */
export interface HostHooks {
    /**
     * A window appeared while the shell runs (not one that was there at boot), after its `onAdded` and `onReady` were
     * queued. A package that places windows places it here.
     */
    clientArrived(client: Client): void;
    /**
     * The collections, besides `gnoblin.clients`, that hold `client` as a root: its listeners hear the client's events
     * and its views are told of its changes. A package that groups clients (a workspace's `clients`) names them here.
     */
    clientRoots(client: Client): CollectionState[];
    /**
     * Whether `a` and `b` are shown together, which is when `raise` keeps a fullscreen window over the one it raised.
     * Every window is shown with every other by default.
     */
    sharesStack(a: Client, b: Client): boolean;
    /** What a window's changed properties mean beyond the events: called once per changed window, after them. */
    clientChange(client: Client, change: ClientChange): void;
    /**
     * A window was destroyed, after it left `gnoblin.clients` and the input contexts forgot it and before its
     * listeners hear `onClosed`. `parent` is the window it was a dialog of.
     */
    clientReleased(client: Client, parent: Client | null): void;
    /** An output joined `gnoblin.outputs` (at boot or later), before its `onAdded` is queued. */
    outputAdded(output: Output): void;
    /** A connected output was announced (it was added after boot), after its events were queued. */
    outputAnnounced(output: Output): void;
    /** An output was destroyed, after it left `gnoblin.outputs` and before its listeners hear `onDisconnected`. */
    outputLeft(output: Output): void;
    /**
     * An output's published properties changed, or an `apply` asked of it was answered. `detached` is its new state when it became a mirror or went off (true)
     * or came back (false), and null when that did not change.
     */
    outputChange(output: Output, detached: boolean | null): void;
    /** A node was destroyed: called for each, deepest first, before a window or output among them is released. */
    nodeDestroyed(node: NodeId): void;
    /** These nodes were destroyed: called once after every one of them was released. */
    nodesDestroyed(nodes: NodeId[]): void;
    /** The pointer crossed, moved over, pressed or released on a node that is not a window; the call to queue, or none. */
    nodePointer(node: NodeId, kind: string, point: NodePointer): (() => void) | undefined;
    /** The pointer edge `kind` (`enter`, `motion`, `leave`, `press` or `release`) reached a window. */
    pointerEdge(kind: string): void;
    /** The entry module finished loading and the shell is about to open: the last chance to supply what it left out. */
    finishLoad(): void;
    /** The shell opened (`flags.opened` is true now), before `onReady` is queued for the windows that were there. */
    opened(): void;
    /** Lays out what changed this frame; returns how many it laid out. */
    layout(): number;
    /** Runs the frame of the package's own toolkit, after the layout. */
    ui(): void;
    /** The frame's chords and gestures were delivered to the input contexts. */
    inputDelivered(): void;
    /** The generation is ending: release what the package holds (claims, nodes, tweens). */
    release(): void;
    /** What a reload keeps of the package's state across generations. */
    snapshot(): WorkspaceSnapshot;
    /** Restores what `snapshot` kept, into a fresh generation. */
    reconcile(previous: WorkspaceSnapshot): void;
    /** The workspaces `gnoblin.find("Workspace")` searches, in order, or null when the package has none. */
    workspaces(): object[] | null;
    /** The rectangle a dragged window stays in, or null when there is none (`InputWorld.workArea`). */
    workArea(client: Client | undefined): Rect | null;
    /** The layouts whose bindings go live in every input context while they are in use (`InputWorld.layouts`). */
    layouts(): InputLayout[];
}
/** The hooks in force. A caller reads one when it needs it and never keeps it. */
export declare const hooks: HostHooks;
/**
 * Takes the hooks a package implements: `feature` holds the ones it fills and the rest stay as they are. A hook that
 * another package took already throws, so two packages cannot silently replace one another's.
 */
export declare function installFeature(name: string, feature: Partial<HostHooks>): void;
