# Build your shell

The stock shell is ordinary Lua running on the [shell object model](shell-object-model.md).
You can change it in three levels. Start at the first and go down only when a level
cannot express what you want.

| Level | You write                                             | You change                                            |
| ----- | ----------------------------------------------------- | ----------------------------------------------------- |
| 1     | Settings in `shell.lua`                               | Window rules and workspace sets                       |
| 2     | A copy of a shipped module, required from `shell.lua` | One class or method the stock shell uses              |
| 3     | Your own classes in `shell/*.lua`                     | Layouts, handlers and behaviour the stock shell lacks |

Your files live in `~/.config/gnoblin`, as described in [shell files](user-shell.md).
They load after the stock shell, so they can replace what it defines. Restart the
session to load changes; a configuration reload does not run these files again.

## Level 1: settings

Most changes are data. Assign a table to the seat, as shown in
[shell files](user-shell.md#window-rules):

```lua
-- ~/.config/gnoblin/shell.lua
seat.data.window_rules = {
    { match = { app_id = "^org%.example%.Editor$" }, maximised = true },
}

seat.data.workspace_sets = {
    { name = "work", entries = { { workspace = "1" }, { workspace = "2" } } },
}
```

Use this level first. Window rules and workspace sets are read by the shell, so
they keep working when the stock shell changes its internals.

## Level 2: replace one method

A shipped module is a class or a set of classes. The stock desk builds each
workspace with `Desk:workspace(name)`, a method on the `Desk` class. Replacing that
method changes which layout every new workspace uses.

The stock layouts are in `gnoblin/shell/tiling.lua`. Copy that file from the
Gnoblin source tree (`data/lua/gnoblin/shell/tiling.lua`) to
`~/.config/gnoblin/mine/tiling.lua`. Edit your copy freely; the shipped file is
not touched.

Then require your copy from `shell.lua` and replace the method:

```lua
-- ~/.config/gnoblin/shell.lua
require("mine.tiling")

function Desk:workspace(name)
    return Tiling({ name = name })
end
```

`shell.lua` runs after the stock shell has defined `Desk`, so the override replaces
the stock method.

The `require` line reads `mine/tiling.lua` from your configuration directory. The
classes it defines (Tiling, Monocle and the rest) become globals you can use.

`Desk:build` calls `self:workspace(id)` for each declared workspace, so the override
applies to the workspaces the desk builds after your file loads. Check it after a
restart: a workspace's class name is its selector, so `ws:is("Tiling")` is true for
a workspace built by your override.

## Level 3: your own layout

A layout is a class that extends `Workspace`. Its `arrange` method places the
windows among its `"> Window"` children. A class with its own `arrange` is bound
to the object model, so `arrange` re-runs when the values it reads change, such as
the workspace's children and their sizes. This is the pattern the stock tiled
layouts use.

Put the class in its own file under `shell/`. The shell reads `shell.lua` first,
then `shell/*.lua` in file-name order, so number the file to control when it loads:

```lua
-- ~/.config/gnoblin/shell/10-cascade.lua
Workspace:extend("Cascade")({ step = 40 })

function Cascade:arrange()
    local wins = self:all("> Window")
    for i, w in ipairs(wins) do
        w.x, w.y = (i - 1) * self.step, (i - 1) * self.step
        w.width, w.height = self.width * 0.6, self.height * 0.6
    end
end
```

Use it for new workspaces by overriding the method from level 2, in `shell.lua`:

```lua
-- ~/.config/gnoblin/shell.lua
function Desk:workspace(name)
    return Cascade({ name = name })
end
```

Give a workspace a different `step` by passing it in the constructor:
`Cascade({ name = name, step = 60 })`.

## When a file fails

A file that raises an error logs one error that names the file. The rest of the
shell keeps running and the files after it still load. Read the error with:

```sh
gnoblinctl log --role runtime --level error
```

To start a program from a shell file, call the global `spawn`, which the stock shell
defines from the system's `spawn`. The [Lua runtime API](shell-api/index.md) lists
the rest of what a shell file can call.
