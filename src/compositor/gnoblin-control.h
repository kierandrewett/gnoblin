/*
 * gnoblin-shell: wire config keybindings and the D-Bus/IPC interface into the
 * action dispatcher (gnoblin-actions.h).
 *
 * This program is free software; you can redistribute it and/or modify it
 * under the terms of the GNU General Public License as published by the Free
 * Software Foundation; either version 2 of the License, or (at your option)
 * any later version.
 */

#pragma once

extern "C" {
#include <meta/display.h>
}

/* Grab the config's `bind = [...]` keybindings, claim dev.gnoblin.Shell on the
 * session bus, and expose the local control socket used by gnoblinctl. All
 * action entry points route to gnoblin_actions_dispatch(). Call once on startup;
 * re-call after a config reload to pick up changed keybindings. */
void gnoblin_control_init(MetaDisplay* display);
void gnoblin_control_reload_keybindings(MetaDisplay* display);

/* Disable mutter's built-in keybindings so gnoblin's config owns them. Must run
 * BEFORE meta_context_setup() — otherwise mutter has already loaded them. */
void gnoblin_control_take_over_keybindings(void);

/* Suppress config keybindings while the session is locked, so they can't be used
 * to bypass the lock screen. */
void gnoblin_control_set_locked(gboolean is_locked);
gboolean gnoblin_control_is_locked(void);
