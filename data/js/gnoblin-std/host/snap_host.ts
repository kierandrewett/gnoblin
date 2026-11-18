// What snapping asks of the library for a window: the snap lives with the window's frame inset and the rectangle it floats at, where
// the library's refit reads it. A window the library does not know yet has no snap, and a snap asked of it is refused.

import { isClient } from "gnoblin-core/host/node_state";
import type { Client } from "gnoblin-core/host/client";
import { SnapKind } from "gnoblin-std/generated/constants";
import { whenKnown } from "gnoblin-std/host/client_state";
import { nativeClient } from "gnoblin-std/host/library";
import { attachSnap } from "gnoblin-std/snap";

/** Attaches snapping to the library. */
export function installSnap(): void {
    attachSnap({
        owns: (client) => isClient(client),
        snap: (client, zone, area, edgeTiling) =>
            whenKnown(
                () => nativeClient(client as Client).snap(zone, area, edgeTiling ? 1 : 0),
                () => false,
            ),
        unsnap: (client, forget) =>
            whenKnown(
                () => nativeClient(client as Client).unsnap(forget ? 1 : 0),
                () => false,
            ),
        record: (client) =>
            whenKnown(
                () => {
                    const state = nativeClient(client as Client).state();
                    return state.snapZone === SnapKind.NONE
                        ? null
                        : { zone: state.snapZone, floating: { ...state.snapFloating } };
                },
                () => null,
            ),
        restore: (client, zone, floating) => nativeClient(client as Client).restoreSnap(zone, floating),
    });
}
