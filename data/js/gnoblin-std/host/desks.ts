// Which output shows which workspace: the library's desks, asked about an output at a time. The desks themselves (one for each
// output that can show a workspace, parked while their output is gone, spanned over several outputs when a display says so) are
// `libgnoblin-std`'s; this module names the answers a script's side needs.

import { host } from "gnoblin-core/host/bridge";
import { ctx } from "gnoblin-core/host/context";
import { stateOf } from "gnoblin-core/host/node_state";
import type { Output } from "gnoblin-core/host/output";
import { OutputFlags } from "gnoblin-std/generated/constants";
import type { OutputInfo } from "gnoblin-std/generated/records";
import { nativeOutput } from "gnoblin-std/host/library";
import { workspaceOf } from "gnoblin-std/host/membership";
import type { Workspace } from "gnoblin-std/host/workspace";

/** What the library knows of an output. */
export function outputInfo(output: Output): OutputInfo {
    return nativeOutput(output).info();
}

/** The outputs that are connected, in the order the compositor made them. */
export function liveOutputs(): Output[] {
    return ctx.outputRoot.list as Output[];
}

/** Whether the output has a desk of its own: it is on and not a mirror. */
export function hasDesk(output: Output): boolean {
    return stateOf(output).alive && (outputInfo(output).flags & OutputFlags.HAS_DESK) !== 0;
}

/** The outputs that have a desk: every connected output but a mirror and one that is off. */
export function deskOutputs(): Output[] {
    return liveOutputs().filter(hasDesk);
}

/** The output flagged primary, or the first one; null while the tree has none. */
export function primaryOutput(): Output | null {
    const list = deskOutputs();
    return list.find((output) => output.primary) ?? list[0] ?? null;
}

/** The workspace an output shows (the one its span shows, for a spanned output), or null. */
export function shownOn(output: Output): Workspace | null {
    const shown = nativeOutput(output).workspace();
    return shown === null ? null : workspaceOf(shown);
}

/** The workspace a new window, a key and a plain `show` act on: the one the output under the pointer shows, else the primary one's. */
export function currentWorkspace(): Workspace | null {
    const node = host.pointerScreen();
    const under = node === undefined ? undefined : ctx.outputsByNode.get(node);
    const output = under !== undefined && hasDesk(under) ? under : primaryOutput();
    return output === null ? null : shownOn(output);
}
