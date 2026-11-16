// The root collections the compositor owns: `gnoblin.clients` and `gnoblin.outputs`. Each is made once per generation by
// `installRoots` and kept on the context. A package that has collections of its own (`gnoblin.workspaces`) makes them
// itself.

import { GnoblinError } from "gnoblin-core/errors";
import { CREATE } from "gnoblin-core/host/bridge";
import type { Client } from "gnoblin-core/host/client";
import { collectionState, rootState } from "gnoblin-core/host/collection_state";
import { Collection, MutableCollection } from "gnoblin-core/host/collections";
import { ctx } from "gnoblin-core/host/context";
import type { Output } from "gnoblin-core/host/output";
import { VirtualOutput } from "gnoblin-core/virtual_output";

/** Refuses virtual outputs because this backend cannot create them. */
function addVirtualOutput(member: unknown): void {
    if (!(member instanceof VirtualOutput)) {
        throw new TypeError("gnoblin.outputs.add: expected a VirtualOutput; the compositor makes every other output");
    }
    throw new GnoblinError("unsupported", "Collection.add", "virtual outputs are not supported by this backend");
}

/** `gnoblin.outputs`: the connected outputs. */
export class OutputCollection extends MutableCollection<Output> {}

/** Creates the root collections for the current generation. */
export function installRoots(): void {
    // All native clients in compositor order.
    ctx.clientRoot = collectionState(new Collection<Client>(CREATE, rootState("Client")));

    // Connected outputs in compositor creation order. A script may add a `VirtualOutput`; it may remove nothing.
    ctx.outputRoot = collectionState(
        new OutputCollection(CREATE, {
            ...rootState("Output"),
            mutation: {
                add: addVirtualOutput,
                remove: () => false,
                clear: () => {
                    throw new GnoblinError("read_only", "Collection.clear", "outputs belong to the compositor");
                },
            },
        }),
    );
}
