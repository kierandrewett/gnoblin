# Complete starter config

Run `gnoblinctl config init` to write the stock default as `~/.config/gnoblin/shell.ts`. The file is the whole configuration:
the shell runs it from top to bottom, and with no `shell.ts` it runs this same file. Nothing is layered under your file,
so keep the lines you want to keep.

The file declares four floating workspaces, starts the titlebar, window menu and not-responding alert, and binds the workspace
keys, the strip focus keys, the three-finger swipe, the launcher key and the media keys, each as a call on a live object. It
is this, in full:

```ts
// The shell reloads `~/.config/gnoblin/shell.ts` when you save it.
// It runs top to bottom against live objects such as `gnoblin.workspaces`, `gnoblin.input` and `gnoblin.clients`.
// Nothing is layered under it, so these statements define the desktop; uncomment examples to enable them.
// A shell with no `shell.ts` runs this same file, so a copy you leave unchanged behaves like the default.
// Check changes with `gnoblinctl check --types shell.ts`; see the shell API pages for each object's details.

import {
    Decorations,
    FloatingLayout,
    Launcher,
    MoveBinding,
    NotRespondingAlerts,
    ResizeBinding,
    StripLayout,
    WindowMenus,
    Workspace,
    WorkspaceSlideBinding,
} from "gnoblin-std";
// To use the examples below, add what they name to the import above.
// import { MasterStackLayout } from "gnoblin-std";

// Four floating workspaces. The first is shown until another is selected.
// Choose a layout: FloatingLayout, MasterStackLayout, GridLayout, MonocleLayout, DwindleLayout or StripLayout.
for (const id of ["1", "2", "3", "4"]) {
    gnoblin.workspaces.add(new Workspace(id, { name: `Workspace ${id}`, layout: new FloatingLayout() }));
}

// gnoblin.workspaces.add(new Workspace("work", { name: "Work", layout: new MasterStackLayout() }));

// Add a titlebar to each window, the window menu and the not-responding alert.
// Remove a call to disable that part.
Decorations.get().start();
WindowMenus.get().start();
NotRespondingAlerts.get().start();

const input = gnoblin.input.primary;

// Super+1 .. Super+9 show the workspace at that position on the display under the pointer. A key past the last
// workspace does nothing.
for (let position = 1; position <= 9; position++) {
    input.bind(`Super+${position}`, () => {
        const target = gnoblin.workspaces.list[position - 1];
        if (target !== undefined) {
            gnoblin.workspaces.show(target, { duration: 250 });
        }
    });
}

// Super+Left and Super+Right move focus between StripLayout columns on the display under the pointer. They do nothing
// on other layouts.
const moveColumn = (side: "focusLeft" | "focusRight") => {
    const layout = input.activeDisplay?.viewport?.content?.layout;
    if (layout instanceof StripLayout) {
        layout[side]();
    }
};
input.bind("Super+Left", () => moveColumn("focusLeft"));
input.bind("Super+Right", () => moveColumn("focusRight"));

// Three-finger sideways swipes switch workspaces.
new WorkspaceSlideBinding({ trigger: "Swipe3" }).install();

// Super with the left button drags a window, and Super with the right button resizes it from the nearest corner.
new MoveBinding().install();
new ResizeBinding().install();

// Volume, microphone and media player keys: each key runs one command.
const mediaKeys: Record<string, string[]> = {
    XF86AudioRaiseVolume: ["wpctl", "set-volume", "@DEFAULT_AUDIO_SINK@", "5%+"],
    XF86AudioLowerVolume: ["wpctl", "set-volume", "@DEFAULT_AUDIO_SINK@", "5%-"],
    XF86AudioMute: ["wpctl", "set-mute", "@DEFAULT_AUDIO_SINK@", "toggle"],
    XF86AudioMicMute: ["wpctl", "set-mute", "@DEFAULT_AUDIO_SOURCE@", "toggle"],
    XF86AudioPlay: ["playerctl", "play-pause"],
    XF86AudioNext: ["playerctl", "next"],
    XF86AudioPrev: ["playerctl", "previous"],
};
for (const [key, command] of Object.entries(mediaKeys)) {
    input.bind(key, () => {
        gnoblin.spawn(command);
    });
}

// Super+space opens the launcher, with a command entry in the middle of the screen.
input.bind("Super+space", () => {
    Launcher.open();
});

// Add custom key bindings here. A key already bound above is refused; remove that binding first.
// input.bind("Super+Return", () => { gnoblin.spawn(["foot"]); });
// input.bind("Super+q", () => { input.focusedClient?.close(); });

// Set a style property to change how each window looks.
// gnoblin.clients.style.cornerRadius = 12;
```

To add a terminal shortcut, add one line after the stock keys; replace `foot` with your terminal.

```ts
gnoblin.input.primary.bind("Super+Return", () => {
    gnoblin.spawn(["foot"]);
});
```

Save the file and the running shell reloads it. A key is bound once at one priority, so a second `bind` of a trigger the
file already binds is refused: delete the stock line first.

To dim unfocused windows, see [dim unfocused windows](/recipes/dim-unfocused-windows). The file does not install or start a
desktop shell; choose one in [shell setup](/bring-your-own-shell).
