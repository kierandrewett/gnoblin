// Example 29 in TypeScript: the window menu of libgnoblin-std, through the classes generated from its catalog.
//
//     gnoblin ./shell.ts
//
// Starts the window menu, gives it three entries of this shell's own (a `MenuProvider`) that log what they run, marks the windows of the app
// `secret` secure (their card is secure too), and logs what the menu does. Every line starts with `menus:`, as the Rust shell of the same
// example spells it.
import { Gnoblin } from "gnoblin-core";
import { MenuEntry, MenuProvider, WindowMenus } from "gnoblin-std";
import type {} from "gnoblin-std";

class Entries extends MenuProvider {
    items(_client: string): MenuEntry[] {
        return ["Alpha", "Beta", "Gamma"].map((label) => {
            const entry = new MenuEntry(label);
            entry.run = (client: string) => {
                console.log(`menus: ran ${label} on window ${client}`);
            };
            return entry;
        });
    }
}

const menus = WindowMenus.get();
menus.provider = new Entries();

Gnoblin.get().clients.on("added", (client) => {
    if (client.appId === "secret") {
        client.secure = true;
    }
});

menus.on("currentChanged", (menu) => {
    console.log(`menus: a card opened for window ${menu.client}, secure: ${menu.panel.secure}`);
    menu.nav.on("selectedChanged", (row) => {
        console.log(`menus: the keyboard is on ${row.label}`);
    });
    menu.on("closed", () => {
        console.log("menus: the card closed");
    });
});
menus.start();
console.log("menus: running");
