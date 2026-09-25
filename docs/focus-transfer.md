# Prevent unsolicited application focus

Gnoblin honors application activation requests by default. To enable Mutter's
focus-stealing prevention, set `focus_new_windows` to `"strict"`:

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
the focused window can still receive focus.

Changes apply on configuration reload. Set `focus_new_windows` to `"smart"` to
restore Gnoblin's default behavior. See the
[`gnoblin.configure.window_management` reference](/config/configure/window_management#focus-behavior)
for focus modes and the [GNOME Shell team's focus-stealing overview](https://blogs.gnome.org/shell-dev/2024/09/20/understanding-gnome-shells-focus-stealing-prevention/).
