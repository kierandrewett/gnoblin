import { Camera3D } from "gnoblin-std/scene/camera";
import { Node } from "gnoblin-std/scene/node";
import type { NodeOptions } from "gnoblin-std/scene/node";
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
/**
 * A node that draws a client or workspace as a picture without moving it. A client is shown directly. A workspace is
 * shown through a mirror made of child Views, fitted to `size` by `contentFit`; those mirror children are not in
 * `children`. The compositor stretches a client's picture to `size`; `contentFit` only affects workspace pictures.
 */
export declare class View extends Node {
    constructor(options?: ViewOptions, cls?: string);
    protected assign(options: ViewOptions): void;
    /** The client or workspace the View shows, or null when it shows none or its client is gone. */
    get content(): object | null;
    /** Replaces the displayed client or workspace; null clears the picture. */
    set content(value: object | null);
    /** Whether the client content is gone; workspace content is not reported as orphaned. */
    get orphaned(): boolean;
    /** Camera used to render the content, or null for flat rendering. */
    get camera(): Camera3D | null;
    /** Sets the camera used to render this View. */
    set camera(value: Camera3D | null);
    /** Width of the View's picture in client pixels. */
    get width(): number;
    /** Sets picture width in client pixels. */
    set width(value: number);
    /** Picture height in client pixels. */
    get height(): number;
    /** Sets picture height in client pixels. */
    set height(value: number);
    /** The View's picture dimensions as `{ width, height }`. */
    get size(): Size2D;
    /** Sets the picture dimensions in client pixels. */
    set size(value: Size2D);
    /** How a workspace picture fits its size: `contain`, `cover` or `stretch` (the default). */
    get contentFit(): ContentFit;
    /** Sets how workspace content is fitted to the View's size. */
    set contentFit(value: ContentFit);
    /** Stops workspace mirroring before destroying this View. */
    destroy(): void;
}
