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

typedef struct {
    char* key;
    char* value;
} Entry;

/* section name -> GPtrArray<Entry*> (in file order, repeats allowed) */
static GHashTable* sections;

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

static void entry_free(gpointer data) {
    Entry* e = data;

    g_free(e->key);
    g_free(e->value);
    g_free(e);
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

void gnoblin_config_reload(void) {
    g_autofree char* contents = NULL;
    g_auto(GStrv) lines = NULL;
    GHashTable* table;
    GPtrArray* current;
    int i;

    table =
        g_hash_table_new_full(g_str_hash, g_str_equal, g_free, (GDestroyNotify)g_ptr_array_unref);

    /* Top-level (before any [section]) lives under "". */
    current = g_ptr_array_new_with_free_func(entry_free);
    g_hash_table_replace(table, g_strdup(""), current);

    if (g_file_get_contents(gnoblin_config_path(), &contents, NULL, NULL)) {
        lines = g_strsplit(contents, "\n", -1);
        for (i = 0; lines[i]; i++) {
            char* line = strip(lines[i]);
            char* eq;
            Entry* e;

            if (line[0] == '\0' || line[0] == '#' || line[0] == ';')
                continue;

            if (line[0] == '[') {
                char* close = strchr(line, ']');
                g_autofree char* name = NULL;

                if (!close)
                    continue;
                *close = '\0';
                name = g_strdup(strip(line + 1));

                current = g_hash_table_lookup(table, name);
                if (!current) {
                    current = g_ptr_array_new_with_free_func(entry_free);
                    g_hash_table_replace(table, g_strdup(name), current);
                }
                continue;
            }

            eq = strchr(line, '=');
            if (!eq)
                continue;
            *eq = '\0';

            e = g_new0(Entry, 1);
            e->key = g_strdup(strip(line));
            e->value = g_strdup(clean_value(eq + 1));
            if (e->key[0] == '\0') {
                entry_free(e);
                continue;
            }
            g_ptr_array_add(current, e);
        }
    }

    if (sections)
        g_hash_table_unref(sections);
    sections = table;
}

static GPtrArray* section(const char* name) {
    if (!sections)
        gnoblin_config_reload();
    return g_hash_table_lookup(sections, name ? name : "");
}

/* Last value wins, so a later line overrides an earlier one. */
static const char* lookup(const char* section_name, const char* key) {
    GPtrArray* entries = section(section_name);
    const char* value = NULL;
    guint i;

    if (!entries)
        return NULL;
    for (i = 0; i < entries->len; i++) {
        Entry* e = g_ptr_array_index(entries, i);

        if (!strcmp(e->key, key))
            value = e->value;
    }
    return value;
}

gboolean gnoblin_config_get_bool(const char* section_name, const char* key, gboolean fallback) {
    const char* v = lookup(section_name, key);

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
    const char* v = lookup(section_name, key);
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
    const char* v = lookup(section_name, key);
    g_autofree char* dup = NULL;

    if (!v)
        return NULL;
    dup = g_strdup(v);
    return g_steal_pointer(&dup);
}

char** gnoblin_config_get_list(const char* section_name, const char* key) {
    GPtrArray* entries = section(section_name);
    GPtrArray* out;
    guint i;

    if (!entries)
        return NULL;

    out = g_ptr_array_new();
    for (i = 0; i < entries->len; i++) {
        Entry* e = g_ptr_array_index(entries, i);
        g_autofree char* dup = NULL;

        if (strcmp(e->key, key))
            continue;
        dup = g_strdup(e->value);
        g_ptr_array_add(out, g_steal_pointer(&dup));
    }

    if (out->len == 0) {
        g_ptr_array_free(out, TRUE);
        return NULL;
    }
    g_ptr_array_add(out, NULL);
    return (char**)g_ptr_array_free(out, FALSE);
}

char** gnoblin_config_get_keys(const char* section_name) {
    GPtrArray* entries = section(section_name);
    GPtrArray* out;
    guint i;

    if (!entries || entries->len == 0)
        return NULL;

    out = g_ptr_array_new();
    for (i = 0; i < entries->len; i++) {
        Entry* e = g_ptr_array_index(entries, i);

        g_ptr_array_add(out, g_strdup(e->key));
    }
    g_ptr_array_add(out, NULL);
    return (char**)g_ptr_array_free(out, FALSE);
}
