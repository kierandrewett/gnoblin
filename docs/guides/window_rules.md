# window_rules

[Configuration API](/config)

A rule has two parts: `match` selects the windows, and the other fields change
what happens to them. Add rules to `~/.config/gnoblin/init.lua` after any
`gnoblin.load(...)` lines. Put rules for all windows before rules for specific ones.

## Add a rule

This places every editor window on the `code` workspace and maximises it:

```lua
gnoblin.window_rule {
    match = {type = "window", app_id = "^org%.example%.Editor$"},
    workspace = {id = "code"},
    maximise = true,
}
```

Save and run `gnoblinctl config reload`. The rule applies to windows that open
after the reload. Windows that are already open keep their current placement.

## Choose what to match

Every condition in `match` must match. For example,
`{type = "window", focused = false}` selects unfocused application windows.
If `focused` is omitted, the rule can match both focused and unfocused windows.

| Key                         | Matches                                                  |
| --------------------------- | -------------------------------------------------------- |
| `type = "window"`           | Application windows                                      |
| `focused = true` or `false` | Focus state                                              |
| `app_id`                    | GTK application ID, or WM class when the GTK ID is empty |
| `title`                     | Current window title                                     |
| `workspace_id`              | Stable ID of the current workspace                       |
| `workspace_number`          | Current one-based workspace position, 1–1024             |
| `client_type`               | `"wayland"`, or `"x11"` for an Xwayland client           |

Choose the match field based on what should identify the window:

- Use `app_id` to match an application across its windows.
- Use `title` to target a particular document or window. Titles often change
  as the document or page changes.

Layer-shell surfaces such as bars and launchers cannot be matched yet. A rule
that sets `match.layer` is refused at load.

## Find the values

List the live windows with:

```sh
gnoblinctl window list
```

Use the `app_id` and `title` of the window you want as the values to match.
The `app_id` matches the GTK application ID, or the WM class when the GTK ID is
empty, so it can differ from the desktop-entry name shown elsewhere.

## Write a matcher

The string fields `app_id` and `title` accept **Lua 5.4 patterns**. They are
case-sensitive and search anywhere by default. Use `^` and `$` to match the
whole value.

A dot (`.`) matches any character; write `%.` to match a literal dot. See the
[Lua pattern reference](https://www.lua.org/manual/5.4/manual.html#6.4.1) for
other pattern characters. JavaScript and PCRE regex syntax is not supported.

To match `org.gnome.Nautilus` only:

```lua
gnoblin.window_rule {
    match = {type = "window", app_id = [[^org%.gnome%.Nautilus$]]},
    maximise = true,
}
```

In Lua's `[[...]]` string, percent signs are kept as written. The `%` before
each dot makes it a literal dot in the pattern.

To match a title containing `Notes` anywhere, omit the anchors:

```lua
gnoblin.window_rule {
    match = {type = "window", title = "Notes"},
    maximise = false,
}
```

This also matches titles such as `Notes — Work`. To match only the exact title
`Notes`, write `title = "^Notes$"`.

When a rule has more than one condition, all of them must match. This targets
only unfocused windows from one app:

```lua
gnoblin.window_rule {
    match = {
        type = "window",
        app_id = [[^org%.gnome%.Nautilus$]],
        focused = false,
    },
    maximise = false,
}
```

## Workspaces {#workspaces}

Use a workspace ID when a rule should keep targeting the same workspace as
workspace positions change. Declare persistent workspaces in the `workspaces`
field of `gnoblin.configure`:

```lua
gnoblin.configure {
    workspaces = {
        {id = "code", name = "Code"},
        {id = "web", name = "Web"},
        {id = "chat", name = "Chat"},
    },
}
```

Match a window's current workspace by ID or by its current one-based position:

```lua
gnoblin.window_rule {
    match = {type = "window", workspace_id = "code"},
    maximise = true,
}

gnoblin.window_rule {
    match = {type = "window", workspace_number = 2},
    maximise = false,
}
```

Workspace matches update when a window moves. Numbers can change as
workspaces are created, removed, or reordered, so use IDs for rules that need
a lasting target.

To place a window on a workspace, add a `workspace` field to its rule:

```lua
gnoblin.window_rule {
    match = {type = "window", app_id = [[^org%.example%.Editor$]]},
    workspace = {id = "code"},
}
```

Use `workspace = {number = 2}` to target a position instead; numbers range from
1 to 1024.

An `{id = ...}` target that names a declared workspace resolves to that
workspace's position when the configuration is committed. The id is then fixed
until the configuration is committed again.

A name that is not declared, and is not a number, is refused at commit with the
field named; the previous accepted rules stay in force. A number is kept as the
workspace id it was written as. If no such workspace exists, the window stays
where it is and a warning is logged.

## When rules run again

A rule runs when a window opens. It runs again when that window's title or
app ID changes. The fields apply again only when the set of matching rules
changes, so a title change that keeps the same matches does nothing.

A re-match can move a window, for example when a title change makes a rule
with `workspace` start to match. A window that loses its match keeps the
placement or maximised state the rule gave it.

A configuration reload does not re-run rules on windows that are already open.

## Rule order

Matching does not stop at the first rule. Later rules override only the fields
they set. For example, a rule that sets `maximise` for every window can be
followed by a rule for one application that sets `maximise = false`.

Use `gnoblin.window_rule` to add rules without removing earlier ones. Passing
a complete `window_rules` list to `gnoblin.configure` replaces the configured list;
see [load order](/guides/files_and_load_order#override-or-append). A list holds at
most 256 rules.

## What else is planned

The effect keys from earlier versions are refused in this build:

- `blur`, `blur-ignore-shadows`, `opacity`, `corners`
- `shader`, `shader-uniforms`, `animation` and `frame`

The planned replacement is an ordered `effects` list. See the
[window rule reference](/config/window_rule#refused-keys) for the migration note.
