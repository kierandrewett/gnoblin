# window_rules

[Shell files](/user-shell)

A window rule changes what the stock shell does to the windows it matches. Rules are a Lua table in your shell files,
not a configuration key. They load with the shell when the session starts.

## Add a rule

Put this in `~/.config/gnoblin/shell.lua`. It places every editor window on workspace `3` and maximises it:

```lua
seat.data.window_rules = {
    {
        match = { app_id = "^org%.example%.Editor$" },
        workspace = { id = "3" },
        maximised = true,
    },
}
```

Restart the session to apply it. A rule runs when a window opens, so it does not move windows that are already open.

The [window rule reference](/config/window_rule) lists every match key and field. [Shell files](/user-shell) explains
where the files go and what they can reach.

## Choose what to match

Every condition in `match` must match. If a key is omitted, the rule can match any value of it.

| Key            | Matches                                                                      |
| -------------- | ---------------------------------------------------------------------------- |
| `app_id`       | A Lua pattern on the application ID, or on the WM class when the ID is empty |
| `title`        | A Lua pattern on the current title                                           |
| `focused`      | `true` or `false`                                                            |
| `workspace_id` | The name of the window's workspace, such as `"2"`                            |

Choose the key based on what identifies the window:

- Use `app_id` to match an application across its windows.
- Use `title` to target a particular document or window. Titles often change as the document or page changes.

Layer-shell surfaces are not windows, so a rule never matches them.

## Find the values

List the live windows with:

```sh
gnoblinctl window list
```

Use the `app_id` and `title` of the window you want as the values to match. The `app_id` is the GTK application ID, or
the WM class when the GTK ID is empty, so it can differ from the desktop-entry name shown elsewhere.

## Write a matcher

`app_id` and `title` take [Lua patterns](https://www.lua.org/manual/5.4/manual.html#6.4.1):

- They are case-sensitive and search anywhere by default.
- Use `^` and `$` to match the whole value.
- A dot matches any character; write `%.` for a literal dot.
- JavaScript and PCRE regex syntax is not supported.

To match `org.gnome.Nautilus` only:

```lua
{ match = { app_id = [[^org%.gnome%.Nautilus$]] }, maximised = true }
```

To match a title that contains `Notes` anywhere, omit the anchors:

```lua
{ match = { title = "Notes" }, maximised = false }
```

This also matches `Notes — Work`. To match only the title `Notes`, write `title = "^Notes$"`.

This rule targets only unfocused windows from one app:

```lua
{ match = { app_id = [[^org%.gnome%.Nautilus$]], focused = false }, opacity = 0.9 }
```

## Workspaces {#workspaces}

The stock desk has nine workspaces, named `"1"` to `"9"`. When your configuration declares `workspaces`, the desk builds
those instead, and each workspace is named by its `id`. Declare them in `init.lua`:

```lua
gnoblin.configure {
    workspaces = {
        { id = "code", name = "Code" },
        { id = "web", name = "Web" },
    },
}
```

Name the workspace by that id in a rule:

```lua
seat.data.window_rules = {
    { match = { app_id = [[^org%.example%.Editor$]] }, workspace = { id = "code" } },
    { match = { workspace_id = "web" }, maximised = true },
}
```

A `workspace` that names no workspace of the desk logs one error for each match, and the window stays where it is.

## When rules run again

A rule runs when a window opens. It runs again when the window's title, app ID, focus or workspace changes the set of
rules that match. The fields apply again only when that set changes, so a title change that keeps the same matches does
nothing.

A field that no rule sets falls back to its default. A window whose title stops matching a `secure` rule is not secure,
and a window that stops matching a `maximised` rule is unmaximised.

## Rule order

Matching does not stop at the first rule. Later rules override only the fields they set, so a rule that maximises every
window can be followed by a rule for one application that sets `maximised = false`.

Assign a whole new list to change the rules. The shell compiles a list once, when it first reads it. A list with a
mistake is refused as a whole, with one logged error, and the rules already in force stay.
