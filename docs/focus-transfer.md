# Prevent unsolicited application focus

Gnoblin prevents unsolicited application focus by default. To allow
applications to activate windows after you have interacted with another
window, set `focus_new_windows` to `"smart"` in
`~/.config/gnoblin/init.lua`:

| Setting             | Accepted values       | Default    | Effect                                         |
| ------------------- | --------------------- | ---------- | ---------------------------------------------- |
| `focus_new_windows` | `"strict"`, `"smart"` | `"strict"` | Selects Mutter's focus policy for new windows. |

```lua
gnoblin.configure {
    window_management = {
        focus_new_windows = "strict",
    },
}
```

With `"strict"`, Mutter applies its focus-stealing checks to new windows and
application activation requests. A request needs recent launch or activation
activity; otherwise, the window stays unfocused. A transient dialog opened by
the focused window can still receive focus. `"smart"` relaxes these checks and
allows activation after you switch to another window.

Changes apply on configuration reload. See the
[`gnoblin.configure.window_management` reference](/config/configure/window_management#focus-behavior)
for focus modes and the [GNOME Shell team's focus-stealing overview](https://blogs.gnome.org/shell-dev/2024/09/20/understanding-gnome-shells-focus-stealing-prevention/).
