import type { Camera3D } from "gnoblin-std/scene/camera";
import { Node } from "gnoblin-std/scene/node";
import type { NodeOptions } from "gnoblin-std/scene/node";
import { View } from "gnoblin-std/scene/view";
import { SpatialLayout } from "gnoblin-std/extensions/xr/layout";
import type { XRClient } from "gnoblin-std/extensions/xr/types";
/** Initial layout and client filter for an XR room. */
export interface XRRoomOptions extends NodeOptions {
    /** The room layout, or null to leave panels unarranged. */
    layout?: SpatialLayout | null;
    /** Whether a client gets a panel. Secure clients never do. Default: every client. */
    accept?: (client: XRClient) => boolean;
}
/** Position and scale of a panel fixed to the head; distances are metres. */
export interface HeadPanelOptions {
    /** Metres right of, above and in front of the head. Default `0, 0, -1.2`. */
    x?: number;
    /** Height above the head, in metres. */
    y?: number;
    /** Distance in front of the head, in metres; negative z is forward. */
    z?: number;
    /** Millimetres of panel for one pixel of window. Default the ring's. */
    scale?: number;
}
/**
 * The room node contains the head camera and one `View` for each accepted client. Panels follow clients: closed or
 * secure clients lose their panels, and minimised clients are hidden. A moved or resized panel stays pinned until
 * `unpin`. The room never changes a client.
 */
export declare class XRRoom extends Node {
    #private;
    /** The camera the program writes the head pose onto: the one named `head`. */
    head: Camera3D;
    /** Ring arrangement, or null to leave the room's panel positions unchanged. */
    layout: SpatialLayout | null;
    /** Selects clients to show; secure clients are excluded regardless of this predicate. */
    accept: (client: XRClient) => boolean;
    constructor(options?: XRRoomOptions);
    /** Metres per room unit: the head camera's. */
    get unit(): number;
    /** The panel that shows `client`, if it has one. */
    viewOf(client: XRClient): View | undefined;
    /** Every panel of the room's ring, in the order the clients gained them. */
    get views(): View[];
    /** Keeps a panel at its current position during layout updates. */
    pin(view: View): void;
    /** Lets layout updates place the panel again. */
    unpin(view: View): void;
    /** Whether layout updates leave this panel at its current position. */
    isPinned(view: View): boolean;
    /** Shows `client` in a panel fixed to the head, in metres in front of the eyes. */
    pinToHead(client: XRClient, options?: HeadPanelOptions): View;
    /**
     * Brings the panels up to date with `clients` and places them. `dialog` says whether a client is a dialog (drawn
     * inside its parent's panel). Writes only what changed.
     */
    reconcile(clients: readonly XRClient[], focus: XRClient | null, dialog: (client: XRClient) => boolean): void;
}
