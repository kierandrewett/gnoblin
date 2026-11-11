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

The whole shell runs in one sandboxed state, so the stock shell's own code cannot
reach the host either.

## Time limits

Every entry into Lua has two bounds, and the first one reached stops it:

- **A count of Lua instructions.** This is the real limit. It is the same on a
  busy machine and an idle one, so a slow machine never fails a script with it,
  and it stops a loop that never ends in a fraction of a second.
- **A wall clock.** This is the backstop for what a count cannot see.

| Entry                                                                  | Instructions | Wall clock | When it goes past                                    |
| ---------------------------------------------------------------------- | ------------ | ---------- | ---------------------------------------------------- |
| Starting the shell: the stock shell, and each of your files on its own | 100 million  | 10 seconds | The file is stopped and reported; the next one loads |
| One handler, binding or timer                                          | 10 million   | 100 ms     | It is stopped and reported; the frame goes on        |
| One frame, all its handlers together                                   | 100 million  | 500 ms     | The shell VM ends                                    |

The start has the wide budget because it runs once, before the first frame, and
a loaded machine can be slow without the script being wrong:

- The stock shell needs about 20 thousand instructions and 10 to 20
  milliseconds on an idle machine, so a file has thousands of times that.
- Each file gets the whole start budget to itself. A slow file does not take
  time from the next.
- A process that is descheduled loses wall-clock time and no instructions, so a
  start fails on the clock only after ten seconds of it.
- The frame and handler clocks stay tight, because a frame that late is one you
  would see.

What happens when an entry goes past its limit:

- It is stopped and reported by name, for example
  `Window on_ready handler: ... instruction budget ... is exhausted`, or, for a
  file that never ends, `.../shell/20-loop.lua: ... instruction budget ...`. The
  other handlers of the frame, and the other files, still run.
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

The stock desk builds the workspaces your configuration declares in `workspaces`, named by their `id`. With none
declared it has nine, named `"1"` to `"9"`.

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

An entry names a workspace (the desk's workspace names; with no declared `workspaces` these are `"1"` to `"9"`) and the
output it shows on, by connector name. An entry with no `output` shows on the
primary output. If an output is not connected, its workspace shows on the
primary output and one error is logged. Activating the set that is already live
changes nothing.

A list is refused whole, with one logged error and the previous sets kept, when
a set has an empty or repeated name, a set has no entries, an entry names a
workspace no desk has, or two entries in one set name the same output.
`activate_set` returns `false`, and logs the reason, for an unknown set.

## When a file fails

A file with a syntax error, one that raises, or one that runs past its start
budget (a loop that never ends) logs one error that names the file. What it set
before it stopped stays. The stock shell is already running and the files after
it still load.

The shell runs in the runtime role, so read the error there:

```sh
gnoblinctl log --role runtime --level error
```

## Moving from `window-rules` and `workspace-sets`

These are gone, and a document that sets `window-rules` or `workspace-sets` is refused with an error that names this
page:

- The `window-rules` and `workspace-sets` settings of `gnoblin.configure`.
- `gnoblin.window_rule`, which raises an error that names `seat.data.window_rules`.
- `gnoblin.workspaces.activate_set` and `gnoblinctl workspace activate-set`. Call `activate_set(name)` from a binding
  in a shell file.

| Old                                                       | New                                                     |
| --------------------------------------------------------- | ------------------------------------------------------- |
| `gnoblin.window_rule { ... }` or `window-rules = { ... }` | An entry in `seat.data.window_rules`                    |
| `match = { type = "window", ... }`                        | Drop `type`: every rule is for windows                  |
| `match["app-id"]`, `match["workspace-id"]`                | `match.app_id`, `match.workspace_id`                    |
| `match.title`, `match.focused`                            | The same keys                                           |
| `match.workspace_number`, `match.client_type`             | No equivalent. Match `workspace_id` instead             |
| `workspace = { id = "code" }`                             | The same, naming a declared workspace                   |
| `workspace = { number = 2 }`                              | `workspace = { id = "2" }`                              |
| `maximise = true`                                         | `maximised = true`                                      |
| `fullscreen`, `secure`, `opacity`                         | The same fields                                         |
| `corners = { radius = 8 }`                                | Removed. The stock shell draws blur and shadow only     |
| `workspace-sets = { { name, workspaces = { ... } } }`     | An entry in `seat.data.workspace_sets`, with `entries`  |
| `gnoblinctl workspace activate-set NAME`                  | `activate_set(NAME)` from a `seat:bind` in a shell file |

For example, this configuration:

```lua
gnoblin.window_rule {
    match = { type = "window", ["app-id"] = [[^org%.example%.Editor$]] },
    workspace = { number = 2 },
    maximise = true,
}
```

becomes this shell file:

```lua
-- ~/.config/gnoblin/shell.lua
seat.data.window_rules = {
    { match = { app_id = [[^org%.example%.Editor$]] }, workspace = { id = "2" }, maximised = true },
}
```

A `workspace-sets` entry such as `{ name = "work", workspaces = { { workspace = "1", output = "DP-1" } } }` becomes
`{ name = "work", entries = { { workspace = "1", output = "DP-1" } } }`.
