// Console representations of host objects, including concise client summaries in nested values.

import { Client } from "gnoblin-core/host/client";
import { Collection } from "gnoblin-core/host/collections";
import { Output } from "gnoblin-core/host/output";
import { focusedAmong } from "gnoblin-core/host/seat";
import { inspect } from "gnoblin-core/inspect";

/** Registers how the console renders each kind of host object. A package that adds fields or kinds registers again. */
export function registerInspectors(): void {
    // Client summaries are concise when nested in another value; collection items include only id, app and title.
    inspect.register(Client, (client) => ({
        name: "Client",
        id: client.id,
        fields: {
            appId: client.appId,
            title: client.title,
            focused: focusedAmong([client]) === client,
            fullscreen: client.fullscreen,
            x: client.x,
            y: client.y,
            width: client.width,
            height: client.height,
            ...(client.alive ? {} : { alive: false }),
        },
        brief: { appId: client.appId, title: client.title },
    }));

    inspect.register(Output, (output) => ({
        name: "Output",
        id: output.id,
        fields: {
            name: output.name,
            mode: output.mode,
            scale: output.scale,
            primary: output.primary,
            x: output.x,
            y: output.y,
            width: output.width,
            height: output.height,
            ...(output.connected ? {} : { connected: false }),
        },
        brief: { name: output.name, mode: output.mode },
    }));

    inspect.register(Collection, (collection) => ({
        name: "Collection",
        fields: {},
        items: [...collection],
    }));
}
