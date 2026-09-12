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

static gboolean validate_document(GVariant* document, GError** error) {
    const char* sections[] = {"protocols", "layer-shell", "window-management"};
    const char* keys[] = {NULL, "preserve-active-window", "constrain-drag-to-work-area"};
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
        if (document && !validate_document(document, error))
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
    return g_build_filename(g_get_user_config_dir(), "gnoblin", "init.lua", NULL);
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
