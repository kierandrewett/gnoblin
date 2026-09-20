# Window rules

[Configuration reference](configuration-reference.md)

Rules select windows and change their appearance. Put broad rules first and
exceptions last.

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

An omitted key adds no constraint. An empty matcher is broad; normally specify
at least a type.

## Match text

Text matchers use **JavaScript regular expressions**, not Lua patterns or globs.
Use `^` and `$` for an exact match.

```lua
local match = {
    app_id = [[^org\.example\.Editor$]],
}
```

Lua's `[[...]]` strings preserve regex backslashes.
In a quoted string, double them: `"^org\\.example\\.Editor$"`.

Inspect windows with `gnoblinctl window list --json`. Its desktop-entry
`appId` may differ from the raw GTK ID or WM class used here.

## Rule order

Matching does not stop at the first rule. Later rules override only the fields
they set. For example, an unfocused-window rule can change opacity while keeping
a radius from an earlier all-window rule.

This is separate from [Lua list merging](configuration-loading.md#override-or-append):
replacing the rule list discards earlier rules before matching even begins.

## What can a rule change?

- [Effects](window-effects.md): blur, opacity, corners, borders, shadows and shaders.
- [Titlebars](window-frames.md): decoration policy and renderer.
- [Layer animations](animations.md#per-surface-animations): entry, exit and timing.

Prefer general rules when a behavior should apply to all clients.
Use app-name exceptions only when you intend different behavior for that app.
