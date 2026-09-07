/*
 * gnoblin: a tiny, dependency-free config file (instead of GSettings/dconf).
 *
 * Read from $GNOBLIN_CONFIG, else $XDG_CONFIG_HOME/gnoblin/gnoblin.toml. The
 * format is TOML. Existing gnoblin.conf files retain legacy INI parsing.
 * Example:
 *
 *     [protocols]
 *     ext-data-control = true
 *
 * Missing files and keys use the caller's default. Invalid reloads retain
 * the last valid configuration. Mutter uses these accessors to gate Wayland
 * protocols; the shell shares the typed TOML parser for live settings.
 * See src/data/gnoblin.toml.example for shell and autostart settings.
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
GVariant *gnoblin_config_parse_toml(const char *contents, GError **error);

/* (Re)load from disk. Safe to call repeatedly. */
void gnoblin_config_reload(void);

/* Last value of `key` in `[section]`. */
gboolean gnoblin_config_get_bool(const char* section, const char* key, gboolean fallback);
int gnoblin_config_get_int(const char* section, const char* key, int fallback);
char* gnoblin_config_get_string(const char* section, const char* key); /* g_free, or NULL */

/* Protocols are available only in the Gnoblin session. Within that boundary,
 * the named [protocols] key defaults on and may explicitly disable a global. */
gboolean gnoblin_config_protocol_enabled(const char* key);

/* All values of a repeated `key` in `[section]`, in file order (e.g. every
 * `exec` in [startup]). g_strfreev, or NULL if none. */
char** gnoblin_config_get_list(const char* section, const char* key);

/* Every key in `[section]`, in file order, including repeated keys (e.g. each
 * accelerator in [bind] or connector in [output]). g_strfreev, or NULL if the
 * section is empty/absent. */
char** gnoblin_config_get_keys(const char* section);

G_END_DECLS
