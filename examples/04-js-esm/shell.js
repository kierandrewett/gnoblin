// Example 4: a JavaScript shell written as ES modules.
//
// `gnoblin-core` is the compositor's own objects (windows, outputs, input); `gnoblin-std` is the optional desktop library
// (workspaces and layouts). A relative import reads a file of this directory. Run it with:
//
//     gnoblin ./shell.js
//
// The shell reloads when this file or a module it imports changes.
import { gnoblin } from "gnoblin-core";
import { MasterStackLayout, Workspace } from "gnoblin-std";

import { bindLaunchers } from "./keys.js";

// One workspace that tiles its windows with the stock master-stack layout.
gnoblin.workspaces.add(new Workspace("main", { name: "Main", layout: new MasterStackLayout() }));

// The key bindings live in a module of their own.
bindLaunchers(gnoblin.input.primary);
