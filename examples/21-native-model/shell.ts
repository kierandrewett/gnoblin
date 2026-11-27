// Example 21 in TypeScript: the object model of libgnoblin-core, through the classes generated from its catalog.
//
//     gnoblin ./shell.ts
//
// Does what the Rust shell of the same example (`21-native-model`) does and logs the same lines, which start with `model:`: reads the
// windows there, looks up one that is not there (`undefined`, not an error) and one that is, follows the windows of one application with
// `where`, and listens to titles and to closings.
import { Gnoblin } from "gnoblin-core";

const model = Gnoblin.get();
const clients = model.clients;
console.log(`model: ${clients.length} windows at the start`);
console.log(`model: absent lookups: at ${String(clients.at(1000))}, get ${String(clients.get("no-such-window"))}`);

const terminals = clients.where({ appId: "foot" });
terminals.on("added", (client) => {
    console.log(`model: a terminal joined: ${client.title}`);
    console.log(`model: the terminal is found by its id: ${clients.get(client.id) !== undefined}`);
});
clients.on("titleChanged", (client, title) => {
    console.log(`model: window ${client.id} is now titled ${JSON.stringify(title)}`);
});
model.on("clientClosed", (client) => {
    console.log(`model: window ${client.id} closed, last titled "${client.title}"`);
});
console.log("model: running");
