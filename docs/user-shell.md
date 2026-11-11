# Shell files

[Configuration API](/config)

The stock shell places, focuses and arranges windows. Your own Lua files change
how it does that. They load when the session starts, after the stock shell and
before the first frame, so a file can set what the shell reads and replace what
it does.

## Where the files go

Put them in the directory that holds your `init.lua`, normally
`~/.config/gnoblin`:

- `shell.lua` loads first.
- `shell/*.lua` load next, in file-name order. Prefix names with numbers
  (`10-rules.lua`, `20-sets.lua`) to control the order.

Neither is needed. A missing file or directory loads nothing. Restart the
session to load changes; a configuration reload does not run these files again.

`init.lua` is the configuration file and still holds settings such as
`gnoblin.configure`. The shell files hold behaviour.

## What a file can see

A file runs in the shell's Lua state. It sees the shell's globals (`root`,
`seat`, `desk()`, `spawn`, the node classes such as `Window` and `Desk`, and
`report`), and a global it defines is the shell's global.

A file cannot reach the host. This is the boundary of the configuration
sandbox:

- A file has no `io`, `debug`, `package`, `dofile` or `loadfile`.
- `load` reads text and never bytecode.
- `os` has only `time`, `date`, `clock` and `difftime`.

Use `spawn` to start a program.

`require("gnoblin.shell.rules")` returns the shipped module, the same one the
stock shell uses. Any other name, such as `require("mine.rules")`, loads
`mine/rules.lua` from the configuration directory. Names with `..` or a path
separator are refused.

A file that runs for more than two seconds is stopped. The same limits hold for
the whole shell, not only for the file: it runs in one sandboxed state, so the
stock shell's own code cannot reach the host either.

A handler, binding or timer has its own limit of 10 million Lua instructions or
100 milliseconds, whichever comes first.

- One that goes past it is stopped and reported by name, for example
  `Window on_ready handler: ... instruction budget ... is exhausted`. The other
  handlers of the frame still run.
- One that raises is reported the same way (`Workspace:arrange: ...`). Its node
  keeps what it was last given.
- The same report is logged once every five seconds, with a count of the copies
  it held back.

## Window rules

Assign a list to `seat.data.window_rules`. A rule is a `match` table and the
fields to set on every window that matches:

```lua
-- ~/.config/gnoblin/shell.lua
seat.data.window_rules = {
    -- Maximise editors, and open terminals on workspace 3.
    { match = { app_id = "^org%.example%.Editor$" }, maximised = true },
    { match = { app_id = "^org%.example%.Term$" }, workspace = { id = "3" } },
    -- Hide a banking window from screen capture, and dim a media player.
    { match = { title = "^Bank" }, secure = true },
    { match = { app_id = "mpv" }, opacity = 0.8 },
}
```

Every key in `match` must match.

| `match` key    | Matches                                                                    |
| -------------- | -------------------------------------------------------------------------- |
| `app_id`       | Lua pattern on the application ID, or on the WM class when the ID is empty |
| `title`        | Lua pattern on the current title                                           |
| `focused`      | `true` or `false`                                                          |
| `workspace_id` | The name of the window's workspace, such as `"2"`                          |

Patterns are [Lua patterns](https://www.lua.org/manual/5.4/manual.html#6.4.1),
case-sensitive, and match anywhere in the text. Use `^` and `$` to match the
whole value. Write `%.` for a literal dot.

| Field        | Effect                                                                    |
| ------------ | ------------------------------------------------------------------------- |
| `workspace`  | `{ id = "3" }`: move the window to the workspace of that name on its desk |
| `maximised`  | `true` maximises the window, `false` unmaximises it                       |
| `fullscreen` | `true` or `false`                                                         |
| `secure`     | `true` masks the window in screen capture                                 |
| `opacity`    | A number from 0 to 1                                                      |

The stock desk has nine workspaces, named `"1"` to `"9"`.

Rules merge in list order. A later rule overrides only the fields it sets, so a
rule for every window can be followed by a rule for one app that undoes one
field. A field that no rule sets falls back to its default: a window
whose title stops matching a `secure` rule is not secure.

A rule runs when a window opens, and again when its title, app ID, focus or
workspace changes the set of rules that match. A rule does not move a window
that is already open.

### Mistakes

The shell compiles a list once, when it first reads it.

- A list that does not compile is refused whole. Causes are a malformed
  pattern, an unknown match key or field, an opacity outside 0 to 1, `corners`,
  and a field of the wrong type. The shell logs one error and the rules in force
  stay. A first list that fails leaves no rules.
- To change the rules, assign a new table. The shell does not see edits to the
  table in place.
- A rule whose `workspace` names a workspace the desk does not have logs one
  error for each match, and the window stays where it is.

## Workspace sets

A set shows named workspaces on named outputs together. Assign the list to
`seat.data.workspace_sets` and activate a set by name:

```lua
-- ~/.config/gnoblin/shell.lua
seat.data.workspace_sets = {
    { name = "work", entries = { { workspace = "1", output = "DP-1" }, { workspace = "2", output = "HDMI-A-1" } } },
    { name = "home", entries = { { workspace = "3" } } },
}

seat:bind({
    ["Super+F1"] = function() activate_set("work") end,
    ["Super+F2"] = function() activate_set("home") end,
})
```

An entry names a workspace (the desk's workspace names, `"1"` to `"9"`) and the
output it shows on, by connector name. An entry with no `output` shows on the
primary output. If an output is not connected, its workspace shows on the
primary output and one error is logged. Activating the set that is already live
changes nothing.

A list is refused whole, with one logged error and the previous sets kept, when
a set has an empty or repeated name, a set has no entries, an entry names a
workspace no desk has, or two entries in one set name the same output.
`activate_set` returns `false`, and logs the reason, for an unknown set.

## When a file fails

A file with a syntax error, or one that raises, logs one error that names the
file. What it set before it raised stays. The stock shell is already running and
the files after it still load. The shell runs in the runtime role, so read the
error there:

```sh
gnoblinctl log --role runtime --level error
```
