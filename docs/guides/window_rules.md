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

## Match a window

Every condition in `match` must match.

| Key                         | Matches                                          |
| --------------------------- | ------------------------------------------------ |
| `type = "window"`           | Application windows                              |
| `type = "layer"`            | Layer-shell surfaces, such as bars and launchers |
| `focused = true` or `false` | Focus state                                      |
| `app_id`                    | GTK application ID, falling back to WM class     |
| `title`                     | Window title                                     |
| `layer`                     | Layer-shell namespace                            |

For example, `{type = "window", focused = false}` selects application windows
that are not focused. It does not select your bar or launcher. Leaving out
`focused` selects both focused and unfocused windows.

A layer namespace is the name a bar or launcher gives its Wayland surface.
Find it in that shell's documentation; it is not necessarily its executable name.

## Match text

Text matchers use **JavaScript regular expressions**, not Lua patterns or globs.
Use `^` and `$` for an exact match.

This example fades a window only while its title is exactly `Notes`:

```lua
gnoblin.window_rule {
    match = {type = "window", title = "^Notes$", focused = false},
    opacity = 0.9,
}
```

`^` means the start of the text and `$` means the end. Without them, `Notes`
also matches `Notes — Work`.

For an app ID containing dots, use `app_id = [[^org\.example\.Editor$]]`.
The backslash makes each dot literal; Lua's `[[...]]` strings keep those
backslashes unchanged. Replace the example ID with the app's GTK ID or WM class.

`gnoblinctl window list --json` shows titles and window IDs. Its `appId` is a
desktop-entry ID and can differ from the ID used by rules. To inspect both raw
IDs, open the [JavaScript console](/developer-console) with Alt+F2 and run:

```javascript
global.get_window_actors().map(({ meta_window: window }) => ({
    title: window.get_title(),
    app_id: window.get_gtk_application_id(),
    wm_class: window.get_wm_class(),
}));
```

Rules use `app_id` when it is set, otherwise `wm_class`.

## Rule order

Matching does not stop at the first rule. Later rules override only the fields
they set. For example, an unfocused-window rule can change opacity while keeping
a radius from an earlier all-window rule.

Use `gnoblin.window_rule` to add rules without removing earlier ones. Passing
a complete `window_rules` list to `gnoblin.configure` replaces the old list;
see [load order](/guides/files_and_load_order#override-or-append).

## What can a rule change?

- [Effects](/guides/window_effects): blur, opacity, corners, borders, shadows and shaders.
- [Titlebars](/guides/window_frames): decoration policy and renderer.
- [Layer animations](/guides/animations#per-surface-animations): entry, exit and timing.

Prefer general rules when a behavior should apply to all clients.
Use app-name exceptions only when you intend different behavior for that app.
