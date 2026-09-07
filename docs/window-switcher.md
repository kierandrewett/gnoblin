# Fast window switching

`src/scripts/window-switcher.js` supplies Gnoblin's compact window switcher.
It runs inside the shell, with direct access to Mutter's windows. Quickshell,
D-Bus requests, thumbnail rendering and external processes are not involved
in handling a switch.

Install it in `~/.config/gnoblin/scripts/`, then run `gnoblinctl reload-scripts`.
This works in the current session without a compositor restart. The script
uses the existing application/window switcher keybindings, normally Alt+Tab
and Super+Tab. It restores the native handlers when disabled or unloaded.
The native `shell.window-switcher` setting still controls that fallback.

- Alt+Tab selects the previous window in most-recently-used order.
- Keep Alt held and press Tab to cycle. Shift+Tab cycles backwards.
- Release Alt to activate the selected window immediately.
- Escape cancels without changing the active window.
- Left/Right and Enter also work while the chooser is open.
- Click an icon to activate it, or outside the chooser to cancel.

Window order stays fixed during the gesture. Windows that close are removed
without reordering the rest. Minimised windows are included. Dialogs attached
to an application window share their parent's entry. Icons are prepared in
idle time and reused. The chooser has no thumbnail generation or fades.
Short taps do not wait for the chooser to render.

Add this table to `~/.config/gnoblin/gnoblin.toml`:

```toml
[switcher]
enabled = true
show-delay = 80
current-workspace-only = false
```

`show-delay` is the visual reveal delay in milliseconds (0–500), not a delay
before switching. It prevents a short Alt+Tab from flashing a popup. Rules
reload automatically; invalid values retain the previous settings. A config
reload or script reload cancels an active gesture and releases its input grab.

`scripts/test-window-switcher.py` uses actual virtual keyboard events and Foot
windows inside `scripts/run-gnome-shell.sh`. It checks forward and reverse
cycling, stable ordering, cancellation, closing the selected window, reload
and key-release-to-focus latency. This measures compositor focus changes,
not the time until the new application's pixels reach the display.
