import type { Client } from "gnoblin-core/host/client";
import type { Output } from "gnoblin-core/host/output";
import type { Rect } from "gnoblin-core/types";
/** What a layout tiles in on `output`: its rectangle less the bars on it. Null for a gone output or one with no size. */
export declare function areaOf(output: Output): Rect | null;
/** The area windows are laid out in on the primary output. Null when the tree has no output yet. */
export declare function screenArea(): Rect | null;
/** The output rectangle `client` is on, or the primary output's when its workspace is on none. */
export declare function outputRectOf(client: Client | null): Rect | null;
/** The work area of the output `client` is on, or the primary output's when its workspace is on none. */
export declare function workAreaOf(client: Client | null): Rect | null;
/**
 * The content's rectangle for a frame: the window's frame lies round the content, so the content is the frame less the
 * frame's inset on each side. A client with no frame is the frame.
 */
export declare function framed(client: Client, frame: Rect): Rect;
