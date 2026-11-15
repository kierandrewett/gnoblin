// A script that works with the objects of the shell: each is a `NativeObject`, and a client takes the property
// writes the library declares.
import { Client, FloatingLayout, NativeObject, Output, Workspace, gnoblin } from "gnoblin";

export const objects: NativeObject[] = [];

export function dim(client: Client): NativeObject {
    client.opacity = 0.9;
    return client;
}

export function show(output: Output): string {
    return output.toString();
}

for (const client of gnoblin.clients) {
    objects.push(dim(client));
}
gnoblin.workspaces.add(new Workspace({ id: "work", name: "Work", layout: new FloatingLayout() }));
console.log("configured", gnoblin.clients.length);
