// What the script's side tells the library at the start of every frame, and reads back: the library lays the workspaces out in its
// own turn after the engine's frame, so what a layout reads (the gaps a script wrote into, the room the bars take, the frames
// round the windows) is handed over first, and the lists a script holds are made the library's.

import { sameInset } from "gnoblin-core/inset";
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
import { NO_INSET } from "gnoblin-core/inset";
import { barInsets } from "gnoblin-std/widgets";

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
        untilKnown(() => {
            const native = nativeClient(client);
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
