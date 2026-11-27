// Example 36 in TypeScript: a window the shell holds as a `Client` of the core, passed to the classes of libgnoblin-std that take one.
//
//     gnoblin ./shell.ts
//
// Does what the Rust shell of the same example (`36-native-clients`) does and logs the same lines, which start with `clients:`: when a window
// appears it asks `Decorations` for the room of the window's frame (`insetOf`), opens the window menu on it (`WindowMenus.show`) and closes
// the menu, and adds it to a workspace (`clients.add`). The library checks every handle against the core: text where a `Client` is taken, an
// object of another class and a window that has closed are each refused, and the log says how. It then reads the members the library adds to
// a window, an output and the root once it is loaded: the window's `workspace`, `moveToWorkspace`, `style`, `styleWarnings`, `moveWithKeys`,
// `output`, `moveToOutput` and `frameInset`, the output's `workspace`, `area` and `show`, and the root's `show`.
import { Gnoblin } from "gnoblin-core";
import type { Client } from "gnoblin-core";
import { Decorations, WindowMenus, Workspace, Workspaces } from "gnoblin-std";
import type {} from "gnoblin-std";

const say = (text: string) => console.log(`clients: ${text}`);
const refused = (error: unknown) => `${(error as Error).constructor.name}: ${(error as Error).message}`;

const gnoblin = Gnoblin.get();
const decorations = Decorations.get();
const menus = WindowMenus.get();
menus.start();
const main = new Workspace("main");
const side = new Workspace("side");
Workspaces.get().add(main);
Workspaces.get().add(side);

// The members the library adds to the core's `Client` and `Output`, read once, for the first window.
let checked = false;
function checkMembers(client: Client): void {
    checked = true;
    say(`${client.title}: its workspace is main: ${client.workspace?.id === "main"}`);
    const moved = client.moveToWorkspace(side);
    say(`${client.title}: moved to side: ${moved}, now in side: ${client.workspace?.id === "side"}`);
    const back = client.moveToWorkspace(main);
    say(`${client.title}: moved back: ${back}, again refused: ${!client.moveToWorkspace(main)}`);
    client.style.opacity = 0.8;
    say(`${client.title}: its own style holds 0.8: ${Math.abs(client.style.opacity - 0.8) < 1e-6}`);
    say(`${client.title}: its style warnings are a list: ${Array.isArray(client.styleWarnings)}`);
    // A selection styles the windows it holds and no others, and a selection that can hold no window has no style.
    const clients = gnoblin.clients!;
    const hasNoStyle = (query: Parameters<typeof clients.where>[0]) => {
        try {
            void clients.where(query).style;
            return false;
        } catch (error) {
            return (error as Error).message.includes("holds no window");
        }
    };
    say(`${client.title}: an empty any list has no style: ${hasNoStyle({ anyAppId: [] })}`);
    const outside = hasNoStyle({ appId: "foot", anyAppId: ["x"] });
    say(`${client.title}: an app id outside its any list has no style: ${outside}`);
    const matching = clients.where({ appId: "foot", anyAppId: ["foot", "x"] }).style;
    matching.background = "#112233";
    clients.where({ appId: "nope" }, 10).style.background = "#ffffff";
    clients.where({ appId: "foot" }).where({ title: "nope" }, 20).style.background = "#ffffff";
    const only = client.computedStyle.background === matching.background;
    say(`${client.title}: only the matching selections style it: ${only}`);
    say(`${client.title}: keyboard move before Repositioning starts: ${client.moveWithKeys()}`);
    const output = gnoblin.outputs!.at(0)!;
    say(`${client.title}: the output shows main: ${output.workspace?.id === "main"}`);
    output.show(side);
    const showed = output.workspace?.id === "side";
    output.show(main);
    say(`${client.title}: the output showed side: ${showed}`);
    say(`${client.title}: its output is the first: ${client.output?.id === output.id}`);
    say(`${client.title}: a move to the output it is on is refused: ${!client.moveToOutput(output)}`);
    const inset = client.frameInset;
    const sides = [inset.top, inset.right, inset.bottom, inset.left];
    say(`${client.title}: its frame inset has 4 sides: ${sides.every((room) => room >= 0)}`);
    const area = output.area;
    say(
        `${client.title}: the output's area is inside it: ${area !== undefined && area.width > 0 && area.width <= output.width}`,
    );
    gnoblin.show(side);
    const shown = output.workspace?.id === "side";
    gnoblin.show(main);
    say(`${client.title}: gnoblin.show showed side: ${shown}`);
    client.floating = true;
    say(`${client.title}: it floats once set: ${client.floating}`);
    client.floatingBounds = { x: 10, y: 20, width: 300, height: 200 };
    say(`${client.title}: it keeps its floating bounds: ${client.floatingBounds?.width === 300}`);
    client.floatingBounds = null;
    client.floating = false;
    say(`${client.title}: it forgets them: ${client.floatingBounds === undefined && !client.floating}`);
    // `set` writes several properties in one transition; without options nothing animates.
    client.set({ floating: true });
    const floated = client.floating;
    client.set({ floating: false });
    say(`${client.title}: set wrote floating: ${floated && !client.floating}`);
    try {
        client.set({ title: "x" } as never);
        say(`${client.title}: set refused title: no`);
    } catch (error) {
        say(`${client.title}: set refused title: ${refused(error)}`);
    }
    main.set({ name: "Primary" });
    const named = main.name === "Primary";
    main.set({ name: "" });
    say(`${client.title}: a workspace's set wrote its name: ${named}`);
}

// The library puts a window on the workspace as it appears and tells the shell of it with the core's `Client`: the same object the core's own
// collections give, which `add` and `contains` take as it is.
const members = main.clients!;
members.on("added", (client) => {
    members.add(client);
    say(`${client.title}: it is in main: ${members.contains(client)}`);
    say(`${client.title}: the library's client is the core's: ${client === gnoblin.clients.get(client.id)}`);
    if (!checked) {
        checkMembers(client);
    }
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
