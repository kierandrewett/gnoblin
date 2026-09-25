# gnoblin.configure.shortcuts

Define named command shortcuts or change a built-in GNOME action. Entries
merge by name; omitted fields keep their earlier values. Set `enable = false`
to disable an imported shortcut. Put this in `~/.config/gnoblin/init.lua`;
changes apply on config reload. Up to 256 entries are allowed. Names must be
unique and contain 1–80 letters, numbers, `_` or `-`.

```lua
gnoblin.configure {
    shortcuts = {
        terminal = {
            binding = "<Super>Return",
            command = {"ptyxis", "--new-window"},
        },
        close_window = {
            action = {
                schema = "org.gnome.desktop.wm.keybindings",
                key = "close",
            },
            binding = {"<Super>q"},
        },
    },
}
```

Use exactly one of `command` or `action`.

- **Command shortcut:** Set one GTK accelerator and provide a nonempty
  argument array. Gnoblin runs the command directly, without shell expansion.
- **Built-in action:** Set a GSettings schema and key, then provide a list of
  accelerators. You can use a short name such as `wm.close` or
  `gnome:shell.show_screenshot_ui`. The available groups are `wm`,
  `gnome:shell`, `mutter` and `wayland`. An empty binding list disables the
  action.

To find keys available on your system, run `gsettings list-keys SCHEMA`. To
read a key's description, run `gsettings describe SCHEMA KEY`. Available keys
vary by GNOME version.

| Field           | Accepted values                                                      | Default and effect                                                                       |
| --------------- | -------------------------------------------------------------------- | ---------------------------------------------------------------------------------------- |
| `binding`       | GTK accelerator string for a command; array of strings for an action | Required. The key or keys that invoke the entry.                                         |
| `command`       | Nonempty array of strings                                            | Required when `action` is absent. Runs the first item with remaining items as arguments. |
| `action`        | `"group.key"` or `{schema = "…", key = "…"}`                         | Required when `command` is absent. Selects a built-in GNOME keybinding.                  |
| `trigger`       | `"press"` or `"release"`                                             | `"press"`; command shortcuts only. Bare `"Super"` runs on release.                       |
| `capture_input` | Boolean                                                              | `false`; buffers typing until an integrated popup reports focus.                         |
| `enable`        | Boolean                                                              | `true`; set to `false` to disable an imported entry.                                     |

