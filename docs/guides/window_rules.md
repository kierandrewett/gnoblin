# window_rules

[Configuration API](/config)

A rule has two parts: `match` selects the windows, and the other fields change
their appearance. Add rules to `~/.config/gnoblin/init.lua` after any
`gnoblin.load(...)` lines. Put rules for all windows before rules for specific ones.

## Add a rule

This dims unfocused application windows without replacing imported rules:

```lua
gnoblin.window_rule {
    match = {type = "window", focused = false},
    opacity = 0.95,
}
```

Save and run `gnoblinctl config reload`. Switch focus to check the result.

## Choose what to match

Every condition in `match` must match. For example,
`{type = "window", focused = false}` selects unfocused application windows;
it does not select a bar or launcher. If `focused` is omitted, the rule can
match both focused and unfocused windows.

| Key                         | Matches                                                  |
| --------------------------- | -------------------------------------------------------- |
| `type = "window"`           | Application windows                                      |
| `type = "layer"`            | Layer-shell surfaces, such as bars and launchers         |
| `focused = true` or `false` | Focus state                                              |
| `app_id`                    | GTK application ID, or WM class when the GTK ID is empty |
| `title`                     | Current window title                                     |
| `layer`                     | Layer-shell namespace supplied by the client             |
| `workspace_id`              | Stable ID of the current workspace                       |
| `workspace_number`          | Current one-based workspace position                     |

Choose the match field based on what should identify the window:

- Use `app_id` to match an application across its windows.
- Use `title` to target a particular document or window. Titles often change
  as the document or page changes.
- Use `layer` to match the namespace chosen by the shell or client. It may not
  match the program's executable name.

## Find the values

List open windows to see their current titles and IDs:

```sh
gnoblinctl window list
```

The `APP ID` shown by `window list` is a desktop-entry ID. It can differ from
the `app_id` used by window rules. To see the exact values for the focused
window, run:

```sh
gnoblinctl window match
```

Use `rule_app_id` for `app_id` and copy the reported `title` for a title rule.
Gnoblin uses the GTK application ID when present and falls back to the WM
class otherwise. Use `--json` for structured output or add a window ID to
inspect a specific window.

For a layer surface, run `gnoblinctl layer list` while it is running. Use the
reported namespace as the `layer` value. Layer surfaces do not appear in
`window list`.

## Write a matcher

The string fields `app_id`, `title`, and `layer` accept **JavaScript regular
expressions**. They are not Lua patterns or shell globs. See the [JavaScript
regular-expression guide](https://developer.mozilla.org/en-US/docs/Web/JavaScript/Guide/Regular_expressions)
for the syntax.

Use `^` at the start and `$` at the end to match the whole value. For example,
if the console reports `org.gnome.Nautilus` as `rule_app_id`, match only that
app like this:

```lua
gnoblin.window_rule {
    match = {type = "window", app_id = [[^org\.gnome\.Nautilus$]]},
    opacity = 0.95,
}
```

In Lua's `[[...]]` string, each backslash is kept as written. The backslash
before each dot makes it a literal dot in the regular expression. Replace the
example with the value reported on your system.

To match a title containing `Notes` anywhere, omit the anchors:

```lua
gnoblin.window_rule {
    match = {type = "window", title = "Notes"},
    opacity = 0.9,
}
```

This also matches titles such as `Notes — Work`. To match only the exact title
`Notes`, write `title = "^Notes$"`.

For a layer client whose namespace is `my-panel`, match that surface like this:

```lua
gnoblin.window_rule {
    match = {type = "layer", layer = "^my-panel$"},
    opacity = 0.9,
}
```

When a rule has more than one condition, all of them must match. This targets
only unfocused windows from one app:

```lua
gnoblin.window_rule {
    match = {
        type = "window",
        app_id = [[^org\.gnome\.Nautilus$]],
        focused = false,
    },
    opacity = 0.9,
}
```

You can combine `app_id` and `title` the same way when a rule should affect a
particular window title from one application.

## Workspaces {#workspaces}

Use a workspace ID when a rule should keep targeting the same configured
workspace as dynamic workspaces are removed or reordered. Configure IDs by
position with `workspace_ids` in `gnoblin.configure.window_management`:

```lua
gnoblin.configure {
    window_management = {
        workspace_ids = {"code", "web", "chat"},
    },
}
```

Each ID must be unique and match `^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$`.
Unconfigured positions get generated `@session-N` IDs for the current session;
these IDs are unsuitable for saved rules or scripts.

`workspace_names` sets displayed labels independently of IDs. Every
`workspace_id` matcher and `{id = ...}` placement target must use an ID from
`workspace_ids`. An undeclared ID makes the configuration invalid.

Match a window's current workspace by ID or by its current one-based position:

```lua
gnoblin.window_rule {
    match = {type = "window", workspace_id = "code"},
    opacity = 0.95,
}

gnoblin.window_rule {
    match = {type = "window", workspace_number = 2},
    opacity = 0.9,
}
```

Workspace matches update when a window moves. Numbers can change when dynamic
workspaces are removed, so use IDs for rules that need a lasting target.

To place a newly created normal window, add a `workspace` effect to its rule:

```lua
gnoblin.window_rule {
    match = {type = "window", app_id = "^org.example.Editor$"},
    workspace = {id = "code"},
}
```

Use `workspace = {number = 2}` to target a position instead. Placement runs
once when the window is created; it is not repeated after title or focus
changes or a config reload. Transient and modal windows remain with their
parent.

Gnoblin leaves the window where it opened and logs a warning if its declared
workspace ID is not currently present. It does the same when a numeric target
position is unavailable when the window opens.

## Rule order

Matching does not stop at the first rule. Later rules override only the fields
they set. For example, an unfocused-window rule can change opacity while keeping
a radius from an earlier all-window rule.

Use `gnoblin.window_rule` to add rules without removing earlier ones. Passing
a complete `window_rules` list to `gnoblin.configure` replaces the configured list;
see [load order](/guides/files_and_load_order#override-or-append).

## What can a rule change?

- [Effects](/guides/window_effects): blur, opacity, corners, borders, shadows and shaders.
- [Titlebars](/guides/window_frames): decoration policy and renderer.
- [Layer animations](/guides/animations#per-surface-animations): entry, exit and timing.

Prefer general rules when a behavior should apply to all clients.
Use app-name exceptions only when you intend different behavior for that app.
