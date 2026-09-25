/* Shared Lua configuration loading and native setting accessors. */
#include "gnoblin-config.h"

#include <errno.h>
#include <glib/gstdio.h>
#include <limits.h>
#include <stdlib.h>
#include <string.h>

typedef struct {
    char* key;
    char* value;
} ConfigEntry;

static GHashTable* loaded_sections;

static void config_entry_free(gpointer data) {
    ConfigEntry* entry = data;
    g_free(entry->key);
    g_free(entry->value);
    g_free(entry);
}

static GPtrArray* ensure_section(GHashTable* sections, const char* name) {
    GPtrArray* entries = g_hash_table_lookup(sections, name);
    if (!entries) {
        entries = g_ptr_array_new_with_free_func(config_entry_free);
        g_hash_table_insert(sections, g_strdup(name), entries);
    }
    return entries;
}

gboolean gnoblin_config_validate_document(GVariant* document, GError** error) {
    g_autoptr(GVariant) workspaces = g_variant_lookup_value(document, "workspaces", NULL);
    if (workspaces) {
        gboolean valid = g_variant_is_of_type(workspaces, G_VARIANT_TYPE("av")) &&
                         g_variant_n_children(workspaces) >= 1;
        GHashTable* ids = g_hash_table_new_full(g_str_hash, g_str_equal, g_free, NULL);
        for (gsize i = 0; valid && i < g_variant_n_children(workspaces); i++) {
            g_autoptr(GVariant) boxed = g_variant_get_child_value(workspaces, i);
            g_autoptr(GVariant) entry = g_variant_get_variant(boxed);
            if (!g_variant_is_of_type(entry, G_VARIANT_TYPE_VARDICT) ||
                g_variant_n_children(entry) != 2) {
                valid = FALSE;
                break;
            }
            g_autoptr(GVariant) id_value = g_variant_lookup_value(entry, "id", NULL);
            g_autoptr(GVariant) name_value = g_variant_lookup_value(entry, "name", NULL);
            const char* id = id_value && g_variant_is_of_type(id_value, G_VARIANT_TYPE_STRING)
                                 ? g_variant_get_string(id_value, NULL)
                                 : "";
            const char* name = name_value && g_variant_is_of_type(name_value, G_VARIANT_TYPE_STRING)
                                   ? g_variant_get_string(name_value, NULL)
                                   : "";
            valid = id_value && name_value &&
                    g_regex_match_simple("^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$", id, G_REGEX_OPTIMIZE,
                                         G_REGEX_MATCH_NOTEMPTY) &&
                    *name && g_utf8_validate(name, -1, NULL) && g_utf8_strlen(name, -1) <= 80 &&
                    !g_hash_table_contains(ids, id);
            if (valid)
                g_hash_table_add(ids, g_strdup(id));
        }
        g_hash_table_unref(ids);
        if (!valid) {
            g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                                "workspaces must be a nonempty array of objects with unique valid "
                                "ids and nonempty names up to 80 characters");
            return FALSE;
        }
    }
    const char* sections[] = {"protocols", "layer-shell"};
    const char* keys[] = {NULL, "preserve-active-window"};
    for (guint i = 0; i < G_N_ELEMENTS(sections); i++) {
        g_autoptr(GVariant) section = g_variant_lookup_value(document, sections[i], NULL);
        if (!section)
            continue;
        gboolean valid = g_variant_is_of_type(section, G_VARIANT_TYPE_VARDICT);
        GVariantIter iter;
        const char* name;
        GVariant* value;
        if (valid)
            g_variant_iter_init(&iter, section);
        while (valid && g_variant_iter_next(&iter, "{&sv}", &name, &value)) {
            valid = g_variant_is_of_type(value, G_VARIANT_TYPE_BOOLEAN) &&
                    (!keys[i] || g_str_equal(name, keys[i]));
            g_variant_unref(value);
        }
        if (!valid) {
            g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                        "%s must be a table of supported boolean settings", sections[i]);
            return FALSE;
        }
    }
    g_autoptr(GVariant) window = g_variant_lookup_value(document, "window-management", NULL);
    if (window) {
        if (g_variant_is_of_type(window, G_VARIANT_TYPE_VARDICT)) {
            g_autoptr(GVariant) legacy_ids = g_variant_lookup_value(window, "workspace-ids", NULL);
            g_autoptr(GVariant) legacy_names =
                g_variant_lookup_value(window, "workspace-names", NULL);
            g_autoptr(GVariant) legacy_dynamic =
                g_variant_lookup_value(window, "dynamic-workspaces", NULL);
            g_autoptr(GVariant) legacy_count =
                g_variant_lookup_value(window, "num-workspaces", NULL);
            gboolean has_legacy_workspaces =
                legacy_ids || legacy_names || legacy_dynamic || legacy_count;
            if (workspaces && has_legacy_workspaces) {
                g_set_error_literal(
                    error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                    "use top-level workspaces without window-management workspace-ids, "
                    "workspace-names, dynamic-workspaces, or num-workspaces");
                return FALSE;
            }
            if (!workspaces && has_legacy_workspaces)
                g_warning("gnoblin-config: window-management workspace settings are deprecated; "
                          "use top-level workspaces = {{id = \"...\", name = \"...\"}, ...}");
        }
        static const char* booleans[] = {
            "constrain-drag-to-work-area",
            "raise-on-click",
            "auto-raise",
            "focus-change-on-pointer-rest",
            "dynamic-workspaces",
            "workspaces-only-on-primary",
            "edge-tiling",
            "center-new-windows",
            "attach-modal-dialogs",
            NULL,
        };
        static const char* titlebar[] = {
            "toggle-maximize",
            "toggle-maximize-horizontally",
            "toggle-maximize-vertically",
            "minimize",
            "none",
            "lower",
            "menu",
            NULL,
        };
        gboolean valid = g_variant_is_of_type(window, G_VARIANT_TYPE_VARDICT);
        GVariantIter iter;
        const char* name;
        GVariant* value;
        if (valid)
            g_variant_iter_init(&iter, window);
        while (valid && g_variant_iter_next(&iter, "{&sv}", &name, &value)) {
            gboolean known_boolean = FALSE;
            for (guint i = 0; booleans[i]; i++)
                known_boolean |= g_str_equal(name, booleans[i]);
            if (known_boolean) {
                valid = g_variant_is_of_type(value, G_VARIANT_TYPE_BOOLEAN);
            } else if (g_str_equal(name, "workspace-names")) {
                valid = g_variant_is_of_type(value, G_VARIANT_TYPE("av")) &&
                        g_variant_n_children(value) <= 36;
                for (gsize i = 0; valid && i < g_variant_n_children(value); i++) {
                    g_autoptr(GVariant) boxed = g_variant_get_child_value(value, i);
                    g_autoptr(GVariant) item = g_variant_get_variant(boxed);
                    valid = g_variant_is_of_type(item, G_VARIANT_TYPE_STRING) &&
                            g_utf8_strlen(g_variant_get_string(item, NULL), -1) <= 80;
                }
            } else if (g_str_equal(name, "workspace-ids")) {
                valid = g_variant_is_of_type(value, G_VARIANT_TYPE("av")) &&
                        g_variant_n_children(value) <= 36;
                GHashTable* ids = g_hash_table_new_full(g_str_hash, g_str_equal, g_free, NULL);
                for (gsize i = 0; valid && i < g_variant_n_children(value); i++) {
                    g_autoptr(GVariant) boxed = g_variant_get_child_value(value, i);
                    g_autoptr(GVariant) item = g_variant_get_variant(boxed);
                    const char* id = g_variant_is_of_type(item, G_VARIANT_TYPE_STRING)
                                         ? g_variant_get_string(item, NULL)
                                         : "";
                    valid = g_regex_match_simple("^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$", id,
                                                 G_REGEX_OPTIMIZE, G_REGEX_MATCH_NOTEMPTY) &&
                            !g_hash_table_contains(ids, id);
                    if (valid)
                        g_hash_table_add(ids, g_strdup(id));
                }
                g_hash_table_unref(ids);
            } else if (g_str_equal(name, "auto-raise-delay") ||
                       g_str_equal(name, "num-workspaces")) {
                gint64 number =
                    g_variant_is_of_type(value, G_VARIANT_TYPE_INT32)   ? g_variant_get_int32(value)
                    : g_variant_is_of_type(value, G_VARIANT_TYPE_INT64) ? g_variant_get_int64(value)
                                                                        : -1;
                valid = g_str_equal(name, "auto-raise-delay") ? number >= 0 && number <= 10000
                                                              : number >= 1 && number <= 36;
            } else if (g_variant_is_of_type(value, G_VARIANT_TYPE_STRING)) {
                const char* string = g_variant_get_string(value, NULL);
                if (g_str_equal(name, "focus-mode"))
                    valid = g_str_equal(string, "click") || g_str_equal(string, "sloppy") ||
                            g_str_equal(string, "mouse");
                else if (g_str_equal(name, "focus-new-windows"))
                    valid = g_str_equal(string, "smart") || g_str_equal(string, "strict");
                else if (g_str_equal(name, "action-double-click-titlebar") ||
                         g_str_equal(name, "action-middle-click-titlebar") ||
                         g_str_equal(name, "action-right-click-titlebar")) {
                    valid = FALSE;
                    for (guint i = 0; titlebar[i]; i++)
                        valid |= g_str_equal(string, titlebar[i]);
                } else
                    valid = FALSE;
            } else
                valid = FALSE;
            g_variant_unref(value);
        }
        if (!valid) {
            g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                                "window-management contains an unsupported name or value");
            return FALSE;
        }
    }
    g_autoptr(GVariant) compositor = g_variant_lookup_value(document, "compositor", NULL);
    if (compositor) {
        static const char* booleans[] = {
            "enable-animations", "locate-pointer", "visual-bell", "audible-bell", NULL,
        };
        gboolean valid = g_variant_is_of_type(compositor, G_VARIANT_TYPE_VARDICT);
        GVariantIter iter;
        const char* name;
        GVariant* value;
        if (valid)
            g_variant_iter_init(&iter, compositor);
        while (valid && g_variant_iter_next(&iter, "{&sv}", &name, &value)) {
            gboolean known_boolean = FALSE;
            for (guint i = 0; booleans[i]; i++)
                known_boolean |= g_str_equal(name, booleans[i]);
            if (known_boolean)
                valid = g_variant_is_of_type(value, G_VARIANT_TYPE_BOOLEAN);
            else if (g_str_equal(name, "visual-bell-type") &&
                     g_variant_is_of_type(value, G_VARIANT_TYPE_STRING)) {
                const char* bell = g_variant_get_string(value, NULL);
                valid = g_str_equal(bell, "fullscreen-flash") || g_str_equal(bell, "frame-flash");
            } else
                valid = FALSE;
            g_variant_unref(value);
        }
        if (!valid) {
            g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                                "compositor contains an unsupported name or value");
            return FALSE;
        }
    }
    g_autoptr(GVariant) cursor = g_variant_lookup_value(document, "cursor", NULL);
    if (cursor) {
        gboolean valid = g_variant_is_of_type(cursor, G_VARIANT_TYPE_VARDICT);
        GVariantIter iter;
        const char* name;
        GVariant* value;
        if (valid)
            g_variant_iter_init(&iter, cursor);
        while (valid && g_variant_iter_next(&iter, "{&sv}", &name, &value)) {
            if (g_str_equal(name, "theme")) {
                if (g_variant_is_of_type(value, G_VARIANT_TYPE_STRING)) {
                    g_autofree char* theme = g_variant_dup_string(value, NULL);
                    valid = *g_strstrip(theme) != '\0';
                } else {
                    valid = FALSE;
                }
            } else if (g_str_equal(name, "size")) {
                gint64 size =
                    g_variant_is_of_type(value, G_VARIANT_TYPE_INT32)   ? g_variant_get_int32(value)
                    : g_variant_is_of_type(value, G_VARIANT_TYPE_INT64) ? g_variant_get_int64(value)
                                                                        : 0;
                valid = size >= 1 && size <= 256 &&
                        (g_variant_is_of_type(value, G_VARIANT_TYPE_INT32) ||
                         g_variant_is_of_type(value, G_VARIANT_TYPE_INT64));
            } else {
                valid = FALSE;
            }
            g_variant_unref(value);
        }
        if (!valid) {
            g_set_error_literal(
                error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                "cursor must contain only a nonempty theme and a size from 1 to 256");
            return FALSE;
        }
    }
    return TRUE;
}

