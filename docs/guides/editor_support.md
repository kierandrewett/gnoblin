# Editor Support

Get completion, argument hints and type checking for `gnoblin.*` in your editor.

Gnoblin publishes a type definition file for [Lua Language Server](https://luals.github.io/),
which VS Code, Neovim, Helix, Zed and most other editors can run. The file describes every
function in the Lua API: its parameters and their types, the keys a table argument accepts, which
are optional, and what the function returns.

## Set it up

1. Download [gnoblin.d.lua](https://gnoblin.org/gnoblin.d.lua) to a folder, for example
   `~/.config/gnoblin/types/`.
2. Create `.luarc.json` in `~/.config/gnoblin/` and point Lua Language Server at that folder:

```json
{
    "workspace.library": ["types"],
    "runtime.version": "Lua 5.4",
    "diagnostics.globals": ["gnoblin"]
}
```

3. Open `init.lua`. Typing `gnoblin.workspaces.` now lists the functions with their
   signatures, and hovering a call shows its description.

The file matches the Gnoblin release it was published with. Download it again after you update.

## The same hints in the console

The [graphical console](/cli/graphical-console) reads the same definitions. Completion lists show
each function's signature, and typing a call such as `gnoblin.workspaces.create(` shows its
parameters, with the keys of a table argument listed and completed as you type them.
