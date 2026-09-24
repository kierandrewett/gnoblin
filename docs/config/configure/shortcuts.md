# gnoblin.configure.shortcuts

Define command shortcuts and change built-in actions by name. Entries merge with the same name; fields you omit keep their previous values. Set `enable = false` to disable an imported shortcut.

```lua
gnoblin.configure {
    shortcuts = {
        terminal = {
            binding = "<Super>Return",
            command = {"ptyxis", "--new-window"},
        },
        screenshot = {
            action = "gnome:shell.show_screenshot_ui",
            binding = {"Print"},
        },
    },
}
```

Use `command` for a program shortcut or `action` for a built-in GNOME or
Mutter action. Built-in actions require a list of bindings; use an empty list
to disable the action. Command shortcuts use one binding string and an argv
array. See the [shortcuts guide](/guides/shortcuts) for key names, conflicts
and command behavior.

Action names use `group.action`. The group selects the GNOME keybinding schema:

| Group         | GSettings schema                       | Example                          |
| ------------- | -------------------------------------- | -------------------------------- |
| `wm`          | `org.gnome.desktop.wm.keybindings`     | `wm.close`                       |
| `gnome:shell` | `org.gnome.shell.keybindings`          | `gnome:shell.show_screenshot_ui` |
| `mutter`      | `org.gnome.mutter.keybindings`         | `mutter.toggle_tiled_left`       |
| `wayland`     | `org.gnome.mutter.wayland.keybindings` | `wayland.restore_shortcuts`      |

The available actions depend on the installed GNOME version. List the keys in
the matching schema to find actions:

```sh
gsettings list-keys org.gnome.desktop.wm.keybindings
gsettings list-keys org.gnome.shell.keybindings
gsettings list-keys org.gnome.mutter.keybindings
gsettings list-keys org.gnome.mutter.wayland.keybindings
```

GSettings prints native keys with hyphens. Use underscores in the action name;
for example, `show-screenshot-ui` becomes
`gnome:shell.show_screenshot_ui`. To read an action's description, run
`gsettings describe SCHEMA KEY`, such as:

```sh
gsettings describe org.gnome.desktop.wm.keybindings close
gsettings describe org.gnome.shell.keybindings show-screenshot-ui
```

These commands list GNOME actions and descriptions; they do not show the
binding currently configured by Gnoblin. Gnoblin applies active bindings from
the Lua config.

## Built-in action catalog

This catalog lists the actions in the GNOME 51 schemas shipped with Gnoblin. Each value is a complete `action` name you can use in `gnoblin.configure.shortcuts`. The installed schema is authoritative: other GNOME versions can add, remove, or rename actions. Use the commands above to check your system.

### Window management (`wm`)

