// The subscriptions the generated classes declare: each handler takes what the event carries, and each call returns the
// `Listener` that detaches it.
import { gnoblin } from "gnoblin-core";
import type { Client, Output } from "gnoblin-core";
import type { Listener } from "gnoblin-core/generated/object_base";
import "gnoblin-std";

export const listeners: Listener[] = [
    // `gnoblin` forwards the events of the collections.
    gnoblin.onClientCreated((client) => {
        console.log(client.title);
    }),
    gnoblin.onClientTitleChanged((client, title) => console.log(client.appId, title.length)),
    gnoblin.onClientFocusChanged((client, focused) => console.log(client.id, focused === true)),
    gnoblin.onOutputConnected((output) => console.log(output.connector)),
    gnoblin.onOutputAdded((output, outputs) => console.log(output.name, outputs.length)),
    // A collection hears every member, and one object hears itself.
    gnoblin.clients.on("titleChanged", (client: Client, title: string) => console.log(client.id, title)),
    gnoblin.clients.on("added", (client, collection) => console.log(client.id, collection.length)),
    gnoblin.outputs.on("removed", (output: Output) => console.log(output.name)),
    gnoblin.clients.where({ focused: true }).on("removed", (client) => console.log(client.id)),
    // The session and the runtime are served by the same implementation.
    gnoblin.session.on("bell", (session, count) => console.log(session.status?.state, count)),
    gnoblin.runtime.on("configReloaded", (_runtime, reload) => console.log(reload.path)),
];

for (const client of gnoblin.clients.toList()) {
    listeners.push(client.on("closed", (closed) => console.log(closed.id)));
}
