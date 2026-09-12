/*
 * gnoblin: shared Lua, TOML and legacy configuration loading.
 *
 * This program is free software; you can redistribute it and/or modify it
 * under the terms of the GNU General Public License as published by the Free
 * Software Foundation; either version 2 of the License, or (at your option)
 * any later version.
 */

#include "gnoblin-config.h"

#include <errno.h>
#include <glib/gstdio.h>
#include <limits.h>
#include <stdlib.h>
#include <string.h>

/*
 * gnoblin.conf grammar (see also src/config/README.md's Grammar Contract,
 * which this must stay in lockstep with):
 *
 * - Each line is trimmed of leading space/tab and trailing space/tab/CR/LF.
 * - Empty lines and lines whose first trimmed byte is `#` or `;` are comments.
 * - A section line starts with `[` and uses the first later `]`; the name inside
 *   is trimmed, trailing text is ignored, and a missing `]` discards the line.
 * - A key/value line uses the first `=`. Empty keys are ignored. Repeated keys
 *   are retained in file order; scalar lookups return the last value.
 * - Values starting with a single or double quote use bytes up to the next same
 *   quote and drop the rest of the line. There is no escape processing.
 * - Unquoted values strip a `#` inline comment only when the `#` is introduced
 *   by space/tab and is outside a simple quoted span. `;` is data inline.
 */
#define ROOT_SECTION ""

typedef struct {
    char* key;
    char* value;
} ConfigEntry;

/* section name -> GPtrArray<ConfigEntry*> (in file order, repeats allowed) */
static GHashTable* loaded_sections;

static gboolean validate_document (GVariant *document, GError **error)
{
    const char *sections[] = {"protocols", "layer-shell", "window-management"};
    const char *keys[] = {NULL, "preserve-active-window", "constrain-drag-to-work-area"};
    for (guint i = 0; i < G_N_ELEMENTS (sections); i++) {
        g_autoptr(GVariant) section = g_variant_lookup_value (document, sections[i], NULL);
        if (!section)
            continue;
        gboolean valid = g_variant_is_of_type (section, G_VARIANT_TYPE_VARDICT);
        if (valid) {
            GVariantIter iter;
            const char *name;
            GVariant *value;
            g_variant_iter_init (&iter, section);
            while (g_variant_iter_next (&iter, "{&sv}", &name, &value)) {
                valid = g_variant_is_of_type (value, G_VARIANT_TYPE_BOOLEAN) &&
                        (!keys[i] || g_str_equal (name, keys[i]));
                g_variant_unref (value);
                if (!valid)
                    break;
            }
        }
        if (!valid) {
            g_set_error (error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                         "%s must be a table of supported boolean settings", sections[i]);
            return FALSE;
        }
    }
    return TRUE;
}

GVariant *gnoblin_config_load_document (const char *path, GPtrArray **paths,
                                       GPtrArray **directories, GError **error)
{
    g_autoptr(GPtrArray) loaded_paths = g_ptr_array_new_with_free_func (g_free);
    g_autoptr(GPtrArray) watched_dirs = g_ptr_array_new_with_free_func (g_free);
    g_autoptr(GVariant) document = NULL;
    g_autofree char *canonical = g_canonicalize_filename (path, NULL);
    g_ptr_array_add (loaded_paths, g_strdup (canonical));
    // Only an absent root uses defaults. Missing explicitly loaded files fail.
    struct stat info;
    if (g_stat (canonical, &info) < 0 && (errno == ENOENT || errno == ENOTDIR) &&
        !g_file_test (canonical, G_FILE_TEST_IS_SYMLINK)) {
        GVariantBuilder empty;
        g_variant_builder_init (&empty, G_VARIANT_TYPE_VARDICT);
        document = g_variant_ref_sink (g_variant_builder_end (&empty));
    } else {
        document = gnoblin_config_evaluate_file (canonical, loaded_paths, watched_dirs, error);
        if (document && !g_variant_is_of_type (document, G_VARIANT_TYPE_VARDICT)) {
            g_set_error_literal (error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                                 "configuration must be a table of settings");
            g_clear_pointer (&document, g_variant_unref);
        }
        if (document && !validate_document (document, error))
            g_clear_pointer (&document, g_variant_unref);
    }
    if (paths)
        *paths = g_steal_pointer (&loaded_paths);
    if (directories)
        *directories = g_steal_pointer (&watched_dirs);
    return g_steal_pointer (&document);
}

char* gnoblin_config_path(void) {
    char *path;
    const char *override = g_getenv ("GNOBLIN_CONFIG");
    if (override && override[0]) {
        path = g_canonicalize_filename (override, NULL);
        return path;
    }
    const char *names[] = {"init.lua", "gnoblin.toml", "gnoblin.conf"};
    for (guint i = 0; i < G_N_ELEMENTS (names); i++) {
        path = g_build_filename (g_get_user_config_dir (), "gnoblin", names[i], NULL);
        if (g_file_test (path, G_FILE_TEST_EXISTS))
            return path;
        g_clear_pointer (&path, g_free);
    }
    path = g_build_filename (g_get_user_config_dir (), "gnoblin", "init.lua", NULL);
    return path;
}