| Action                            | What it does                                 |
| --------------------------------- | -------------------------------------------- |
| `wm.switch_to_workspace_1`        | Switch to workspace 1                        |
| `wm.switch_to_workspace_2`        | Switch to workspace 2                        |
| `wm.switch_to_workspace_3`        | Switch to workspace 3                        |
| `wm.switch_to_workspace_4`        | Switch to workspace 4                        |
| `wm.switch_to_workspace_5`        | Switch to workspace 5                        |
| `wm.switch_to_workspace_6`        | Switch to workspace 6                        |
| `wm.switch_to_workspace_7`        | Switch to workspace 7                        |
| `wm.switch_to_workspace_8`        | Switch to workspace 8                        |
| `wm.switch_to_workspace_9`        | Switch to workspace 9                        |
| `wm.switch_to_workspace_10`       | Switch to workspace 10                       |
| `wm.switch_to_workspace_11`       | Switch to workspace 11                       |
| `wm.switch_to_workspace_12`       | Switch to workspace 12                       |
| `wm.switch_to_workspace_left`     | Switch to workspace left                     |
| `wm.switch_to_workspace_right`    | Switch to workspace right                    |
| `wm.switch_to_workspace_up`       | Switch to workspace above                    |
| `wm.switch_to_workspace_down`     | Switch to workspace below                    |
| `wm.switch_to_workspace_last`     | Switch to last workspace                     |
| `wm.switch_group`                 | Switch windows of an application             |
| `wm.switch_group_backward`        | Reverse switch windows of an application     |
| `wm.switch_applications`          | Switch applications                          |
| `wm.switch_applications_backward` | Reverse switch applications                  |
| `wm.switch_windows`               | Switch windows                               |
| `wm.switch_windows_backward`      | Reverse switch windows                       |
| `wm.switch_panels`                | Switch system controls                       |
| `wm.switch_panels_backward`       | Reverse switch system controls               |
| `wm.cycle_group`                  | Switch windows of an app directly            |
| `wm.cycle_group_backward`         | Reverse switch windows of an app directly    |
| `wm.cycle_windows`                | Switch windows directly                      |
| `wm.cycle_windows_backward`       | Reverse switch windows directly              |
| `wm.cycle_panels`                 | Switch system controls directly              |
| `wm.cycle_panels_backward`        | Reverse switch system controls directly      |
| `wm.show_desktop`                 | Hide all normal windows                      |
| `wm.panel_main_menu`              | Deprecated; ignored by GNOME 51              |
| `wm.panel_run_dialog`             | Show the run command prompt                  |
| `wm.set_spew_mark`                | Do not use (as noted in the schema)          |
| `wm.activate_window_menu`         | Activate the window menu                     |
| `wm.toggle_fullscreen`            | Toggle fullscreen mode                       |
| `wm.toggle_maximized`             | Toggle maximization state                    |
| `wm.toggle_above`                 | Toggle window always appearing on top        |
| `wm.maximize`                     | Maximize window                              |
| `wm.unmaximize`                   | Restore window                               |
| `wm.minimize`                     | Minimize window                              |
| `wm.close`                        | Close window                                 |
| `wm.begin_move`                   | Move window                                  |
| `wm.begin_resize`                 | Resize window                                |
| `wm.toggle_on_all_workspaces`     | Toggle window on all workspaces or one       |
| `wm.move_to_workspace_1`          | Move window to workspace 1                   |
| `wm.move_to_workspace_2`          | Move window to workspace 2                   |
| `wm.move_to_workspace_3`          | Move window to workspace 3                   |
| `wm.move_to_workspace_4`          | Move window to workspace 4                   |
| `wm.move_to_workspace_5`          | Move window to workspace 5                   |
| `wm.move_to_workspace_6`          | Move window to workspace 6                   |
| `wm.move_to_workspace_7`          | Move window to workspace 7                   |
| `wm.move_to_workspace_8`          | Move window to workspace 8                   |
| `wm.move_to_workspace_9`          | Move window to workspace 9                   |
| `wm.move_to_workspace_10`         | Move window to workspace 10                  |
| `wm.move_to_workspace_11`         | Move window to workspace 11                  |
| `wm.move_to_workspace_12`         | Move window to workspace 12                  |
| `wm.move_to_workspace_last`       | Move window to last workspace                |
| `wm.move_to_workspace_left`       | Move window one workspace to the left        |
| `wm.move_to_workspace_right`      | Move window one workspace to the right       |
| `wm.move_to_workspace_up`         | Move window one workspace up                 |
| `wm.move_to_workspace_down`       | Move window one workspace down               |
| `wm.move_to_monitor_left`         | Move window to the next monitor on the left  |
| `wm.move_to_monitor_right`        | Move window to the next monitor on the right |
| `wm.move_to_monitor_up`           | Move window to the next monitor above        |
| `wm.move_to_monitor_down`         | Move window to the next monitor below        |
| `wm.raise_or_lower`               | Raise window if covered, otherwise lower it  |
| `wm.raise`                        | Raise window above other windows             |
| `wm.lower`                        | Lower window below other windows             |
| `wm.maximize_vertically`          | Maximize window vertically                   |
| `wm.maximize_horizontally`        | Maximize window horizontally                 |
| `wm.move_to_corner_nw`            | Move window to top left corner               |
| `wm.move_to_corner_ne`            | Move window to top right corner              |
| `wm.move_to_corner_sw`            | Move window to bottom left corner            |
| `wm.move_to_corner_se`            | Move window to bottom right corner           |
| `wm.move_to_side_n`               | Move window to top edge of screen            |
| `wm.move_to_side_s`               | Move window to bottom edge of screen         |
| `wm.move_to_side_e`               | Move window to right side of screen          |
| `wm.move_to_side_w`               | Move window to left side of screen           |
| `wm.move_to_center`               | Move window to center of screen              |
| `wm.switch_input_source`          | Switch input source                          |
| `wm.switch_input_source_backward` | Switch input source backward                 |
| `wm.always_on_top`                | Toggle window to be always on top            |

### GNOME Shell (`gnome:shell`)

