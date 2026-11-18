// Output rectangles and layout work areas, as the library holds them: an output's rectangle less the room the shell's bars take,
// and the output a client is on (the one that holds the middle of it when its workspace spans several).

import { shrinkRect } from "gnoblin-core/inset";
import type { Client } from "gnoblin-core/host/client";
import { stateOf } from "gnoblin-core/host/node_state";
import type { Output } from "gnoblin-core/host/output";
import type { Rect } from "gnoblin-core/types";
import { Client as NativeClient } from "gnoblin-std/generated/classes";
import { AreasFlags, OutputFlags } from "gnoblin-std/generated/constants";
import { nativeClient } from "gnoblin-std/host/library";
import { outputInfo } from "gnoblin-std/host/desks";

/** What a layout tiles in on `output`: its rectangle less the bars on it. Null for a gone output or one with no size. */
export function areaOf(output: Output): Rect | null {
    if (!stateOf(output).alive) {
        return null;
    }
    const info = outputInfo(output);
    return (info.flags & OutputFlags.HAS_RECT) === 0 ? null : info.area;
}

/** The areas of the output `client` is on, or of the primary output when none is asked for. */
function areasOf(client: Client | null) {
    return (client === null ? new NativeClient(0) : nativeClient(client)).areas();
}

/** The area windows are laid out in on the primary output. Null when the tree has no output yet. */
export function screenArea(): Rect | null {
    const areas = areasOf(null);
    return (areas.flags & AreasFlags.HAS_AREA) === 0 ? null : areas.area;
}

/** The output rectangle `client` is on, or the primary output's when its workspace is on none. */
export function outputRectOf(client: Client | null): Rect | null {
    const areas = areasOf(client);
    return (areas.flags & AreasFlags.HAS_OUTPUT) === 0 ? null : areas.output;
}

/** The work area of the output `client` is on, or the primary output's when its workspace is on none. */
export function workAreaOf(client: Client | null): Rect | null {
    const areas = areasOf(client);
    return (areas.flags & AreasFlags.HAS_AREA) === 0 ? null : areas.area;
}

/**
 * The content's rectangle for a frame: the window's frame lies round the content, so the content is the frame less the
 * frame's inset on each side. A client with no frame is the frame.
 */
export function framed(client: Client, frame: Rect): Rect {
    return shrinkRect(frame, client.frameInset);
}