static void config_entry_free(gpointer data) {
    ConfigEntry* e = data;

    g_free(e->key);
    g_free(e->value);
    g_free(e);
}

static GPtrArray* entry_list_new(void) {
    return g_ptr_array_new_with_free_func(config_entry_free);
}

static GHashTable* section_table_new(void) {
    GHashTable* table =
        g_hash_table_new_full(g_str_hash, g_str_equal, g_free, (GDestroyNotify)g_ptr_array_unref);

    /* Top-level keys before any [section] live under ROOT_SECTION. */
    g_hash_table_replace(table, g_strdup(ROOT_SECTION), entry_list_new());
    return table;
}

static GPtrArray* ensure_section(GHashTable* table, const char* name) {
    GPtrArray* entries = g_hash_table_lookup(table, name);

    if (!entries) {
        entries = entry_list_new();
        g_hash_table_replace(table, g_strdup(name), entries);
    }
    return entries;
}

static char* strip(char* s) {
    char* end;

    while (*s == ' ' || *s == '\t')
        s++;
    if (*s == '\0')
        return s;
    end = s + strlen(s) - 1;
    while (end > s && (*end == ' ' || *end == '\t' || *end == '\r' || *end == '\n'))
        *end-- = '\0';
    return s;
}

/* Turn the raw text after `=` into the final value: a fully-quoted string is the
 * text between the opening quote and its match (anything after, e.g. a trailing
 * comment, is dropped); otherwise a whitespace-introduced `#` inline comment is
 * stripped — but not one inside a quoted span, since `spawn`/bind values embed
 * shell commands that legitimately contain `#` (and `;`) inside quoted args.
 * Only `#` starts a comment (not `;`), so shell separators survive. Modifies `s`
 * in place. */
static char* clean_value(char* s) {
    char quote = 0;
    char* c;

    while (*s == ' ' || *s == '\t')
        s++;

    if (*s == '"' || *s == '\'') {
        char* start = s + 1;
        char* end = strchr(start, *s);

        if (end) {
            *end = '\0';
            return start;
        }
    }

    for (c = s; *c; c++) {
        if (quote) {
            if (*c == quote)
                quote = 0;
        } else if (*c == '"' || *c == '\'') {
            quote = *c;
        } else if (*c == '#' && c != s && (c[-1] == ' ' || c[-1] == '\t')) {
            *c = '\0';
            break;
        }
    }

    return strip(s);
}

static gboolean switch_current_section(GHashTable* table, char* line, GPtrArray** current) {
    char* close;
    char* name;

    if (line[0] != '[')
        return FALSE;

    close = strchr(line, ']');
    if (!close)
        return TRUE;

    *close = '\0';
    name = strip(line + 1);
    *current = ensure_section(table, name);
    return TRUE;
}

static void append_entry(GPtrArray* entries, char* line) {
    char* eq = strchr(line, '=');
    ConfigEntry* e;

    if (!eq)
        return;
    *eq = '\0';

    e = g_new0(ConfigEntry, 1);
    e->key = g_strdup(strip(line));
    e->value = g_strdup(clean_value(eq + 1));
    if (e->key[0] == '\0') {
        config_entry_free(e);
        return;
    }
    g_ptr_array_add(entries, e);
}

/* Parse `contents` into `table` (sectioned, repeats allowed, last value wins).
 * Appends to existing sections, so calling it twice — shipped defaults first,
 * then the user's file — layers the user's values on top (scalar lookup returns
 * the last match). Modifies a private copy of `contents`. */
static void parse_into(GHashTable* table, const char* contents) {
    g_auto(GStrv) lines = NULL;
    GPtrArray* current = ensure_section(table, ROOT_SECTION);
    int i;

    if (!contents)
        return;

    lines = g_strsplit(contents, "\n", -1);
    for (i = 0; lines[i]; i++) {
        char* line = strip(lines[i]);

        if (line[0] == '\0' || line[0] == '#' || line[0] == ';')
            continue;

        if (switch_current_section(table, line, &current))
            continue;
        append_entry(current, line);
    }
}

