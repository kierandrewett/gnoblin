// ~/.config/gnoblin/init.ts: your Gnoblin configuration. Save the file and the shell reloads it.
//
// As written, this file does exactly what Gnoblin does with no init.ts at all, so nothing changes until you
// edit it. A line that starts with "// " and then code is an example: delete the "// " to switch it on.
// To check your edits before you save them, run `gnoblinctl check --types init.ts`.
// Every key below is listed, with its type and an example, in the defineConfig reference.
// `spawn` is imported for the key examples below. Remove it if you do not use it.
import { defineConfig, spawn } from "gnoblin";

export default defineConfig({
    // Space between windows (inner) and between windows and the screen edge (outer), in pixels.
    gaps: { inner: 0, outer: 0 },

    // A titlebar on every window. Use false for none.
    decorations: true,

    // Your workspaces. Super+1 shows the first one, Super+2 the second, and so on.
    // A layout is floating, master-stack, grid, monocle, dwindle, kiosk, strip or carousel-3d.
    workspaces: [
        { id: "1", name: "Workspace 1", layout: "floating" },
        { id: "2", name: "Workspace 2", layout: "floating" },
        { id: "3", name: "Workspace 3", layout: "floating" },
        { id: "4", name: "Workspace 4", layout: "floating" },
        // { id: "work", layout: "master-stack" },
    ],

    // Your keys are added to the stock ones (workspaces, volume, media keys).
    // If you bind a key the stock config already binds, yours wins.
    keys: {
        // "Super+Return": spawn("foot"),
        // "Super+b": spawn("firefox"),
        // "Super+q": "close",
    },

    // Window rules. Every rule that matches a window applies. If two rules set the same field, the later rule wins.
    rules: [
        // { appId: "pavucontrol", floating: true },
    ],

    // Commands to start once per session. A reload does not start them again.
    autostart: [
        // "waybar",
    ],

    // The escape hatch, for what defineConfig has no key for. It runs once, after the config is applied.
    // The first number is seconds.
    // setup(gnoblin) {
    //     gnoblin.after(5, () => gnoblin.spawn(["notify-send", "Gnoblin", "Five seconds in."]));
    // },
});
