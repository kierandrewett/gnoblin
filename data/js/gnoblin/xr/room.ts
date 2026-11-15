// `XRRoom`: the nodes the stock shell builds when the XR seat arrives: a panel for each client, the head panel, and
// the ring that places them.

import type { Camera3D } from "gnoblin/scene/camera";
import { Node } from "gnoblin/scene/node";
import type { NodeOptions } from "gnoblin/scene/node";
import { View } from "gnoblin/scene/view";
import { XRCamera3D } from "gnoblin/xr/camera";
import { SpatialLayout } from "gnoblin/xr/layout";
import { RING_DEFAULTS } from "gnoblin/xr/ring";
import type { XRClient } from "gnoblin/xr/types";

/** Initial layout and client filter for an XR room. */
export interface XRRoomOptions extends NodeOptions {
    layout?: SpatialLayout | null;
    /** Whether a client gets a panel. Secure clients never do. Default: every client. */
    accept?: (client: XRClient) => boolean;
}

/** Position and scale of a panel fixed to the head; distances are metres. */
export interface HeadPanelOptions {
    /** Metres right of, above and in front of the head. Default `0, 0, -1.2`. */
    x?: number;
    y?: number;
    z?: number;
    /** Millimetres of panel for one pixel of window. Default the ring's. */
    scale?: number;
}

/**
 * The node the XR program reads, with the head camera under it and one `View` for each client, placed by `layout`.
 * Panels follow the clients: a client that maps gets one, one that closes or turns secure loses it, a minimised
 * client's panel is hidden. A panel the user moved or resized is pinned: the layout leaves its place alone until
 * `unpin`. The room never changes a client.
 */
export class XRRoom extends Node {
    /** The camera the program writes the head pose onto: the one named `head`. */
    head: Camera3D;
    layout: SpatialLayout | null;
    accept: (client: XRClient) => boolean;
    readonly #views = new Map<XRClient, View>();
    /** The clients in the order they gained a View: the ring's order. */
    readonly #order: XRClient[] = [];
    readonly #pinned = new WeakSet<View>();
    readonly #headPanels = new Map<View, XRClient>();

    constructor(options: XRRoomOptions = {}) {
        const { layout, accept, ...rest } = options;
        const created = layout === undefined ? new SpatialLayout() : layout;
        super({ name: "room", y: -(created?.ring.eye ?? RING_DEFAULTS.eye), ...rest });
        this.layout = created;
        this.accept = accept ?? (() => true);
        this.head = new XRCamera3D();
        this.add(this.head);
    }

    /** Metres per room unit: the head camera's. */
    get unit(): number {
        return this.head.metres;
    }

    /** The panel that shows `client`, if it has one. */
    viewOf(client: XRClient): View | undefined {
        return this.#views.get(client);
    }

    /** Every panel of the room's ring, in the order the clients gained them. */
    get views(): View[] {
        return this.#order.flatMap((client) => this.#views.get(client) ?? []);
    }

    pin(view: View): void {
        this.#pinned.add(view);
    }

    unpin(view: View): void {
        this.#pinned.delete(view);
    }

    isPinned(view: View): boolean {
        return this.#pinned.has(view);
    }

    /** Shows `client` in a panel fixed to the head, in metres in front of the eyes. */
    pinToHead(client: XRClient, options: HeadPanelOptions = {}): View {
        const metres = this.unit;
        const scale = options.scale ?? this.layout?.ring.mmPerPx ?? RING_DEFAULTS.mmPerPx;
        const view = new View({
            content: client,
            x: (options.x ?? 0) / metres,
            y: (options.y ?? 0) / metres,
            z: (options.z ?? -1.2) / metres,
            scale: [scale, scale, scale],
            width: client.width,
            height: client.height,
        });
        this.head.add(view);
        this.#headPanels.set(view, client);
        return view;
    }

    #addView(client: XRClient): void {
        const view = new View({
            content: client,
            width: client.width,
            height: client.height,
            visible: !client.minimised,
        });
        view.input = "forward";
        this.add(view);
        this.#views.set(client, view);
        this.#order.push(client);
    }

    #dropView(client: XRClient): void {
        const view = this.#views.get(client);
        this.#views.delete(client);
        const at = this.#order.indexOf(client);
        if (at >= 0) this.#order.splice(at, 1);
        view?.destroy();
    }

    /**
     * Brings the panels up to date with `clients` and places them. `dialog` says whether a client is a dialog (drawn
     * inside its parent's panel). Writes only what changed.
     */
    reconcile(clients: readonly XRClient[], focus: XRClient | null, dialog: (client: XRClient) => boolean): void {
        const layout = this.layout;
        if (layout === null) {
            return;
        }
        const wanted = new Set<XRClient>();
        for (const client of clients) {
            if (client.alive && !client.secure && !dialog(client) && this.accept(client)) wanted.add(client);
        }
        for (const client of [...this.#views.keys()]) {
            const view = this.#views.get(client);
            if (!wanted.has(client) || view === undefined || !view.alive || view.orphaned) {
                this.#dropView(client);
            }
        }
        for (const client of clients) {
            if (wanted.has(client) && !this.#views.has(client)) this.#addView(client);
        }
        const hidden: View[] = [];
        const ring: { client: XRClient; view: View }[] = [];
        const pinned: View[] = [];
        for (const client of this.#order) {
            const view = this.#views.get(client);
            if (view === undefined) continue;
            view.width = client.width;
            view.height = client.height;
            view.visible = !client.minimised;
            if (client.minimised) hidden.push(view);
            else if (this.#pinned.has(view)) pinned.push(view);
            else ring.push({ client, view });
        }
        const { poses, order } = layout.arrange(
            ring.map(({ client, view }) => ({ id: client, width: view.width, height: view.height })),
            focus,
        );
        ring.forEach(({ view }, at) => {
            const pose = poses[at];
            view.x = pose.x;
            view.y = pose.y;
            view.z = pose.z;
            view.rotation = pose.rotation;
            view.scale = [pose.scale, pose.scale, pose.scale];
        });
        // Back to front: the program submits Views in tree order and nothing depth-tests, so the nearest is last.
        // A pinned panel goes where its distance puts it among the ring's.
        const back: View[] = order.map((at) => ring[at].view);
        const distance = (view: View): number => view.worldTransform.position.length();
        for (const view of pinned) {
            const own = distance(view);
            const at = back.findIndex((other) => distance(other) < own);
            back.splice(at < 0 ? back.length : at, 0, view);
        }
        const want = [...hidden, ...back];
        const mine = new Set<View>(want);
        const current = this.children.filter((child): child is View => child instanceof View && mine.has(child));
        if (current.length !== want.length || current.some((view, at) => view !== want[at])) {
            for (const view of want) this.add(view);
        }
        for (const [panel, client] of [...this.#headPanels]) {
            if (!client.alive || client.secure || !panel.alive || panel.orphaned) {
                this.#headPanels.delete(panel);
                panel.destroy();
                continue;
            }
            panel.width = client.width;
            panel.height = client.height;
            panel.visible = !client.minimised;
        }
    }
}
