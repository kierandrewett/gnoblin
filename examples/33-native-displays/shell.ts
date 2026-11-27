// Example 33 in TypeScript: output profiles, displays and viewports of libgnoblin-std, through the classes generated from its catalog.
//
//     gnoblin ./shell.ts
//
// The library keeps one `single` display for every output, so the list of displays says what each monitor shows. This shell:
//
// - makes two workspaces and shows the second one on the display of the output, through the display's viewport;
// - adds an output profile for that output (a scale), and hears the compositor take it (`applied`);
// - adds a display of its own over the profile, which takes the output from the display of the library, and removes it again, which gives
//   the output a display of the library's again;
// - tries what the checks refuse: a scale out of range, a display with nothing to present, a second display over an output that has one.
//
// Every line starts with `displays:`, spelt as the Rust shell of the same example (`33-native-displays`) spells it, so that one test reads
// all three.
import { Display, DisplayManager, OutputProfile, Viewport, Workspace, Workspaces } from "gnoblin-std";
import type {} from "gnoblin-std";

function say(text: string): void {
    console.log(`displays: ${text}`);
}

const workspaces = Workspaces.get();
const one = new Workspace("one");
const two = new Workspace("two");
workspaces.add(one);
workspaces.add(two);
const manager = DisplayManager.get();

/** The displays as `id:mode:outputs:workspace` words, sorted. */
function listed(): string {
    return manager.displays
        .map(
            (display) =>
                `${display.id}:${display.mode}:${display.outputs.length}:${display.viewport.content?.id ?? "none"}`,
        )
        .sort()
        .join(" ");
}

let phase = 0;
let held: OutputProfile | undefined;

// The first time the library has a display for an output: show the second workspace on it and add the profile.
function onChanged(): void {
    if (phase !== 0 || manager.displays.length === 0) return;
    phase = 1;
    say(`listed ${listed()}`);
    const display = manager.displays[0];
    display.viewport.content = two;
    say(`the display ${display.id} shows ${display.viewport.content?.id}`);
    const connector = display.id.replace(/^display-/, "");
    const profile = new OutputProfile({ connector });
    profile.settings = { mode: "preferred", scale: 1.25 };
    manager.addProfile(profile);
    held = profile;
    say(`the profile ${profile.id} is in the manager: ${profile.listed}`);
}

// The compositor answered the profile's request: use the profile in a display of this shell's own.
function onApplied(_output: number, error: string): void {
    if (phase !== 1 || held === undefined) return;
    phase = 2;
    say(`the compositor answered the request: ${error === "" ? "ok" : error}`);
    const profile = held;
    const settings = profile.settings;
    say(`the profile reads back scale ${settings.scale} mode ${settings.mode}`);

    const mine = new Display([{ profile }]);
    mine.id = "main";
    const viewport = new Viewport();
    viewport.content = one;
    mine.viewport = viewport;
    manager.addDisplay(mine);
    say(`listed ${listed()}`);

    const rival = new Display([{ profile }]);
    rival.id = "rival";
    try {
        manager.addDisplay(rival);
        say("refused overlap: no");
    } catch (error) {
        say(`refused overlap: ${(error as Error).message}`);
    }

    say(`removed main: ${manager.removeDisplay(mine)}`);
    say(`listed ${listed()}`);

    try {
        new OutputProfile({ connector: "HEADLESS-1" }).settings = { scale: 9 };
        say("refused scale: no");
    } catch (error) {
        say(`refused scale: ${(error as Error).message}`);
    }
    try {
        new Display([]);
        say("refused selectors: no");
    } catch (error) {
        say(`refused selectors: ${(error as Error).message}`);
    }
    say("done");
}

manager.on("changed", onChanged);
manager.on("applied", onApplied);
say("running");
onChanged();
