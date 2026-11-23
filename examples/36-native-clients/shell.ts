// Example 36 in TypeScript: a window the shell holds as a `Client` of the core, passed to the classes of libgnoblin-std that take one.
//
//     gnoblin ./shell.ts
//
// Does what the Rust shell of the same example (`36-native-clients`) does and logs the same lines, which start with `clients:`: when a window
// appears it asks `Decorations` for the room of the window's frame (`insetOf`), opens the window menu on it (`WindowMenus.show`) and closes
// the menu, and adds it to a workspace (`clients.add`). The library checks every handle against the core: text where a `Client` is taken, an
// object of another class and a window that has closed are each refused, and the log says how.
import { Gnoblin } from "gnoblin-core/generated/objects";
import { Decorations, WindowMenus, Workspace, Workspaces } from "gnoblin-std/generated/objects";
import type {} from "gnoblin-std";

const say = (text: string) => console.log(`clients: ${text}`);
const refused = (error: unknown) => `${(error as Error).constructor.name}: ${(error as Error).message}`;

const gnoblin = Gnoblin.get();
const decorations = Decorations.get();
const menus = WindowMenus.get();
menus.start();
const main = new Workspace("main");
Workspaces.get().add(main);

// The library puts a window on the workspace as it appears and tells the shell its id as text. `clients.get(id)` is the `Client` of it,
// which `add` and `contains` take.
const members = main.clients!;
members.on("added", (id: string) => {
    const client = gnoblin.clients.get(id);
    if (client === undefined) {
        return;
    }
    members.add(client);
    say(`${client.title}: it is in main: ${members.contains(client)}`);
});

gnoblin.clients.on("added", (client) => {
    const inset = decorations.insetOf(client);
    say(`${client.title}: the frame takes ${inset.top} ${inset.right} ${inset.bottom} ${inset.left}`);
    const menu = menus.show(client, 10, 10);
    say(`${client.title}: the menu is for the window: ${menu?.client === client.id}`);
    menus.close();
    try {
        decorations.insetOf(client.id as never);
    } catch (error) {
        say(`${client.title}: an id is refused: ${refused(error)}`);
    }
    try {
        decorations.insetOf(gnoblin as never);
    } catch (error) {
        say(`${client.title}: another class is refused: ${refused(error)}`);
    }
});

// A window that has closed is still a `Client`, and its last values can be read; the library says it is gone.
gnoblin.on("clientClosed", (client) => {
    try {
        decorations.insetOf(client);
        say(`${client.title}: a closed window was answered`);
    } catch (error) {
        say(`${client.title}: a closed window is refused: ${refused(error)}`);
    }
});
say("running");