| Action                                        | What it does                                                 |
| --------------------------------------------- | ------------------------------------------------------------ |
| `gnome:shell.shift_overview_up`               | Keybinding to shift between overview states                  |
| `gnome:shell.shift_overview_down`             | Keybinding to shift between overview states                  |
| `gnome:shell.toggle_application_view`         | Keybinding to open the “Show Applications” view              |
| `gnome:shell.toggle_overview`                 | Keybinding to open the overview                              |
| `gnome:shell.toggle_message_tray`             | Keybinding to toggle the visibility of the notification list |
| `gnome:shell.toggle_quick_settings`           | Keybinding to toggle the quick settings menu                 |
| `gnome:shell.focus_active_notification`       | Keybinding to focus the active notification                  |
| `gnome:shell.switch_to_application_1`         | Switch to application 1                                      |
| `gnome:shell.switch_to_application_2`         | Switch to application 2                                      |
| `gnome:shell.switch_to_application_3`         | Switch to application 3                                      |
| `gnome:shell.switch_to_application_4`         | Switch to application 4                                      |
| `gnome:shell.switch_to_application_5`         | Switch to application 5                                      |
| `gnome:shell.switch_to_application_6`         | Switch to application 6                                      |
| `gnome:shell.switch_to_application_7`         | Switch to application 7                                      |
| `gnome:shell.switch_to_application_8`         | Switch to application 8                                      |
| `gnome:shell.switch_to_application_9`         | Switch to application 9                                      |
| `gnome:shell.open_new_window_application_1`   | Open a new instance of application 1                         |
| `gnome:shell.open_new_window_application_2`   | Open a new instance of application 2                         |
| `gnome:shell.open_new_window_application_3`   | Open a new instance of application 3                         |
| `gnome:shell.open_new_window_application_4`   | Open a new instance of application 4                         |
| `gnome:shell.open_new_window_application_5`   | Open a new instance of application 5                         |
| `gnome:shell.open_new_window_application_6`   | Open a new instance of application 6                         |
| `gnome:shell.open_new_window_application_7`   | Open a new instance of application 7                         |
| `gnome:shell.open_new_window_application_8`   | Open a new instance of application 8                         |
| `gnome:shell.open_new_window_application_9`   | Open a new instance of application 9                         |
| `gnome:shell.show_screenshot_ui`              | Take a screenshot interactively                              |
| `gnome:shell.show_screen_recording_ui`        | Record a screencast interactively                            |
| `gnome:shell.screenshot_window`               | Take a screenshot of a window                                |
| `gnome:shell.screenshot`                      | Take a screenshot                                            |
| `gnome:shell.screen_brightness_up`            | Screen brightness up                                         |
| `gnome:shell.screen_brightness_up_monitor`    | Screen brightness up on current monitor                      |
| `gnome:shell.screen_brightness_down`          | Screen brightness down                                       |
| `gnome:shell.screen_brightness_down_monitor`  | Screen brightness down on current monitor                    |
| `gnome:shell.screen_brightness_cycle`         | Screen brightness cycle                                      |
| `gnome:shell.screen_brightness_cycle_monitor` | Screen brightness cycle on current monitor                   |

### Mutter (`mutter`)

| Action                        | What it does                               |
| ----------------------------- | ------------------------------------------ |
| `mutter.toggle_tiled_left`    | View split on left                         |
| `mutter.toggle_tiled_right`   | View split on right                        |
| `mutter.switch_monitor`       | Switch monitor configurations              |
| `mutter.rotate_monitor`       | Rotates the built-in monitor configuration |
| `mutter.cancel_input_capture` | Cancel any active input capture session    |

### Wayland (`wayland`)

| Action                         | What it does        |
| ------------------------------ | ------------------- |
| `wayland.switch_to_session_1`  | Switch to VT 1      |
| `wayland.switch_to_session_2`  | Switch to VT 2      |
| `wayland.switch_to_session_3`  | Switch to VT 3      |
| `wayland.switch_to_session_4`  | Switch to VT 4      |
| `wayland.switch_to_session_5`  | Switch to VT 5      |
| `wayland.switch_to_session_6`  | Switch to VT 6      |
| `wayland.switch_to_session_7`  | Switch to VT 7      |
| `wayland.switch_to_session_8`  | Switch to VT 8      |
| `wayland.switch_to_session_9`  | Switch to VT 9      |
| `wayland.switch_to_session_10` | Switch to VT 10     |
| `wayland.switch_to_session_11` | Switch to VT 11     |
| `wayland.switch_to_session_12` | Switch to VT 12     |
| `wayland.restore_shortcuts`    | Re-enable shortcuts |

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
