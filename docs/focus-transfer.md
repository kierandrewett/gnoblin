# Prevent unsolicited application focus

Gnoblin lets a window take focus when you start it from a launcher, and when an
app opens a prompt such as a password dialog. A background app can also take
focus this way. To stop that, set `focus_new_windows` to `"prevent"` in
`~/.config/gnoblin/init.lua`:

| Setting             | Accepted values       | Starter config value | Effect                                         |
| ------------------- | --------------------- | -------------------- | ---------------------------------------------- |
| `focus_new_windows` | `"allow"`, `"prevent"` | `"allow"`            | Selects Mutter's focus policy for new windows. |

```lua
gnoblin.configure {
    window_management = {
        focus_new_windows = "prevent",
    },
}
```

With `"prevent"`, Mutter applies its focus-stealing checks to new windows and
application activation requests. A request needs recent launch or activation
activity; otherwise, the window stays unfocused. A transient dialog opened by
the focused window can still receive focus.

A window you start from a launcher also stays unfocused, and so does a password
dialog from the keyring or GPG. Typing in it does nothing until you click it.

`"allow"` relaxes these checks and allows activation after you switch to
another window.

Changes apply on configuration reload. See the
[`gnoblin.configure.window_management` reference](/config/configure/window_management#focus-behavior)
for focus modes and the [GNOME Shell team's focus-stealing overview](https://blogs.gnome.org/shell-dev/2024/09/20/understanding-gnome-shells-focus-stealing-prevention/).
