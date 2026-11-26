// A script that works with the objects of the shell: each is a class the native library declares, and a client takes the
// property writes the library declares.
import { Client, Output, gnoblin } from "gnoblin-core";
import { FloatingLayout, Workspace } from "gnoblin-std";

export const clients: Client[] = [];

export function dim(client: Client): Client {
    client.opacity = 0.9;
    return client;
}

export function show(output: Output): string {
    return output.name;
}

for (const client of gnoblin.clients.toList()) {
    clients.push(dim(client));
}
gnoblin.workspaces.add(new Workspace("work", { name: "Work", layout: new FloatingLayout() }));
console.log("configured", gnoblin.clients.length);
