// Example 37 in TypeScript: the members the std library adds to the `gnoblin` root of the core, through the classes generated from the catalogs.
//
//     gnoblin ./shell.ts
//
// Importing `gnoblin-std/generated/objects` gives the core's `gnoblin` the members `workspaces`, `displays`, `scene`, `gestures` and
// `requestTransition`, its `outputs` the member `profiles`, each input context `activeDisplay` and each collection of windows `style`. A shell
// that never imports it (example 35) has none of them. This shell:
//
// - adds two workspaces to `gnoblin.workspaces`, reads `scene` and `gestures`, and runs a transition through `gnoblin.requestTransition`;
// - sets the style of every window through `gnoblin.clients.style`, and checks that a selection made with `where`, and a selection of it,
//   each have a style of their own, and that a list made with `new ClientCollection()` has none;
// - when the library has a display for an output: reads the area windows are laid out in, shows each workspace with `show` and `showById`,
//   and tries what the checks refuse (a workspace that is not in the list, an id that is not there), and reads the workspace the display
//   under the pointer shows through `gnoblin.input.primary.activeDisplay`;
// - adds an output profile to `gnoblin.outputs.profiles`, adds a display of its own over it, swaps it for another with
//   `gnoblin.displays.replace`, and tries to replace a display of the library's, which is refused.
//
// Every line starts with `stdroot:`, spelt as the Rust and Python shells of the same example (`37-native-std-root`) spell it, so that one test
// reads all three.
import { ClientCollection, gnoblin } from "gnoblin-core/generated/objects";
import { Display, OutputProfile, Workspace } from "gnoblin-std/generated/objects";

function say(text: string): void {
    console.log(`stdroot: ${text}`);
}

const workspaces = gnoblin.workspaces;
const displays = gnoblin.displays;
const profiles = gnoblin.outputs?.profiles;
if (profiles === undefined) {
    throw new Error("the core has no outputs");
}

const main = new Workspace("main");
const side = new Workspace("side");
const ghost = new Workspace("ghost");
workspaces.add(main);
workspaces.add(side);
say(`workspaces hold ${workspaces.list.map((workspace) => workspace.id).join(", ")}`);

say(`scene metres per unit is 1: ${gnoblin.scene.metresPerUnit === 1}`);
say(`gestures swipe threshold is positive: ${gnoblin.gestures.swipeThreshold > 0}`);

let ran = false;
gnoblin.requestTransition(
    () => {
        ran = true;
    },
    { duration: 0 },
    () => {},
);
say(`requestTransition ran its work: ${ran}`);
try {
    gnoblin.requestTransition(
        () => {},
        { duration: -1 },
        () => {},
    );
    say("requestTransition refused: no");
} catch (error) {
    say(`requestTransition refused: ${(error as Error).message}`);
}

// The style of a collection of windows: every window for `gnoblin.clients`, and a scope of its own for a selection and for a selection of it.
const clients = gnoblin.clients!;
clients.style.opacity = 0.9;
say(`clients style opacity is 0.9: ${Math.abs(clients.style.opacity - 0.9) < 1e-6}`);
const foot = clients.where({ appId: "foot" });
say(`a selection has its own style: ${foot.style.path !== clients.style.path}`);
say(`a selection of a selection has its own style: ${foot.where({ title: "x" }).style.path !== foot.style.path}`);
try {
    say(`refused list style: no ${new ClientCollection().style.path}`);
} catch (error) {
    say(`refused list style: ${(error as Error).message}`);
}

/** The ids of the displays in the list, sorted and joined. */
function ids(): string {
    return displays.list
        .map((display) => display.id)
        .sort()
        .join(", ");
}

/** A display over the profile `id`, which has the id `name`. */
function over(id: string, name: string): Display {
    const display = new Display([{ profile: profiles!.byId(id) }]);
    display.id = name;
    return display;
}

let done = false;

// The first time the library has a display for an output.
function onChanged(): void {
    if (done || displays.length === 0) return;
    done = true;
    const area = workspaces.area;
    say(`the area is known: ${area !== undefined && area.width > 0 && area.height > 0}`);

    workspaces.show(side);
    say(`show side: true`);
    say(`side is active: ${side.active}`);
    workspaces.showById("main");
    say(`show main by id: true`);
    say(`main is active: ${main.active}`);
    // `find` has the kind `Workspace` once std is loaded: by field values, and by a function.
    const workspaceIds = (found: Workspace[]) => found.map((workspace) => workspace.id).join(",");
    say(`find Workspace by id: ${workspaceIds(gnoblin.find("Workspace", { id: "side" }))}`);
    say(`find Workspace shown: ${workspaceIds(gnoblin.find("Workspace", { active: true }))}`);
    say(
        `find Workspace by a function: ${workspaceIds(gnoblin.find("Workspace", (found) => found.id.startsWith("m")))}`,
    );
    const shown = gnoblin.input?.primary?.activeDisplay?.viewport?.content;
    say(`the active display shows main: ${shown?.id === "main"}`);
    try {
        workspaces.show(ghost);
        say("refused ghost: no");
    } catch (error) {
        say(`refused ghost: ${(error as Error).message}`);
    }
    try {
        workspaces.showById("nope");
        say("refused nope: no");
    } catch (error) {
        say(`refused nope: ${(error as Error).message}`);
    }

    const library = displays.list[0];
    say(`displays hold ${displays.length}`);
    const profile = new OutputProfile({ connector: library.id.replace(/^display-/, "") });
    profiles!.add(profile);
    say(`profiles hold ${profiles!.length}`);

    try {
        displays.replace(library, over(profile.id, "other"));
        say("refused library display: no");
    } catch (error) {
        say(`refused library display: ${(error as Error).message}`);
    }
    const mine = over(profile.id, "mine");
    displays.add(mine);
    say(`displays hold ${ids()}`);
    displays.replace(mine, over(profile.id, "next"));
    say(`replaced mine with next: ${ids()}`);
    say(`removed next: ${displays.remove(displays.byId("next")!)}`);
    say(`after removing, displays hold ${displays.length}`);
    say(`removed profile: ${profiles!.remove(profile)}`);
    say(`profiles hold ${profiles!.length}`);
    say("done");
}

displays.on("changed", onChanged);
say("running");
onChanged();
