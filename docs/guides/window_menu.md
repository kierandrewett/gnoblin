# Window menu requests

Gnoblin reports window-menu requests to the active shell. The shell draws the
menu as its own Wayland surface and chooses which actions to show.

Subscribe to `gnoblin.window.menu-requested` through the
[compositor bridge](/compositor-bridge#api-127-shell-presentation-requests).
The event includes:

- `window_id`: stable ID of the window that requested the menu;
- `menu_type`: `wm` for a window-manager menu or `app` for an application menu;
- `x` and `y`: requested menu position in global logical coordinates.

For a window-manager menu, API 1.30 also provides a one-use `menu_context`.
Use it with `window.begin_move` or `window.begin_resize` if the shell offers
those actions. The context is tied to the requesting window, expires after
five seconds, and is revoked on disconnect or session lock. Application menu
requests are informational and do not include this capability. See the
[menu action details](/compositor-bridge#api-130-wm-menu-actions).

Use the supplied `window_id` for other window operations; do not target the
currently focused window after the menu opens. Read the live window record to
check which actions are available, then send the corresponding typed
operation through the bridge. Gnoblin rechecks the window and session state
when it applies the operation.
