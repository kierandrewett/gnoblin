// `Outlet`: a named place in a window frame for script-provided nodes.
// It is a `Box`, made when first read and placed from the window's size and frame inset.

import type { FrameClient } from "gnoblin-std/frame/client";
import type { FrameInset } from "gnoblin-core/inset";
import { Node } from "gnoblin-std/scene/node";
import { mountUnder } from "gnoblin-std/scene/registry";
import { Box } from "gnoblin-std/ui/box";
import type { BoxOptions } from "gnoblin-std/ui/box";
import type { Canvas } from "gnoblin-std/ui/canvas";
import type { Rect } from "gnoblin-core/types";

/** The window's content size and the room its frame takes around it: what an outlet is placed from. */
export interface FrameGeometry {
    /** The window content width in logical pixels. */
    readonly width: number;
    /** The window content height in logical pixels. */
    readonly height: number;
    /** The room the frame takes around the content, in logical pixels. */
    readonly inset: FrameInset;
}

/** What an outlet asks of the frame that made it. */
export interface OutletOwner {
    /** The window whose frame owns the outlet. */
    readonly client: FrameClient;
    /** The content's size and the inset now, which a placed outlet takes its rectangle from. */
    geometry(): FrameGeometry;
}

/**
 * A place in a frame. `add` takes a canvas (a `Box`, `Text`, `Icon` or a widget of your own) or a scene node (a
 * `View`) and puts it under the outlet; `clear` destroys what was added. Nodes in an outlet are positioned as the
 * outlet's `Box` positions them: a row or a column stretches them across and gives a node with `grow` the room that
 * is left.
 *
 * An outlet is hidden, and takes no room, while the window is fullscreen, while `hidden` is true and while `hideWhen`
 * returns true for the window. It follows the window once a frame: a script puts anything that reads the window into
 * the nodes it adds (their `refresh`, or `properties.bind`) or into `onRefresh`, and they are up to date without the
 * script running again.
 */
export class Outlet extends Box {
    /** The outlet's name in its frame: `top`, `top.start`, `overlay`. */
    readonly name: string;
    /** The frame that created this outlet. */
    readonly owner: OutletOwner;
    /** Hides the outlet and takes back its room while true. */
    hidden = false;
    /** Hides the outlet and takes back its room while it returns true for the window: `(c) => c.maximised`. */
    hideWhen: ((client: FrameClient) => boolean) | null = null;
    /** Runs once a frame before the outlet is placed: read the window into the outlet's own fields here. */
    onRefresh: (() => void) | null = null;
    /** Told of each node added: an edge learns from it how thick its nodes are. */
    onAdded: ((child: Canvas | Node) => void) | null = null;
    /** The scene nodes added, which are not canvases and so are not among `children`. */
    readonly #scene: Node[] = [];

    constructor(owner: OutletOwner, name: string, options: BoxOptions = {}) {
        super(options);
        this.owner = owner;
        this.name = name;
    }

    /** Whether the outlet shows and takes its room now. */
    get shown(): boolean {
        return !this.owner.client.fullscreen && this.enabled;
    }

    /** Whether the outlet would show and take its room if the window were not fullscreen. */
    get enabled(): boolean {
        return !this.hidden && !(this.hideWhen?.(this.owner.client) ?? false);
    }

    /** Where the outlet goes, in the window's coordinates, or null when its parent's layout places it. */
    protected place(): Rect | null {
        return null;
    }

    /** Puts `child` last under the outlet. A canvas goes where `Box.add` puts it; a scene node is mounted under it. */
    override add(child: Canvas | Node): this {
        if (child instanceof Node) {
            mountUnder(child, this.node);
            this.#scene.splice(0, this.#scene.length, ...this.#scene.filter((node) => node.alive), child);
        } else {
            super.add(child);
        }
        this.onAdded?.(child);
        return this;
    }

    /** Destroys every node that was added and leaves the outlet empty. */
    clear(): void {
        for (const child of [...this.children, ...this.#scene.splice(0)]) {
            child.destroy();
        }
    }

    /** The room the nodes under the outlet take along its row or column, with the padding and the gaps. */
    protected override naturalWidth(): number {
        return this.#natural("width", this.direction === "row");
    }

    protected override naturalHeight(): number {
        return this.#natural("height", this.direction === "column");
    }

    #natural(axis: "width" | "height", along: boolean): number {
        const sizes = this.children.map((child) => child[axis]);
        const total = along
            ? sizes.reduce((sum, size) => sum + size, 0) + this.gap * Math.max(sizes.length - 1, 0)
            : Math.max(0, ...sizes);
        return total + 2 * this.padding;
    }

    /** Updates the outlet's placement, visibility and refresh callback. */
    override refresh(): void {
        const place = this.place();
        if (place !== null) {
            this.x = place.x;
            this.y = place.y;
            this.width = place.width;
            this.height = place.height;
        }
        this.visible = this.shown;
        this.onRefresh?.();
    }
}