`trigger` applies to command shortcuts. Bare `"Super"` runs on release. Set
`capture_input = true` to buffer typing until an integrated popup reports
focus through the [compositor bridge](/compositor-bridge#bare-super-and-buffered-typing).
See the [shortcuts guide](/guides/shortcuts) for key names, conflicts and
examples.

### Window management — `org.gnome.desktop.wm.keybindings`

| GSettings key                  | What it does                                 |
| ------------------------------ | -------------------------------------------- |
| `switch-to-workspace-1`        | Switch to workspace 1                        |
| `switch-to-workspace-2`        | Switch to workspace 2                        |
| `switch-to-workspace-3`        | Switch to workspace 3                        |
| `switch-to-workspace-4`        | Switch to workspace 4                        |
| `switch-to-workspace-5`        | Switch to workspace 5                        |
| `switch-to-workspace-6`        | Switch to workspace 6                        |
| `switch-to-workspace-7`        | Switch to workspace 7                        |
| `switch-to-workspace-8`        | Switch to workspace 8                        |
| `switch-to-workspace-9`        | Switch to workspace 9                        |
| `switch-to-workspace-10`       | Switch to workspace 10                       |
| `switch-to-workspace-11`       | Switch to workspace 11                       |
| `switch-to-workspace-12`       | Switch to workspace 12                       |
| `switch-to-workspace-left`     | Switch to workspace left                     |
| `switch-to-workspace-right`    | Switch to workspace right                    |
| `switch-to-workspace-up`       | Switch to workspace above                    |
| `switch-to-workspace-down`     | Switch to workspace below                    |
| `switch-to-workspace-last`     | Switch to last workspace                     |
| `switch-group`                 | Switch windows of an application             |
| `switch-group-backward`        | Reverse switch windows of an application     |
| `switch-applications`          | Switch applications                          |
| `switch-applications-backward` | Reverse switch applications                  |
| `switch-windows`               | Switch windows                               |
| `switch-windows-backward`      | Reverse switch windows                       |
| `switch-panels`                | Switch system controls                       |
| `switch-panels-backward`       | Reverse switch system controls               |
| `cycle-group`                  | Switch windows of an app directly            |
| `cycle-group-backward`         | Reverse switch windows of an app directly    |
| `cycle-windows`                | Switch windows directly                      |
| `cycle-windows-backward`       | Reverse switch windows directly              |
| `cycle-panels`                 | Switch system controls directly              |
| `cycle-panels-backward`        | Reverse switch system controls directly      |
| `show-desktop`                 | Hide all normal windows                      |
| `panel-main-menu`              | Deprecated; ignored by GNOME 51              |
| `panel-run-dialog`             | Show the run command prompt                  |
| `set-spew-mark`                | Do not use (as noted in the schema)          |
| `activate-window-menu`         | Activate the window menu                     |
| `toggle-fullscreen`            | Toggle fullscreen mode                       |
| `toggle-maximized`             | Toggle maximization state                    |
| `toggle-above`                 | Toggle window always appearing on top        |
| `maximize`                     | Maximize window                              |
| `unmaximize`                   | Restore window                               |
| `minimize`                     | Minimize window                              |
| `close`                        | Close window                                 |
| `begin-move`                   | Move window                                  |
| `begin-resize`                 | Resize window                                |
| `toggle-on-all-workspaces`     | Toggle window on all workspaces or one       |
| `move-to-workspace-1`          | Move window to workspace 1                   |
| `move-to-workspace-2`          | Move window to workspace 2                   |
| `move-to-workspace-3`          | Move window to workspace 3                   |
| `move-to-workspace-4`          | Move window to workspace 4                   |
| `move-to-workspace-5`          | Move window to workspace 5                   |
| `move-to-workspace-6`          | Move window to workspace 6                   |
| `move-to-workspace-7`          | Move window to workspace 7                   |
| `move-to-workspace-8`          | Move window to workspace 8                   |
| `move-to-workspace-9`          | Move window to workspace 9                   |
| `move-to-workspace-10`         | Move window to workspace 10                  |
| `move-to-workspace-11`         | Move window to workspace 11                  |
| `move-to-workspace-12`         | Move window to workspace 12                  |
| `move-to-workspace-last`       | Move window to last workspace                |
| `move-to-workspace-left`       | Move window one workspace to the left        |
| `move-to-workspace-right`      | Move window one workspace to the right       |
| `move-to-workspace-up`         | Move window one workspace up                 |
| `move-to-workspace-down`       | Move window one workspace down               |
| `move-to-monitor-left`         | Move window to the next monitor on the left  |
| `move-to-monitor-right`        | Move window to the next monitor on the right |
| `move-to-monitor-up`           | Move window to the next monitor above        |
| `move-to-monitor-down`         | Move window to the next monitor below        |
| `raise-or-lower`               | Raise window if covered, otherwise lower it  |
| `raise`                        | Raise window above other windows             |
| `lower`                        | Lower window below other windows             |
| `maximize-vertically`          | Maximize window vertically                   |
| `maximize-horizontally`        | Maximize window horizontally                 |
| `move-to-corner-nw`            | Move window to top left corner               |
| `move-to-corner-ne`            | Move window to top right corner              |
| `move-to-corner-sw`            | Move window to bottom left corner            |
| `move-to-corner-se`            | Move window to bottom right corner           |
| `move-to-side-n`               | Move window to top edge of screen            |
| `move-to-side-s`               | Move window to bottom edge of screen         |
| `move-to-side-e`               | Move window to right side of screen          |
| `move-to-side-w`               | Move window to left side of screen           |
| `move-to-center`               | Move window to center of screen              |
| `switch-input-source`          | Switch input source                          |
| `switch-input-source-backward` | Switch input source backward                 |
| `always-on-top`                | Toggle window to be always on top            |

### GNOME Shell — `org.gnome.shell.keybindings`

| GSettings key                     | What it does                                                 |
| --------------------------------- | ------------------------------------------------------------ |
| `shift-overview-up`               | Keybinding to shift between overview states                  |
| `shift-overview-down`             | Keybinding to shift between overview states                  |
| `toggle-application-view`         | Keybinding to open the “Show Applications” view              |
| `toggle-overview`                 | Keybinding to open the overview                              |
| `toggle-message-tray`             | Keybinding to toggle the visibility of the notification list |
| `toggle-quick-settings`           | Keybinding to toggle the quick settings menu                 |
| `focus-active-notification`       | Keybinding to focus the active notification                  |
| `switch-to-application-1`         | Switch to application 1                                      |
| `switch-to-application-2`         | Switch to application 2                                      |
| `switch-to-application-3`         | Switch to application 3                                      |
| `switch-to-application-4`         | Switch to application 4                                      |
| `switch-to-application-5`         | Switch to application 5                                      |
| `switch-to-application-6`         | Switch to application 6                                      |
| `switch-to-application-7`         | Switch to application 7                                      |
| `switch-to-application-8`         | Switch to application 8                                      |
| `switch-to-application-9`         | Switch to application 9                                      |
| `open-new-window-application-1`   | Open a new instance of application 1                         |
| `open-new-window-application-2`   | Open a new instance of application 2                         |
| `open-new-window-application-3`   | Open a new instance of application 3                         |
| `open-new-window-application-4`   | Open a new instance of application 4                         |
| `open-new-window-application-5`   | Open a new instance of application 5                         |
| `open-new-window-application-6`   | Open a new instance of application 6                         |
| `open-new-window-application-7`   | Open a new instance of application 7                         |
| `open-new-window-application-8`   | Open a new instance of application 8                         |
| `open-new-window-application-9`   | Open a new instance of application 9                         |
| `show-screenshot-ui`              | Take a screenshot interactively                              |
| `show-screen-recording-ui`        | Record a screencast interactively                            |
| `screenshot-window`               | Take a screenshot of a window                                |
| `screenshot`                      | Take a screenshot                                            |
| `screen-brightness-up`            | Screen brightness up                                         |
| `screen-brightness-up-monitor`    | Screen brightness up on current monitor                      |
| `screen-brightness-down`          | Screen brightness down                                       |
| `screen-brightness-down-monitor`  | Screen brightness down on current monitor                    |
| `screen-brightness-cycle`         | Screen brightness cycle                                      |
| `screen-brightness-cycle-monitor` | Screen brightness cycle on current monitor                   |

### Mutter — `org.gnome.mutter.keybindings`

| GSettings key          | What it does                               |
| ---------------------- | ------------------------------------------ |
| `toggle-tiled-left`    | View split on left                         |
| `toggle-tiled-right`   | View split on right                        |
| `switch-monitor`       | Switch monitor configurations              |
| `rotate-monitor`       | Rotates the built-in monitor configuration |
| `cancel-input-capture` | Cancel any active input capture session    |

### Wayland — `org.gnome.mutter.wayland.keybindings`

| GSettings key          | What it does        |
| ---------------------- | ------------------- |
| `switch-to-session-1`  | Switch to VT 1      |
| `switch-to-session-2`  | Switch to VT 2      |
| `switch-to-session-3`  | Switch to VT 3      |
| `switch-to-session-4`  | Switch to VT 4      |
| `switch-to-session-5`  | Switch to VT 5      |
| `switch-to-session-6`  | Switch to VT 6      |
| `switch-to-session-7`  | Switch to VT 7      |
| `switch-to-session-8`  | Switch to VT 8      |
| `switch-to-session-9`  | Switch to VT 9      |
| `switch-to-session-10` | Switch to VT 10     |
| `switch-to-session-11` | Switch to VT 11     |
| `switch-to-session-12` | Switch to VT 12     |
| `restore-shortcuts`    | Re-enable shortcuts |

Existing configs can continue using the older `keybindings` field. See the
[migration note](/config/configure/keybindings) to convert one to `shortcuts`.

The bundled config supplies these names:

| Keys       | Shortcut names                                               |
| ---------- | ------------------------------------------------------------ |
| Volume     | `volume-up`, `volume-down`, `volume-mute`, `microphone-mute` |
| Brightness | `brightness-up`, `brightness-down`                           |
| Playback   | `media-play-pause`, `media-next`, `media-previous`           |
| Files      | `files`                                                      |
| Built-in   | `screenshot`, `close_window`                                 |

To disable one after loading the bundled config:

```lua
gnoblin.configure.shortcuts["volume-up"].enable = false
```