GVariant* gnoblin_config_load_document(const char* path, GPtrArray** paths, GPtrArray** directories,
                                       GError** error) {
    g_autoptr(GPtrArray) loaded_paths = g_ptr_array_new_with_free_func(g_free);
    g_autoptr(GPtrArray) watched_dirs = g_ptr_array_new_with_free_func(g_free);
    g_autofree char* canonical = g_canonicalize_filename(path, NULL);
    GVariant* document = NULL;
    g_ptr_array_add(loaded_paths, g_strdup(canonical));

    if (!g_file_test(canonical, G_FILE_TEST_EXISTS)) {
        GVariantBuilder empty;
        g_variant_builder_init(&empty, G_VARIANT_TYPE_VARDICT);
        document = g_variant_ref_sink(g_variant_builder_end(&empty));
    } else {
        document = gnoblin_config_evaluate_file(canonical, loaded_paths, watched_dirs, error);
        if (document)
            g_variant_ref_sink(document);
        if (document && !g_variant_is_of_type(document, G_VARIANT_TYPE_VARDICT)) {
            g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                                "configuration must be a table of settings");
            g_clear_pointer(&document, g_variant_unref);
        }
        if (document && !gnoblin_config_validate_document(document, error))
            g_clear_pointer(&document, g_variant_unref);
    }
    if (paths)
        *paths = g_steal_pointer(&loaded_paths);
    if (directories)
        *directories = g_steal_pointer(&watched_dirs);
    return document;
}

