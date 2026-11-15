// `View`: a node that shows a client or a workspace, either as the client's own surface in the tree or as a mirror of
// the workspace's windows.

import { noteSignal, raiseSignal } from "gnoblin/properties";
import { host } from "gnoblin/scene/backend";
import { Camera3D } from "gnoblin/scene/camera";
import { Node } from "gnoblin/scene/node";
import type { NodeOptions } from "gnoblin/scene/node";
import { label, live, nodeIdOf } from "gnoblin/scene/registry";
import { finite } from "gnoblin/scene/vector";
import { WorkspaceMirror } from "gnoblin/scene_mirror";
import type { ContentFit } from "gnoblin/scene_mirror";

/** A width and a height in the View's units. */
export interface Size2D {
    width: number;
    height: number;
}

/** Initial content and size for a view; width and height are in client pixels. */
export interface ViewOptions extends NodeOptions {
    /** The client or workspace the View shows. */
    content?: object | null;
    /** The camera the content is drawn through. Null is the flat default. */
    camera?: Camera3D | null;
    /** The View's own rectangle: the size its picture is drawn at. Written before `width` and `height`. */
    size?: Size2D;
    contentFit?: ContentFit;
    width?: number;
    height?: number;
}

/** The workspace a View shows, and the mirror that draws it. */
interface Shown {
    content: object;
    mirror: WorkspaceMirror;
}

const shown = new WeakMap<object, Shown>();

const fits = new WeakMap<object, ContentFit>();

/**
 * A node that draws another node as a picture. `content` is a client or a workspace and is not reparented or moved:
 * a View is a second, separate way to see it. A client is the picture of its node. A workspace is not a tree node, so
 * the View keeps a picture made of one child View per client on it (`gnoblin/scene_mirror`), fitted to `size` by
 * `contentFit`; the children are not in `children`. The picture of a client is stretched to `size` by the compositor:
 * `contentFit` is kept and read back but only a workspace is fitted by it.
 */
export class View extends Node {
    constructor(options: ViewOptions = {}, cls = "View") {
        super(options, cls);
    }

    protected override assign(options: ViewOptions): void {
        super.assign(options);
        if (options.camera !== undefined) this.camera = options.camera;
        if (options.size !== undefined) this.size = options.size;
        if (options.width !== undefined) this.width = options.width;
        if (options.height !== undefined) this.height = options.height;
        if (options.contentFit !== undefined) this.contentFit = options.contentFit;
        if (options.content !== undefined) this.content = options.content;
    }

    /** The client or workspace the View shows, or null when it shows none or its client is gone. */
    get content(): object | null {
        const mirrored = shown.get(this);
        if (mirrored !== undefined) {
            return mirrored.content;
        }
        const id = this.read<number>("source");
        return id ? (host().contentOf(id) ?? null) : null;
    }
    set content(value: object | null) {
        if (value === null) {
            unshow(this);
            this.write("source", 0);
            return;
        }
        const id = typeof value === "object" ? host().nodeOf(value) : undefined;
        if (id !== undefined) {
            unshow(this);
            this.write("source", id);
            return;
        }
        const source = typeof value === "object" ? host().workspace(value) : undefined;
        if (source === undefined) {
            throw new TypeError(`${label(this)}.content: must be a client, a workspace or null`);
        }
        unshow(this);
        this.write("source", 0);
        const mirror = new WorkspaceMirror(
            host(),
            source,
            nodeIdOf(this),
            this,
            `${label(this)}.content`,
            () => fits.get(this) ?? "stretch",
        );
        shown.set(this, { content: value, mirror });
    }

    /** Whether the View's client is gone: the tree clears `source` when its node is destroyed. A workspace is never gone. */
    get orphaned(): boolean {
        return !shown.has(this) && this.read<number>("source") === 0;
    }

    get camera(): Camera3D | null {
        const id = this.read<number>("camera");
        const found = id ? live.get(id) : undefined;
        return found instanceof Camera3D ? found : null;
    }
    set camera(value: Camera3D | null) {
        this.write("camera", value === null ? 0 : nodeIdOf(value));
    }

    /** The width of the View's picture in pixels. */
    get width(): number {
        return this.read<number>("width");
    }
    set width(value: number) {
        this.write("width", finite(`${label(this)}.width`, value));
    }

    get height(): number {
        return this.read<number>("height");
    }
    set height(value: number) {
        this.write("height", finite(`${label(this)}.height`, value));
    }

    /** The View's rectangle as the specification names it: `{ width, height }`, the same numbers as `width` and `height`. */
    get size(): Size2D {
        return { width: this.width, height: this.height };
    }
    set size(value: Size2D) {
        if (typeof value !== "object" || value === null) {
            throw new TypeError(`${label(this)}.size: must be { width, height }`);
        }
        const width = finite(`${label(this)}.size.width`, value.width);
        const height = finite(`${label(this)}.size.height`, value.height);
        this.width = width;
        this.height = height;
    }

    /** How a picture is fitted to `size`: `contain`, `cover` or `stretch` (the default, which is what the compositor draws). */
    get contentFit(): ContentFit {
        noteSignal(this, "fit");
        return fits.get(this) ?? "stretch";
    }
    set contentFit(value: ContentFit) {
        if (value !== "contain" && value !== "cover" && value !== "stretch") {
            throw new TypeError(`${label(this)}.contentFit: must be "contain", "cover" or "stretch"`);
        }
        fits.set(this, value);
        raiseSignal(this, "fit");
    }

    override destroy(): void {
        unshow(this);
        super.destroy();
    }
}

/** Stops the mirror of a View that shows a workspace and removes its nodes. A function, not a method: a constructor option calls it before the class's private members exist. */
function unshow(view: View): void {
    shown.get(view)?.mirror.stop();
    shown.delete(view);
}
