// ~/.config/gnoblin/init.ts: the shell's configuration. Saving this file reloads it.
//
// As written this is what Gnoblin runs when there is no init.ts at all, so nothing changes until you
// edit it. A line that starts with "// " and then code is an example: remove the "// " to switch it on.
// Check the file with `gnoblinctl check --types init.ts`. Every key is in the defineConfig reference.
import { defineConfig, spawn } from "gnoblin";

export default defineConfig({
    // Space between windows (inner) and at the screen edge (outer), in pixels.
    gaps: { inner: 0, outer: 0 },

    // A titlebar on every window. Use false for none.
    decorations: true,

    // The workspaces: Super+1 shows the first, Super+2 the second, and so on.
    // A layout is floating, master-stack, grid, monocle, dwindle, kiosk, strip or carousel-3d.
    workspaces: [
        { id: "1", name: "Workspace 1", layout: "floating" },
        { id: "2", name: "Workspace 2", layout: "floating" },
        { id: "3", name: "Workspace 3", layout: "floating" },
        { id: "4", name: "Workspace 4", layout: "floating" },
        // { id: "work", layout: "master-stack" },
    ],

    // Your keys are added to the stock ones (workspaces, volume, media). The same trigger replaces a stock key.
    keys: {
        // "Super+Return": spawn("foot"),
        // "Super+b": spawn("firefox"),
        // "Super+q": "close",
    },

    // Window rules. Every rule that matches a window applies, the later one winning for a field.
    rules: [
        // { appId: "pavucontrol", floating: true },
    ],

    // Commands started once per session. A reload does not start them again.
    autostart: [
        // "waybar",
    ],

    // The escape hatch, for what defineConfig has no key for. It runs once, after the config is applied.
    // setup(gnoblin) {
    //     gnoblin.after(5, () => gnoblin.spawn(["notify-send", "Gnoblin", "Five seconds in."]));
    // },
});
