/*
 * gnoblin-shell: a minimal libmutter MetaPlugin — the gnoblin compositor.
 *
 * This program is free software; you can redistribute it and/or
 * modify it under the terms of the GNU General Public License as
 * published by the Free Software Foundation; either version 2 of the
 * License, or (at your option) any later version.
 */

#pragma once

extern "C" {
#include <meta/meta-plugin.h>
}

#define GNOBLIN_TYPE_SHELL_PLUGIN (gnoblin_shell_plugin_get_type())
G_DECLARE_FINAL_TYPE(GnoblinShellPlugin, gnoblin_shell_plugin, GNOBLIN, SHELL_PLUGIN, MetaPlugin)

/* Re-apply the compositor fallback background colour from config. Wallpaper
 * images are rendered by the background layer-shell client. */
void gnoblin_shell_plugin_reload_appearance(void);
