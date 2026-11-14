// ~/.config/gnoblin/init.ts: your Gnoblin configuration. Save the file and the shell reloads it.
//
// This file is the whole configuration. Gnoblin runs it from top to bottom against live objects, such as
// `gnoblin.workspaces`, `gnoblin.input` and `gnoblin.clients`. There is no config object and nothing is layered under
// it, so what is below is exactly what Gnoblin does with no init.ts at all. Delete a statement and that part of the
// desktop is gone. Add one and it is there after the save.
//
// A line that starts with "// " and then code is an example: delete the "// " to switch it on.
// To check your edits before you save them, run `gnoblinctl check --types init.ts`.
// Every object is described in the shell API pages of the documentation.
import {
    FloatingLayout,
    StripLayout,
    Workspace,
    WorkspaceSlideBinding,
    openLauncher,
    startDecorations,
    startNotRespondingAlert,
    startWindowMenu,
} from "gnoblin";
// To use the examples below, add what they name to the import above.
// import { MasterStackLayout } from "gnoblin";

// Four floating workspaces. The first is shown until a key shows another.
// A layout is an object: FloatingLayout, MasterStackLayout, GridLayout, MonocleLayout, DwindleLayout, KioskLayout or
// StripLayout.
for (const id of ["1", "2", "3", "4"]) {
    gnoblin.workspaces.add(new Workspace({ id, name: `Workspace ${id}`, layout: new FloatingLayout() }));
}
// gnoblin.workspaces.add(new Workspace({ id: "work", name: "Work", layout: new MasterStackLayout() }));

// A titlebar on every window, the menu the compositor asks for, and the dialog for a window that stops answering.
// Leave a line out and that part is not there.
startDecorations();
startWindowMenu();
startNotRespondingAlert();

const input = gnoblin.input.primary;

// Super+1 .. Super+9 show the workspace at that position on the display the pointer is on. A key past the last
// workspace does nothing.
for (let position = 1; position <= 9; position++) {
    input.bind(`Super+${position}`, () => {
        const target = gnoblin.workspaces.at(position - 1);
        if (target !== undefined) {
            gnoblin.workspaces.show(target, { duration: 0.25 });
        }
    });
}

// Super+Left and Super+Right move the focus between the columns of a StripLayout, and do nothing on other layouts.
const moveColumn = (side: "focusLeft" | "focusRight") => {
    const shown = (input.activeDisplay as { viewport: { content: Workspace | null } } | null)?.viewport.content;
    if (shown?.layout instanceof StripLayout) {
        shown.layout[side](shown);
    }
};
input.bind("Super+Left", () => moveColumn("focusLeft"));
input.bind("Super+Right", () => moveColumn("focusRight"));

// Three fingers sliding sideways slide between workspaces.
input.bind("Swipe3", new WorkspaceSlideBinding());

// Volume, microphone and player keys.
const run = (command: string[]) => {
    gnoblin.spawn(command);
};
input.bind("XF86AudioRaiseVolume", () => run(["wpctl", "set-volume", "@DEFAULT_AUDIO_SINK@", "5%+"]));
input.bind("XF86AudioLowerVolume", () => run(["wpctl", "set-volume", "@DEFAULT_AUDIO_SINK@", "5%-"]));
input.bind("XF86AudioMute", () => run(["wpctl", "set-mute", "@DEFAULT_AUDIO_SINK@", "toggle"]));
input.bind("XF86AudioMicMute", () => run(["wpctl", "set-mute", "@DEFAULT_AUDIO_SOURCE@", "toggle"]));
input.bind("XF86AudioPlay", () => run(["playerctl", "play-pause"]));
input.bind("XF86AudioNext", () => run(["playerctl", "next"]));
input.bind("XF86AudioPrev", () => run(["playerctl", "previous"]));

// Super+space opens the launcher: a command entry in the middle of the screen.
input.bind("Super+space", () => {
    openLauncher();
});

// Your own keys. Binding a key that is bound above is refused, so delete that line first.
// input.bind("Super+Return", () => { gnoblin.spawn(["foot"]); });
// input.bind("Super+q", () => { input.focusedClient?.close(); });

// How every window looks. A style is a property you set, and the shell draws it.
// gnoblin.clients.style.cornerRadius = 12;
