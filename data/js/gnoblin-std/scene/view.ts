// A View draws a client or a workspace in the scene tree without moving the content it shows.

import { noteSignal, raiseSignal } from "gnoblin-core/properties";
import { host } from "gnoblin-std/scene/backend";
import { Camera3D } from "gnoblin-std/scene/camera";
import { Node } from "gnoblin-std/scene/node";
import type { NodeOptions } from "gnoblin-std/scene/node";
import { label, live, nodeIdOf } from "gnoblin-std/scene/registry";
import { finite } from "gnoblin-core/math/vector";
import { WorkspaceMirror } from "gnoblin-std/scene_mirror";
import type { ContentFit } from "gnoblin-std/scene_mirror";

/** The width and height of a rectangle in a View's units. */
export interface Size2D {
    /** Width in the View's coordinate units. */
    width: number;
    /** Height in the View's coordinate units. */
    height: number;
}

/** Initial content and dimensions for a view. Width and height are in client pixels. */
export interface ViewOptions extends NodeOptions {
    /** The client or workspace the View shows. */
    content?: object | null;
    /** The camera the content is drawn through. Null is the flat default. */
    camera?: Camera3D | null;
    /** The View's own rectangle: the size its picture is drawn at. Written before `width` and `height`. */
    size?: Size2D;
    /** Fitting rule used for workspace content; defaults to `stretch`. */
    contentFit?: ContentFit;
    /** Picture width in client pixels. */
    width?: number;
    /** Picture height in client pixels. */
    height?: number;
}

/** Content and mirror used when a View shows a workspace. */
interface Shown {
    content: object;
    mirror: WorkspaceMirror;
}

const shown = new WeakMap<object, Shown>();

const fits = new WeakMap<object, ContentFit>();

/**
 * A node that draws a client or workspace as a picture without moving it. A client is shown directly. A workspace is
 * shown through a mirror made of child Views, fitted to `size` by `contentFit`; those mirror children are not in
 * `children`. The compositor stretches a client's picture to `size`; `contentFit` only affects workspace pictures.
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
    /** Replaces the displayed client or workspace; null clears the picture. */
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

    /** Whether the client content is gone; workspace content is not reported as orphaned. */
    get orphaned(): boolean {
        return !shown.has(this) && this.read<number>("source") === 0;
    }

    /** Camera used to render the content, or null for flat rendering. */
    get camera(): Camera3D | null {
        const id = this.read<number>("camera");
        const found = id ? live.get(id) : undefined;
        return found instanceof Camera3D ? found : null;
    }
    /** Sets the camera used to render this View. */
    set camera(value: Camera3D | null) {
        this.write("camera", value === null ? 0 : nodeIdOf(value));
    }

    /** Width of the View's picture in client pixels. */
    get width(): number {
        return this.read<number>("width");
    }
    /** Sets picture width in client pixels. */
    set width(value: number) {
        this.write("width", value);
    }

    /** Picture height in client pixels. */
    get height(): number {
        return this.read<number>("height");
    }
    /** Sets picture height in client pixels. */
    set height(value: number) {
        this.write("height", value);
    }

    /** The View's picture dimensions as `{ width, height }`. */
    get size(): Size2D {
        return { width: this.width, height: this.height };
    }
    /** Sets the picture dimensions in client pixels. */
    set size(value: Size2D) {
        if (typeof value !== "object" || value === null) {
            throw new TypeError(`${label(this)}.size: must be { width, height }`);
        }
        const width = finite(`${label(this)}.size.width`, value.width);
        const height = finite(`${label(this)}.size.height`, value.height);
        this.width = width;
        this.height = height;
    }

    /** How a workspace picture fits its size: `contain`, `cover` or `stretch` (the default). */
    get contentFit(): ContentFit {
        noteSignal(this, "fit");
        return fits.get(this) ?? "stretch";
    }
    /** Sets how workspace content is fitted to the View's size. */
    set contentFit(value: ContentFit) {
        if (value !== "contain" && value !== "cover" && value !== "stretch") {
            throw new TypeError(`${label(this)}.contentFit: must be "contain", "cover" or "stretch"`);
        }
        fits.set(this, value);
        raiseSignal(this, "fit");
    }

    /** Stops workspace mirroring before destroying this View. */
    override destroy(): void {
        unshow(this);
        super.destroy();
    }
}

/** Stops a workspace mirror and removes its nodes before the View is destroyed or its content changes. */
function unshow(view: View): void {
    shown.get(view)?.mirror.stop();
    shown.delete(view);
}
