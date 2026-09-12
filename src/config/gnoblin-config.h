/*
 * gnoblin: shared configuration loading for Mutter and Shell.
 *
 * Read from $GNOBLIN_CONFIG, else init.lua under $XDG_CONFIG_HOME/gnoblin.
 * Configuration is Lua only. `GNOBLIN_CONFIG` can select any Lua filename.
 *
 * Missing files and keys use the caller's default. Invalid reloads retain
 * the last valid configuration. Mutter uses these accessors to gate Wayland
 * protocols; the shell receives the same evaluated document for live settings.
 *
 * This program is free software; you can redistribute it and/or modify it
 * under the terms of the GNU General Public License as published by the Free
 * Software Foundation; either version 2 of the License, or (at your option)
 * any later version.
 */

#pragma once

#include <glib.h>

G_BEGIN_DECLS

/* Current root filename. Free the result with g_free(). */
char* gnoblin_config_path(void);
/* Evaluate one config with a fresh Lua state and record every dependency. */
GVariant *gnoblin_config_evaluate_file(const char *path, GPtrArray *paths,
                                      GPtrArray *directories, GError **error);

/* Read the selected root. An absent root uses defaults. Errors retain paths
 * and directories so file monitors can retry when a dependency is repaired. */
GVariant *gnoblin_config_load_document(const char *path, GPtrArray **paths,
                                      GPtrArray **directories, GError **error);

/* Expand a config-relative path or glob in deterministic order. `watched_dirs`
 * receives canonical directories that must be monitored for later matches. */
GPtrArray *gnoblin_config_expand_paths(const char *including_file,
                                       const char *pattern,
                                       GPtrArray *watched_dirs,
                                       GError **error);

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