char* gnoblin_config_path(void) {
    const char* override = g_getenv("GNOBLIN_CONFIG");
    if (override && override[0])
        return g_canonicalize_filename(override, NULL);
    g_autofree char* directory = g_build_filename(g_get_user_config_dir(), "gnoblin", NULL);
    /* Keep existing installations working after the Lua-root migration. */
    const char* names[] = {"init.lua", "gnoblin.toml", "gnoblin.conf"};
    for (guint i = 0; i < G_N_ELEMENTS(names); i++) {
        g_autofree char* candidate = g_build_filename(directory, names[i], NULL);
        if (g_file_test(candidate, G_FILE_TEST_EXISTS))
            return g_steal_pointer(&candidate);
    }
    return g_build_filename(directory, "init.lua", NULL);
}

void gnoblin_config_reload(void) {
    g_autofree char* path = gnoblin_config_path();
    g_autoptr(GError) error = NULL;
    g_autoptr(GVariant) document = gnoblin_config_load_document(path, NULL, NULL, &error);
    if (!document) {
        g_warning("gnoblin-config: %s", error ? error->message : "invalid configuration");
        return;
    }
    GHashTable* sections =
        g_hash_table_new_full(g_str_hash, g_str_equal, g_free, (GDestroyNotify)g_ptr_array_unref);
    GVariantIter section_iter;
    const char* section;
    GVariant* values;
    g_variant_iter_init(&section_iter, document);
    while (g_variant_iter_next(&section_iter, "{&sv}", &section, &values)) {
        if (g_variant_is_of_type(values, G_VARIANT_TYPE_VARDICT)) {
            GVariantIter value_iter;
            const char* key;
            GVariant* value;
            g_variant_iter_init(&value_iter, values);
            while (g_variant_iter_next(&value_iter, "{&sv}", &key, &value)) {
                ConfigEntry* entry = g_new0(ConfigEntry, 1);
                entry->key = g_strdup(key);
                entry->value = g_variant_is_of_type(value, G_VARIANT_TYPE_STRING)
                                   ? g_variant_dup_string(value, NULL)
                                   : g_variant_print(value, FALSE);
                g_ptr_array_add(ensure_section(sections, section), entry);
                g_variant_unref(value);
            }
        }
        g_variant_unref(values);
    }
    g_clear_pointer(&loaded_sections, g_hash_table_unref);
    loaded_sections = sections;
}

