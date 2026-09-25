# Developer console

Press **Alt+F2** to open the console in an unlocked Gnoblin session.
Choose JavaScript or Lua using the tabs above the prompt.

![Gnoblin's developer console open in JavaScript mode](images/gnoblin-developer-console.png)

To capture it from a disposable devkit profile, run
`scripts/capture-doc-examples.sh console`.

Disable the Alt+F2 binding in `~/.config/gnoblin/init.lua` with:

```lua
gnoblin.configure {
    shortcuts = {
        ["disable-run-dialog"] = {
            action = "gnome:shell.panel_run_dialog",
            binding = {},
        },
    },
}
```

Live edits affect the session but do not write your config file.

## Controls

| Key or command | Action                                    |
| -------------- | ----------------------------------------- |
| Enter          | Evaluate, or accept a selected suggestion |
| Shift+Enter    | Insert a newline                          |
| Up/Down, Tab   | Select and accept suggestions             |
| Ctrl+L         | Clear output, keeping variables           |
| Escape         | Dismiss suggestions, then close           |
| Alt+F2         | Close immediately                         |
| F1 or `:help`  | Show help                                 |
| `:reset`       | Reset both language contexts              |

The console closes on lock. Each language keeps its own unfinished input and state.

## Try JavaScript

```javascript
windows();
gnoblin.get();
gnoblin.set("shell.layer-duration", 350);
gnoblin.undo();
gnoblin.reload();
```

`gnoblin.get()` returns a copy of the effective config.
For several changes, modify that copy and pass it to `gnoblin.apply(document)`.

The context exposes `global`, `Main` and GNOME libraries. It supports retained
bindings and top-level `await`. Previous results are available through `r(id)`
and `$_`.

## Try Lua

```lua
gnoblin.configure {
    shell = {layer_duration = 350},
}
```

Then submit `:apply` to validate and apply the working copy.
Nothing applies merely because you type it.

Global variables remain available for later submissions. A `local` variable
is available only within the submission that creates it.
Lua uses the restricted config runtime. Module paths resolve from the active
config root.

## Undo and reload

| Command   | Result                                                        |
| --------- | ------------------------------------------------------------- |
| `:apply`  | Apply the Lua working copy                                    |
| `:undo`   | Undo a live change; up to 50 retained                         |
| `:reload` | Reload from disk and reset the Lua context                    |
| `:reset`  | Clear variables and inspection handles; keep session settings |

File reload discards live changes and undo history.
Lua keeps a separate working copy. After changing settings through JavaScript
or the config file, use `:reload` before editing in Lua again so you do not
apply an outdated copy.

Live edits support rules, animations, shortcuts and permissions.
Saved feature preferences, autostart, renderer services and startup protocols
must be changed in the file instead.

## Inspect values

Expand an object or table to inspect it. Properties load in pages of 100.
Circular references are marked; mutable values are read when expanded.

JavaScript getters run only when clicked and may change state.
Lua inspection reads raw table entries without invoking metamethods.
Reset releases retained inspection handles.

Completion and parameter help use runtime API metadata. They provide syntax
assistance, not static type checking.

## Developing the console

After Sass changes:

```sh
ninja -C build/gnome-shell data/gnome-shell-theme.gresource
meson install -C build/gnome-shell --no-rebuild
just test-developer-console
```
