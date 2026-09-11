/*
 * gnoblin: a tiny, dependency-free sectioned config file.
 *
 * This program is free software; you can redistribute it and/or modify it
 * under the terms of the GNU General Public License as published by the Free
 * Software Foundation; either version 2 of the License, or (at your option)
 * any later version.
 */

#include "gnoblin-config.h"

#include <errno.h>
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

static gboolean is_table (GVariant *value)
{
    return value && g_variant_is_of_type (value, G_VARIANT_TYPE_VARDICT);
}

static gboolean append_array_key (const char *key)
{
    return g_str_equal (key, "autostart") || g_str_equal (key, "window-rules") ||
           g_str_equal (key, "shortcuts") || g_str_equal (key, "rules");
}

static GVariant *merge_documents (GVariant *base, GVariant *overlay, const char *key);

static GVariant *merge_arrays (GVariant *base, GVariant *overlay)
{
    GVariantBuilder builder;
    GVariantIter iter;
    GVariant *value;

    g_variant_builder_init (&builder, G_VARIANT_TYPE ("av"));
    g_variant_iter_init (&iter, base);
    while (g_variant_iter_next (&iter, "v", &value)) {
        g_variant_builder_add (&builder, "v", value);
        g_variant_unref (value);
    }
    g_variant_iter_init (&iter, overlay);
    while (g_variant_iter_next (&iter, "v", &value)) {
        g_variant_builder_add (&builder, "v", value);
        g_variant_unref (value);
    }
    return g_variant_builder_end (&builder);
}

static GVariant *merge_documents (GVariant *base, GVariant *overlay, const char *key)
{
    if (is_table (base) && is_table (overlay)) {
        GVariantDict dictionary;
        GVariantIter iter;
        const char *name;
        GVariant *value;

        g_variant_dict_init (&dictionary, base);
        g_variant_iter_init (&iter, overlay);
        while (g_variant_iter_next (&iter, "{&sv}", &name, &value)) {
            g_autoptr(GVariant) previous = g_variant_dict_lookup_value (&dictionary, name, NULL);
            g_autoptr(GVariant) merged = previous ?
                merge_documents (previous, value, name) : g_variant_ref (value);
            g_variant_dict_insert_value (&dictionary, name, merged);
            g_variant_unref (value);
        }
        return g_variant_ref_sink (g_variant_dict_end (&dictionary));
    }
    if (base && overlay && g_variant_is_of_type (base, G_VARIANT_TYPE ("av")) &&
        g_variant_is_of_type (overlay, G_VARIANT_TYPE ("av")) && append_array_key (key))
        return merge_arrays (base, overlay);
    return g_variant_ref (overlay);
}

static gboolean include_value (GVariant *value, GPtrArray *paths, GError **error)
{
    GVariantIter iter;
    GVariant *entry;

    if (g_variant_is_of_type (value, G_VARIANT_TYPE_STRING)) {
        g_ptr_array_add (paths, g_variant_dup_string (value, NULL));
        return TRUE;
    }
    if (!g_variant_is_of_type (value, G_VARIANT_TYPE ("av"))) {
        g_set_error_literal (error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                             "include/source must be a path or an array of paths");
        return FALSE;
    }
    g_variant_iter_init (&iter, value);
    while (g_variant_iter_next (&iter, "v", &entry)) {
        if (!g_variant_is_of_type (entry, G_VARIANT_TYPE_STRING)) {
            g_variant_unref (entry);
            g_set_error_literal (error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                                 "include/source must contain only paths");
            return FALSE;
        }
        g_ptr_array_add (paths, g_variant_dup_string (entry, NULL));
        g_variant_unref (entry);
    }
    if (paths->len == 0) {
        g_set_error_literal (error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                             "include/source must contain at least one path");
        return FALSE;
    }
    return TRUE;
}

static char *resolve_include (const char *including, const char *value)
{
    g_autofree char *expanded = NULL;

    if (g_str_has_prefix (value, "~/"))
        expanded = g_build_filename (g_get_home_dir (), value + 2, NULL);
    else if (g_path_is_absolute (value))
        expanded = g_strdup (value);
    else
        expanded = g_build_filename (g_path_get_dirname (including), value, NULL);
    return g_canonicalize_filename (expanded, NULL);
}