static const char* lookup_last_value(const char* section, const char* key) {
    if (!loaded_sections)
        gnoblin_config_reload();
    GPtrArray* entries = loaded_sections ? g_hash_table_lookup(loaded_sections, section) : NULL;
    const char* value = NULL;
    for (guint i = 0; entries && i < entries->len; i++) {
        ConfigEntry* entry = g_ptr_array_index(entries, i);
        if (!strcmp(entry->key, key))
            value = entry->value;
    }
    return value;
}

gboolean gnoblin_config_get_bool(const char* section, const char* key, gboolean fallback) {
    const char* value = lookup_last_value(section, key);
    if (!value)
        return fallback;
    if (!g_ascii_strcasecmp(value, "true") || !strcmp(value, "1"))
        return TRUE;
    if (!g_ascii_strcasecmp(value, "false") || !strcmp(value, "0"))
        return FALSE;
    return fallback;
}

int gnoblin_config_get_int(const char* section, const char* key, int fallback) {
    const char* value = lookup_last_value(section, key);
    char* end;
    long number;
    if (!value)
        return fallback;
    errno = 0;
    number = strtol(value, &end, 10);
    return errno || end == value || *end || number < INT_MIN || number > INT_MAX ? fallback
                                                                                 : number;
}

char* gnoblin_config_get_string(const char* section, const char* key) {
    const char* value = lookup_last_value(section, key);
    return value ? g_strdup(value) : NULL;
}

gboolean gnoblin_config_protocol_enabled(const char* key) {
    return g_strcmp0(g_getenv("GNOME_SHELL_SESSION_MODE"), "gnoblin") == 0 &&
           gnoblin_config_get_bool("protocols", key, TRUE);
}

char** gnoblin_config_get_list(const char* section, const char* key) {
    const char* value = lookup_last_value(section, key);
    if (!value)
        return NULL;
    char** list = g_new0(char*, 2);
    list[0] = g_strdup(value);
    return list;
}

char** gnoblin_config_get_keys(const char* section) {
    if (!loaded_sections)
        gnoblin_config_reload();
    GPtrArray* entries = loaded_sections ? g_hash_table_lookup(loaded_sections, section) : NULL;
    if (!entries)
        return NULL;
    char** keys = g_new0(char*, entries->len + 1);
    for (guint i = 0; i < entries->len; i++)
        keys[i] = g_strdup(((ConfigEntry*)g_ptr_array_index(entries, i))->key);
    return keys;
}
