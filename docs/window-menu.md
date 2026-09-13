# Desktop window menu

Gnoblin owns window identity, allowed actions and input. A configured desktop
command owns the menu UI. Bingux uses a compact Quickshell window-menu surface
that follows the keyboard layout selector's row spacing,
including keyboard navigation, outside-click dismissal, theme and screen-edge
placement. The GTK SSD renderer does not create a second popup.

```lua
return {
    shell = {
        ["window-menu"] = { "binguxctl", "ipc", "shell", "windowMenu" },
    },
}
```

The Bingux integration drop-in sets the command. No keyboard shortcut is added.
`gnoblinctl window menu active` is available for a binding of your choosing.
`gnoblinctl config reload` applies command changes. An empty argv disables the
external handler. This does not enable the inherited menu in session modes that
disable it. A configured handler suppresses the inherited menu, so they never
open together. Failed commands are logged; no second desktop menu is launched.

The command is spawned directly, without a shell. Gnoblin appends **one JSON
argument** with `version: 1`, stable string `window` ID, `title`, global logical
pointer coordinates `x`/`y`, and `actions`. Entries have `id`, `text`, `enabled`,
optional `checked`, or `isSeparator`. Renderers must retain the supplied window
ID instead of acting on whichever window is focused after the popup opens.
Actions run through `gnoblinctl window ACTION ID`; Gnoblin resolves the target
again and rejects closed windows and unavailable geometry operations.

Requests arrive from SSD titlebar right-clicks, standard client window-menu
requests, or `gnoblinctl window menu ID`. Locked sessions cannot request window
management. Move/Resize start interactive compositor grabs, not client-side
geometry guesses. This is the window-management menu, not an application's
document/preferences menu.

Private integration coverage:
`GNOBLIN_TEST_WINDOW_MENU=1` with `tests/test-window-frames.py` and the Bingux
renderer exercises real right-click -> Quickshell popup -> original-window action.
