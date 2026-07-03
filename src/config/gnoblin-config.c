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
 * gnoblin.conf grammar, kept byte-compatible with
 * src/clients/shell-ui/src/config.rs:
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

const char* gnoblin_config_path(void) {
    static char* path;

    if (!path) {
        const char* override = g_getenv("GNOBLIN_CONFIG");

        if (override && override[0])
            path = g_strdup(override);
        else
            path = g_build_filename(g_get_user_config_dir(), "gnoblin", "gnoblin.conf", NULL);
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

    if (g_file_get_contents(gnoblin_config_path(), &contents, NULL, NULL))
        parse_into(table, contents);

    if (loaded_sections)
        g_hash_table_unref(loaded_sections);
    loaded_sections = table;
}

static GPtrArray* loaded_section(const char* name) {
    if (!loaded_sections)
        gnoblin_config_reload();
    return g_hash_table_lookup(loaded_sections, name ? name : ROOT_SECTION);
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
