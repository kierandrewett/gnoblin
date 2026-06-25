/*
 * gnoblin: a tiny, dependency-free config file (instead of GSettings/dconf).
 *
 * Read from $GNOBLIN_CONFIG, else $XDG_CONFIG_HOME/gnoblin/gnoblin.conf. The
 * format is INI-like: `[section]` headers and `key = value` lines, one
 * directive per line, `#` comments. Keys may repeat (e.g. several `exec`
 * lines, or one line per keybinding). Example:
 *
 *     [startup]
 *     exec = gnoblin-topbar
 *
 *     [bind]
 *     Super+Q = close
 *
 *     [protocols]
 *     ext-data-control = on
 *
 * Values are bools (on/off/true/false/yes/no/1/0), ints, or strings. Missing
 * file/keys fall back to the caller's default. gnoblin-shell watches the file
 * and calls gnoblin_config_reload() on change.
 *
 * In the compositor build the shipped gnoblin.defaults.conf is compiled in as a
 * base layer under the user's file (GNOBLIN_EMBED_DEFAULTS), so unset keys still
 * resolve to gnoblin's defaults; the Rust shell embeds the same file.
 *
 * This program is free software; you can redistribute it and/or modify it
 * under the terms of the GNU General Public License as published by the Free
 * Software Foundation; either version 2 of the License, or (at your option)
 * any later version.
 */

#pragma once

#include <glib.h>

G_BEGIN_DECLS

const char* gnoblin_config_path(void);

/* (Re)load from disk. Safe to call repeatedly. */
void gnoblin_config_reload(void);

/* Last value of `key` in `[section]`. */
gboolean gnoblin_config_get_bool(const char* section, const char* key, gboolean fallback);
int gnoblin_config_get_int(const char* section, const char* key, int fallback);
char* gnoblin_config_get_string(const char* section, const char* key); /* g_free, or NULL */

/* All values of a repeated `key` in `[section]`, in file order (e.g. every
 * `exec` in [startup]). g_strfreev, or NULL if none. */
char** gnoblin_config_get_list(const char* section, const char* key);

/* Every key in `[section]`, in file order (e.g. each accelerator in [bind] or
 * connector in [output]). g_strfreev, or NULL if the section is empty/absent. */
char** gnoblin_config_get_keys(const char* section);

G_END_DECLS
