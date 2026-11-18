// What a failed transition puts back of the workspaces: the layout each had, its gaps and name, the own fields of that layout and
// which workspace each output showed. The windows' own writes are the transition's (it drops what it staged); these are the
// script's objects and the library's choices a callback can have changed.

import { deskOutputs, shownOn } from "gnoblin-std/host/desks";
import { bridgeLayout } from "gnoblin-std/host/layout_bridge";
import { showWorkspace } from "gnoblin-std/host/show";
import { std } from "gnoblin-std/host/state";
import type { Workspace } from "gnoblin-std/host/workspace";
import { wsState } from "gnoblin-std/host/workspace_state";
import { layoutFields } from "gnoblin-std/workspace";

/**
 * Notes the workspaces as they are now; the function it returns puts them back. A layout a callback assigned (and so was not the
 * workspace's layout when this was called) keeps the field changes made to it.
 */
export function snapshotWorkspaceState(): () => void {
    const restores: (() => void)[] = [];
    for (const workspace of std.workspaceRoot.list as Workspace[]) {
        const state = wsState(workspace);
        const { layout: held, bridge, gaps } = state;
        const name = state.native.info().name;
        const own = layoutFields(held);
        restores.push(() => {
            if (state.layout !== held) {
                state.layout = held;
                state.bridge = bridge;
                state.native.setLayout(bridgeLayout(held, () => workspace).spec);
            }
            state.gaps = gaps;
            state.native.setGaps(gaps.inner, gaps.outer);
            state.gapsSeen = [gaps.inner, gaps.outer];
            state.native.setName(name);
            for (const key of Reflect.ownKeys(held)) {
                if (Object.getOwnPropertyDescriptor(held, key)?.enumerable && !own.has(key)) {
                    Reflect.deleteProperty(held, key);
                }
            }
            for (const [key, value] of own) {
                Reflect.set(held, key, value);
            }
        });
    }
    const shownBefore = deskOutputs().map((output) => [output, shownOn(output)] as const);
    restores.push(() => {
        for (const [output, shown] of shownBefore) {
            if (shown !== null && output.alive && shownOn(output) !== shown) {
                showWorkspace(shown, { output });
            }
        }
    });
    return () => {
        for (const restore of restores) {
            restore();
        }
    };
}
