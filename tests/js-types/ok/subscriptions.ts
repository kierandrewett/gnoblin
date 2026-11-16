// The named subscriptions the library declares from its listener classes: each handler takes what the listener method
// takes, and each call returns the function that detaches it.
import { gnoblin } from "gnoblin-core";
import type { Client, Output, Unsubscribe } from "gnoblin-core";
import "gnoblin-std";

export const stops: Unsubscribe[] = [
    // `gnoblin` forwards to the collections.
    gnoblin.onClientCreated((client) => {
        console.log(client.title);
    }),
    gnoblin.onClientTitleChanged((client, title) => console.log(client.appId, title.length)),
    gnoblin.onClientFocusChanged((client, focused) => console.log(client.id, focused === true)),
    gnoblin.onClientGeometryChanged((client, geometry) => console.log(client.id, geometry.width)),
    gnoblin.onClientRequest((_client, request) => request.handle()),
    gnoblin.onOutputConnected((output) => console.log(output.connector)),
    gnoblin.onOutputAdded((output, outputs) => console.log(output.name, outputs.length)),
    gnoblin.onOutputModeChanged((output, mode) => console.log(output.name, mode.length)),
    // A collection hears every member, and one object hears itself.
    gnoblin.clients.onTitleChanged((client: Client, title: string) => console.log(client.id, title)),
    gnoblin.clients.onAdded((client, collection) => console.log(client.id, collection.length)),
    gnoblin.clients.onRequest((_client, request) => request.handle()),
    gnoblin.outputs.onScaleChanged((output: Output, scale: number) => console.log(output.name, scale)),
    gnoblin.outputs.onRemoved((output) => console.log(output.name)),
    gnoblin.clients.where({ focused: true }).onClosed((client) => console.log(client.id)),
    // The session and the runtime are served by the same implementation.
    gnoblin.session.onBell((session, count) => console.log(session.status().state, count)),
    gnoblin.runtime.onConfigReloaded((_runtime, reload) => console.log(reload.path)),
];

for (const client of gnoblin.clients) {
    stops.push(client.onClosed((closed) => console.log(closed.id)));
}

// `Output.apply` is an operation, like the other calls that finish later.
const output = gnoblin.outputs.at(0);
if (output !== undefined) {
    output.apply({ scale: 1.25 }).onComplete((applied, error) => {
        console.log(applied?.name, error?.code, error?.message);
    });
}
