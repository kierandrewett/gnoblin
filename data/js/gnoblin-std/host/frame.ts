// What the script's side tells the library at the start of every frame, and reads back: the library lays the workspaces out in its
// own turn after the engine's frame, so what a layout reads (the gaps a script wrote into, the room the bars take, the frames
// round the windows) is handed over first, and the lists a script holds are made the library's.

import { growRect, NO_INSET, sameInset, shrinkRect } from "gnoblin-core/inset";
import type { FrameInset } from "gnoblin-core/inset";
import type { Rect } from "gnoblin-core/types";
import type { Client } from "gnoblin-core/host/client";
import { ctx } from "gnoblin-core/host/context";
import { guard } from "gnoblin-core/host/dispatch";
import { flushBindings } from "gnoblin-core/properties";
import { bridgeLayout } from "gnoblin-std/host/layout_bridge";
import { liveOutputs } from "gnoblin-std/host/desks";
import { failureCode, FAILED, nativeClient, nativeOutput } from "gnoblin-std/host/library";
import { flushHeld } from "gnoblin-std/host/client_state";
import { flushMoves, syncMembership } from "gnoblin-std/host/membership";
import { std } from "gnoblin-std/host/state";
import { applyStyles } from "gnoblin-std/host/styles";
import { wsState } from "gnoblin-std/host/workspace_state";
import { workAreaOf } from "gnoblin-std/host/work_area";
import { barInsets } from "gnoblin-std/widgets";
import { windowState } from "gnoblin-std/window_state";

/** How many frames a window that closed stays known to the layouts that are told of it. */
const CLOSED_FRAMES = 4;

/** A window the library does not know yet (it appeared this frame) is asked again next frame. */
function untilKnown(call: () => void): void {
    try {
        call();
    } catch (error) {
        if (failureCode(error) !== FAILED.noSuchNode) {
            throw error;
        }
    }
}

/** Hands the library what a script wrote into the objects it holds: a workspace's gaps and the options of its floating layout. */
function sendWorkspaces(): void {
    for (const workspace of std.byHandle.values()) {
        const state = wsState(workspace);
        if (state.gaps.inner !== state.gapsSeen[0] || state.gaps.outer !== state.gapsSeen[1]) {
            state.gapsSeen = [state.gaps.inner, state.gaps.outer];
            state.native.setGaps(state.gaps.inner, state.gaps.outer);
        }
        if (state.bridge.stale()) {
            state.bridge = bridgeLayout(state.layout, () => workspace);
            state.native.setLayout(state.bridge.spec);
        } else if (state.bridge.changed()) {
            // The layout's own fields are what it is: the library asks for its revision, but only when it has a reason to look.
            state.native.invalidate();
        }
    }
}

/** Tells the library how much room the shell's bars take from each output: a layout tiles in what is left. */
function sendInsets(): void {
    for (const output of liveOutputs()) {
        const { top, bottom } = barInsets(output);
        const key = `${top},${bottom}`;
        if ((std.insets.get(output) ?? "0,0") === key) {
            continue;
        }
        untilKnown(() => {
            nativeOutput(output).setInsets(top, bottom);
            std.insets.set(output, key);
        });
    }
}

/** A rectangle that moved to another place: shrunk to the area when larger, then moved inside it. */
function fitInto(rect: Rect, area: Rect): Rect {
    const width = Math.min(rect.width, area.width);
    const height = Math.min(rect.height, area.height);
    return {
        x: Math.min(Math.max(rect.x, area.x), area.x + area.width - width),
        y: Math.min(Math.max(rect.y, area.y), area.y + area.height - height),
        width,
        height,
    };
}

/**
 * A frame changed its room: the FRAME keeps its place, so the rectangle the script's side keeps for the window to go back to
 * (maximise and fullscreen remember where it floated) moves in by what the top and left gained and stays inside the area.
 * The library does the same for what it keeps (the rectangle a window floats at, a snap's).
 */
function keepFramePlace(client: Client, was: FrameInset, inset: FrameInset): void {
    const area = workAreaOf(client);
    const kept = windowState.kept(client).floating;
    if (area === null || kept === null) {
        return;
    }
    const moved = { ...kept, x: kept.x + inset.left - was.left, y: kept.y + inset.top - was.top };
    windowState.keep(client, { floating: shrinkRect(fitInto(growRect(moved, inset), area), inset) });
}

/** Tells the library the room each window's frame takes, which a layout places the frame round. */
function sendFrames(): void {
    const decorations = std.decorations;
    if (decorations === null) {
        return;
    }
    for (const client of ctx.clientRoot.list as Client[]) {
        const inset = decorations.reservedOf(client);
        if (sameInset(inset, std.insetSent.get(client) ?? NO_INSET)) {
            continue;
        }
        const was = std.insetSent.get(client);
        untilKnown(() => {
            const native = nativeClient(client);
            if (was !== undefined) {
                keepFramePlace(client, was, inset);
            }
            native.setState({
                ...native.state(),
                insetTop: inset.top,
                insetRight: inset.right,
                insetBottom: inset.bottom,
                insetLeft: inset.left,
            });
            std.insetSent.set(client, inset);
        });
    }
}

/**
 * A generation that frames no window gives back the room an earlier generation's frames took, which the library still holds when a
 * reload stays in one core generation (a runtime reload opens a new one, whose library holds none).
 */
export function clearInheritedFrames(): void {
    if (std.decorations !== null) {
        return;
    }
    for (const client of ctx.clientRoot.list as Client[]) {
        untilKnown(() => {
            const native = nativeClient(client);
            const state = native.state();
            if (state.insetTop + state.insetRight + state.insetBottom + state.insetLeft > 0) {
                native.setState({ ...state, insetTop: 0, insetRight: 0, insetBottom: 0, insetLeft: 0 });
            }
        });
    }
}

/**
 * Runs once a frame, after the listeners. The layout itself is not run here: it is the library's turn, which follows the
 * engine's frame, and it writes the placements as tree writes of its own.
 */
export function layout(): number {
    std.frame += 1;
    for (const [node, gone] of std.closed) {
        if (std.frame - gone.frame > CLOSED_FRAMES) {
            std.closed.delete(node);
        }
    }
    // Bindings first, so what they write is handed over in the same frame.
    guard("gnoblin.properties", flushBindings);
    guard("gnoblin.library", () => {
        flushHeld();
        flushMoves();
        sendWorkspaces();
        sendInsets();
        sendFrames();
        syncMembership();
    });
    std.displayManager.frame();
    guard("gnoblin.style", applyStyles);
    return 0;
}
