/*
 * gnoblin-shell: role-based client exec + feature toggles.
 *
 * Shell UI is bring-your-own: the compositor spawns whatever binary the config
 * binds to a role, passing it context on the command line (--role/--window/
 * --x/--y/--reason) so the client is a stateless renderer. Roles are configured
 * in [roles] (role = command); feature APIs are toggled in [features]. This is
 * how `window-menu`, and future on-demand UI, are wired without the compositor
 * hard-coding any toolkit.
 *
 * This program is free software; you can redistribute it and/or modify it
 * under the terms of the GNU General Public License as published by the Free
 * Software Foundation; either version 2 of the License, or (at your option)
 * any later version.
 */

#pragma once

#include <glib.h>

extern "C" {
#include <meta/window.h>
}

G_BEGIN_DECLS

/* Command bound to `role` in [roles], or NULL if unset. g_free. */
char* gnoblin_role_command(const char* role);

/* Whether the [features] API `feature` is enabled (fallback `dflt` if unset). */
gboolean gnoblin_feature_enabled(const char* feature, gboolean dflt);

/* Spawn the command bound to `role`, appending the invocation context as flags
 * (--role, and when given --window/--x/--y/--reason). Returns TRUE if a role
 * was configured and launched; FALSE if no command is bound (so the caller can
 * fall back, e.g. to the native menu). `window` may be NULL. */
gboolean gnoblin_role_spawn(const char* role, MetaWindow* window, int x, int y,
                            const char* reason);

/* Export GNOBLIN_ACTIVE_OUTPUT = the current monitor's connector, so plainly
 * spawned gnoblin clients (launcher, osd) land on the active monitor unless an
 * explicit --output was given. Call before spawning on-demand UI. */
void gnoblin_export_active_output(MetaDisplay* display);

G_END_DECLS
