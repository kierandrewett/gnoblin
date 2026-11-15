// The listener classes a script extends to observe clients, collections and outputs. Each method is optional; a
// listener is told of an event only when it defines the handler.

import type { Client } from "gnoblin/host/client";
import type { Collection } from "gnoblin/host/collections";
import type { Output } from "gnoblin/host/output";
import type { Workspace } from "gnoblin/host/workspace";
import type { Rect } from "gnoblin/types";
import type { ClientRequest } from "gnoblin/window_state";

/** A pointer edge on a window: where, in the window's own coordinates, and which button (0 for the others). */
export interface ClientPointerEvent {
    readonly x: number;
    readonly y: number;
    readonly button: number;
}

/** Observes the events of a client: attach it to one client, or to a collection for every member. */
export class ClientListener {
    onTitleChanged?(client: Client, title: string): void;
    onAppIdChanged?(client: Client, appId: string): void;
    /** The compositor changed who draws the window's frame: `client`, `server` or `none` (see `Client.decoration`). */
    onDecorationChanged?(client: Client, decoration: string): void;
    /** The pointer crossed into the window. `event` is in the window's own coordinates. */
    onPointerEnter?(client: Client, event: ClientPointerEvent): void;
    onPointerMotion?(client: Client, event: ClientPointerEvent): void;
    onPointerLeave?(client: Client, event: ClientPointerEvent): void;
    /** A button went down on the window. The window's client hears it too. */
    onPointerPress?(client: Client, event: ClientPointerEvent): void;
    onPointerRelease?(client: Client, event: ClientPointerEvent): void;
    onFocusChanged?(client: Client, focused: boolean): void;
    onGeometryChanged?(client: Client, geometry: Rect): void;
    onClosed?(client: Client): void;
    onReady?(client: Client): void;
    /**
     * The client asked for a change of its window (`request.kind`): maximize, fullscreen, minimize, activate, move and
     * so on. Call `request.handle()` to answer it yourself; the first listener to do so wins and the built-in answer
     * (`windowState`) does not run. The listeners of the client itself are asked before those of the collections.
     */
    onRequest?(client: Client, request: ClientRequest<Client>): void;
}

/** Observes membership: an object joining or leaving a collection. */
export class CollectionListener {
    onAdded?(member: Client | Workspace | Output, collection: Collection<Client | Workspace | Output>): void;
    onRemoved?(member: Client | Workspace | Output, collection: Collection<Client | Workspace | Output>): void;
}

/**
 * Observes the outputs: attach it to one output, or to `gnoblin.outputs` for all of them. A listener on the
 * collection hears `onConnected` for an output that is plugged in while the shell runs, not for the ones that were
 * there when it started.
 */
export class OutputListener {
    /** An `apply` asked of this output was answered: `error` is null when it took effect, else why it did not. */
    onApplied?(output: Output, error: string | null): void;
    /** The output appeared: it was plugged in, or made by the compositor. */
    onConnected?(output: Output): void;
    /** The output is gone. Its last values stay readable; `connected` is false. */
    onDisconnected?(output: Output): void;
    /** The output's mode (`WxH@Hz`) changed. */
    onModeChanged?(output: Output, mode: string): void;
    onScaleChanged?(output: Output, scale: number): void;
    /** The output's place or size in the compositor's coordinates changed. */
    onGeometryChanged?(output: Output, geometry: Rect): void;
}

/** Checks listener input before it is attached. Throws for non-object values. */
export function assertListener(listener: unknown, target: string): asserts listener is object {
    if (typeof listener !== "object" || listener === null) {
        throw new TypeError(`${target}: a listener must be an object, such as an instance of ClientListener`);
    }
}
