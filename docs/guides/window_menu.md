# Window menu

[Configuration reference](/config/configure)

The window menu contains actions such as Minimise, Maximise and Close. Your
desktop shell draws it; Gnoblin tells the shell which window it is for and
which actions are available.

Bingux is one separate shell project with a ready-made menu handler. Other
shells can supply their own.

## Configure the command

For Bingux:

```lua
gnoblin.configure {
    shell = {
        window_menu = {"binguxctl", "ipc", "shell", "windowMenu"},
    },
}
```

Reload, then try:

```sh
gnoblinctl window menu active
```

The same handler receives titlebar right-clicks and standard client menu requests.
An empty command list disables the handler. No shortcut is added automatically.

## Write a handler

The command runs without a shell. Gnoblin appends one JSON argument:

| Field     | Meaning                                                         |
| --------- | --------------------------------------------------------------- |
| `version` | `1`                                                             |
| `window`  | Stable string window ID                                         |
| `title`   | Window title                                                    |
| `x`, `y`  | Pointer in logical desktop coordinates                          |
| `actions` | Entries with id, text, enabled, optional checked or isSeparator |

Example payload, with the actions list shortened:

```json
{
    "version": 1,
    "window": "42",
    "title": "Notes",
    "x": 640,
    "y": 120,
    "actions": [
        { "id": "minimize", "text": "Minimize", "enabled": true },
        { "id": "above", "text": "Always on Top", "enabled": true, "checked": false },
        { "isSeparator": true },
        { "id": "close", "text": "Close", "enabled": true }
    ]
}
```

A handler reads that argument as JSON, draws the enabled actions and uses their
`id` values. For example, selecting Close for this payload runs:

```sh
gnoblinctl window close 42
```

Keep the supplied window ID. Run actions with
`gnoblinctl window ACTION ID`, not whichever window is focused after the popup opens.

Gnoblin rejects closed windows, locked sessions and unavailable actions.
Move and resize put the window into interactive move/resize mode, with
Gnoblin handling the pointer until the action finishes.

If the configured command fails, no fallback menu opens; check the session
log. Clearing `window_menu` disables this menu rather than restoring GNOME's.

## Test

Use `GNOBLIN_TEST_WINDOW_MENU=1` with `tests/test-window-frames.py` and Bingux.
The test clicks a titlebar, opens the menu and checks the original window's action.