void gnoblin_config_reload(void) {
    g_autofree char *path = gnoblin_config_path ();
    g_autoptr(GError) error = NULL;
    GHashTable *table = section_table_new ();

    if (!g_str_has_suffix (path, ".conf")) {
        g_autoptr(GVariant) document = gnoblin_config_load_document (path, NULL, NULL, &error);
        if (!document) {
            g_warning ("gnoblin-config: %s", error ? error->message : "invalid configuration");
            g_hash_table_unref (table);
            return;
        }
        GVariantIter sections;
        const char *section;
        GVariant *values;
        g_variant_iter_init(&sections, document);
        while (g_variant_iter_next(&sections, "{&sv}", &section, &values)) {
            if (g_variant_is_of_type(values, G_VARIANT_TYPE_VARDICT)) {
                GVariantIter entries;
                const char *key;
                GVariant *value;
                g_variant_iter_init(&entries, values);
                while (g_variant_iter_next(&entries, "{&sv}", &key, &value)) {
                    ConfigEntry *entry = g_new0(ConfigEntry, 1);
                    entry->key = g_strdup(key);
                    if (g_variant_is_of_type(value, G_VARIANT_TYPE_STRING))
                        entry->value = g_variant_dup_string(value, NULL);
                    else
                        entry->value = g_variant_print(value, FALSE);
                    g_ptr_array_add(ensure_section(table, section), entry);
                    g_variant_unref(value);
                }
            }
            g_variant_unref(values);
        }
    } else {
        g_autofree char *contents = NULL;
        if (g_file_get_contents (path, &contents, NULL, &error)) {
            parse_into (table, contents);
        } else if (!g_error_matches (error, G_FILE_ERROR, G_FILE_ERROR_NOENT)) {
            g_warning ("gnoblin-config: %s", error->message);
            g_hash_table_unref (table);
            return;
        }
    }

    if (loaded_sections)
        g_hash_table_unref (loaded_sections);
    loaded_sections = table;
}

static GPtrArray* loaded_section(const char* name) {
    if (!loaded_sections)
        gnoblin_config_reload();
    return loaded_sections ? g_hash_table_lookup(loaded_sections, name ? name : ROOT_SECTION) : NULL;
}

/* Last value wins, so a later line overrides an earlier one. */
static const char* lookup_last_value(const char* section_name, const char* key) {
    GPtrArray* entries = loaded_section(section_name);
    const char* value = NULL;
    guint i;

    if (!entries)
        return NULL;
    for (i = 0; i < entries->len; i++) {
        ConfigEntry* e = g_ptr_array_index(entries, i);

        if (!strcmp(e->key, key))
            value = e->value;
    }
    return value;
}

gboolean gnoblin_config_get_bool(const char* section_name, const char* key, gboolean fallback) {
    const char* v = lookup_last_value(section_name, key);

    if (!v)
        return fallback;
    if (!g_ascii_strcasecmp(v, "true") || !strcmp(v, "1") || !g_ascii_strcasecmp(v, "yes") ||
        !g_ascii_strcasecmp(v, "on"))
        return TRUE;
    if (!g_ascii_strcasecmp(v, "false") || !strcmp(v, "0") || !g_ascii_strcasecmp(v, "no") ||
        !g_ascii_strcasecmp(v, "off"))
        return FALSE;
    return fallback;
}

int gnoblin_config_get_int(const char* section_name, const char* key, int fallback) {
    const char* v = lookup_last_value(section_name, key);
    char* end;
    long n;

    if (!v || v[0] == '\0')
        return fallback;
    errno = 0;
    n = strtol(v, &end, 10);
    if (end == v || *end != '\0' || errno == ERANGE || n < INT_MIN || n > INT_MAX)
        return fallback;
    return (int)n;
}

char* gnoblin_config_get_string(const char* section_name, const char* key) {
    const char* v = lookup_last_value(section_name, key);

    if (!v)
        return NULL;
    return g_strdup(v);
}

gboolean gnoblin_config_protocol_enabled(const char* key) {
    const char* mode = g_getenv("GNOME_SHELL_SESSION_MODE");

    if (g_strcmp0(mode, "gnoblin") != 0)
        return FALSE;
    return gnoblin_config_get_bool("protocols", key, TRUE);
}

char** gnoblin_config_get_list(const char* section_name, const char* key) {
    GPtrArray* entries = loaded_section(section_name);
    GPtrArray* out;
    guint i;

    if (!entries)
        return NULL;

    out = g_ptr_array_new();
    for (i = 0; i < entries->len; i++) {
        ConfigEntry* e = g_ptr_array_index(entries, i);

        if (strcmp(e->key, key))
            continue;
        g_ptr_array_add(out, g_strdup(e->value));
    }

    if (out->len == 0) {
        g_ptr_array_free(out, TRUE);
        return NULL;
    }
    g_ptr_array_add(out, NULL);
    return (char**)g_ptr_array_free(out, FALSE);
}

char** gnoblin_config_get_keys(const char* section_name) {
    GPtrArray* entries = loaded_section(section_name);
    GPtrArray* out;
    guint i;

    if (!entries || entries->len == 0)
        return NULL;

    out = g_ptr_array_new();
    for (i = 0; i < entries->len; i++) {
        ConfigEntry* e = g_ptr_array_index(entries, i);

        g_ptr_array_add(out, g_strdup(e->key));
    }
    g_ptr_array_add(out, NULL);
    return (char**)g_ptr_array_free(out, FALSE);
}
