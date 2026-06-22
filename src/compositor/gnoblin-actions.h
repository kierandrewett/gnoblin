/*
 * gnoblin-shell: the window-action API — gnoblin's public "dispatcher".
 *
 * Every window-management operation (close, maximize, snap, move, …) is a named
 * action run through gnoblin_actions_dispatch(). gnoblin's own config
 * keybindings, window menus and the D-Bus/IPC interface all funnel through this
 * one entry point, and so can user scripts — the same API gnoblin dogfoods is
 * the one users get.
 *
 * This program is free software; you can redistribute it and/or modify it
 * under the terms of the GNU General Public License as published by the Free
 * Software Foundation; either version 2 of the License, or (at your option)
 * any later version.
 */

#pragma once

#include <glib.h>
extern "C" {
#include <meta/display.h>
#include <meta/window.h>
}

/* Run `action` (with optional space-or-arg `arg`) on `target` — or, when
 * `target` is NULL, on the currently focused window. Unknown actions warn and
 * no-op. `timestamp` is a user-interaction time (0 = current). */
void gnoblin_actions_dispatch(MetaDisplay* display, const char* action, const char* arg,
                              MetaWindow* target, guint32 timestamp);

/* The names of all built-in actions (NULL-terminated, static). */
const char* const* gnoblin_actions_list(void);