static GVariant *load_toml_file (const char *path, GHashTable *stack, GError **error)
{
    g_autofree char *canonical = g_canonicalize_filename (path, NULL);
    g_autofree char *contents = NULL;
    g_autoptr(GVariant) document = NULL;
    g_autoptr(GVariant) merged = NULL;
    g_autoptr(GVariant) includes = NULL;
    g_autoptr(GVariant) source = NULL;
    g_autoptr(GPtrArray) paths = NULL;
    g_autoptr(GError) parse_error = NULL;

    if (g_hash_table_contains (stack, canonical)) {
        g_set_error (error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                     "%s: include cycle", canonical);
        return NULL;
    }
    if (g_hash_table_size (stack) >= 32) {
        g_set_error (error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                     "%s: include nesting exceeds 32 files", canonical);
        return NULL;
    }
    if (!g_file_get_contents (canonical, &contents, NULL, error)) {
        g_prefix_error (error, "%s: cannot read included config: ", canonical);
        return NULL;
    }
    document = gnoblin_config_parse_toml (contents, &parse_error);
    if (!document) {
        g_propagate_prefixed_error (error, g_steal_pointer (&parse_error),
                                    "%s: invalid TOML: ", canonical);
        return NULL;
    }
    includes = g_variant_lookup_value (document, "include", NULL);
    source = g_variant_lookup_value (document, "source", NULL);
    if (includes && source) {
        g_set_error_literal (error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                             "use either include or source, not both");
        return NULL;
    }
    paths = g_ptr_array_new_with_free_func (g_free);
    if (includes && !include_value (includes, paths, error))
        return NULL;
    if (source && !include_value (source, paths, error))
        return NULL;

    g_hash_table_add (stack, g_strdup (canonical));
    GVariantBuilder empty;
    g_variant_builder_init (&empty, G_VARIANT_TYPE_VARDICT);
    merged = g_variant_ref_sink (g_variant_builder_end (&empty));
    for (guint i = 0; i < paths->len; i++) {
        g_autofree char *child = resolve_include (canonical, g_ptr_array_index (paths, i));
        g_autoptr(GVariant) included = load_toml_file (child, stack, error);
        g_autoptr(GVariant) next = NULL;
        if (!included) {
            g_hash_table_remove (stack, canonical);
            return NULL;
        }
        next = merge_documents (merged, included, NULL);
        g_clear_pointer (&merged, g_variant_unref);
        merged = g_steal_pointer (&next);
    }
    g_hash_table_remove (stack, canonical);

    GVariantDict local;
    g_variant_dict_init (&local, document);
    g_variant_dict_remove (&local, "include");
    g_variant_dict_remove (&local, "source");
    g_autoptr(GVariant) local_document = g_variant_ref_sink (g_variant_dict_end (&local));
    g_autoptr(GVariant) result = merge_documents (merged, local_document, NULL);
    return g_steal_pointer (&result);
}

const char* gnoblin_config_path(void) {
    static char* path;

    if (!path) {
        const char* override = g_getenv("GNOBLIN_CONFIG");

        if (override && override[0])
            path = g_strdup(override);
        else {
            path = g_build_filename(g_get_user_config_dir(), "gnoblin", "gnoblin.toml", NULL);
            if (!g_file_test(path, G_FILE_TEST_EXISTS)) {
                char *legacy = g_build_filename(g_get_user_config_dir(), "gnoblin", "gnoblin.conf", NULL);
                if (g_file_test(legacy, G_FILE_TEST_EXISTS)) {
                    g_free(path);
                    path = legacy;
                } else {
                    g_free(legacy);
                }
            }
        }
    }

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
    g_autofree char* contents = NULL;
    GHashTable* table;

    table = section_table_new();

    if (g_file_get_contents(gnoblin_config_path(), &contents, NULL, NULL)) {
        if (!g_str_has_suffix(gnoblin_config_path(), ".conf")) {
            g_autoptr(GError) error = NULL;
            g_autoptr(GHashTable) stack = g_hash_table_new_full(g_str_hash, g_str_equal, g_free, NULL);
            g_autoptr(GVariant) document = load_toml_file(gnoblin_config_path(), stack, &error);
            if (!document) {
                g_warning("gnoblin-config: %s", error->message);
                g_hash_table_unref(table);
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
            parse_into(table, contents);
        }
    }

    if (loaded_sections)
        g_hash_table_unref(loaded_sections);
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
