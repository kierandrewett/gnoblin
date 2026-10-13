#include "gnoblin-config-internal.h"

GVariant* gnoblin_config_load_document(const char* path, GPtrArray** paths, GPtrArray** directories,
                                       GError** error) {
    g_autoptr(GPtrArray) loaded_paths = g_ptr_array_new_with_free_func(g_free);
    g_autoptr(GPtrArray) watched_dirs = g_ptr_array_new_with_free_func(g_free);
    g_autofree char* canonical = g_canonicalize_filename(path, NULL);
    GVariant* document = NULL;
    g_ptr_array_add(loaded_paths, g_strdup(canonical));

    if (!g_str_has_suffix(canonical, ".lua")) {
        g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                    "%s: Gnoblin configuration uses Lua only. Convert this file to Lua and "
                    "give it a .lua filename.",
                    canonical);
    } else if (!g_file_test(canonical, G_FILE_TEST_EXISTS)) {
        document = gnoblin_config_default_document();
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

static char* last_good_document_path(void) {
    return g_build_filename(g_get_user_state_dir(), "gnoblin", "last-good-config.gv", NULL);
}

/* Mutter reads some enum values by name and treats an unknown name as its fallback. A last-good
 * document that an older Gnoblin saved can still hold the deprecated spellings, which would then be
 * applied wrongly. Report them so the caller can refuse the document instead. */
static gboolean document_has_deprecated_enum_values(GVariant* document) {
    static const struct {
        const char* section;
        const char* group;
        const char* key;
        const char* old_values[3];
    } checks[] = {
        {"window-management", NULL, "focus-mode", {"sloppy", "mouse", NULL}},
        {"window-management", NULL, "focus-new-windows", {"smart", "strict", NULL}},
        {"input", "touchpad", "left-handed", {"mouse", NULL, NULL}},
        {"input", "touchpad", "tap-button-map", {"lrm", "lmr", NULL}},
    };
    for (guint i = 0; i < G_N_ELEMENTS(checks); i++) {
        g_autoptr(GVariant) container =
            g_variant_lookup_value(document, checks[i].section, G_VARIANT_TYPE_VARDICT);
        if (container && checks[i].group) {
            g_autoptr(GVariant) group =
                g_variant_lookup_value(container, checks[i].group, G_VARIANT_TYPE_VARDICT);
            g_variant_unref(container);
            container = g_steal_pointer(&group);
        }
        const char* value = NULL;
        if (!container || !g_variant_lookup(container, checks[i].key, "&s", &value))
            continue;
        for (guint j = 0; j < G_N_ELEMENTS(checks[i].old_values) && checks[i].old_values[j]; j++) {
            if (g_str_equal(value, checks[i].old_values[j]))
                return TRUE;
        }
    }
    return FALSE;
}

GVariant* gnoblin_config_load_last_good_document(const char* config_path, GError** error) {
    g_autofree char* cache_path = last_good_document_path();
    g_autofree char* directory = g_path_get_dirname(cache_path);
    g_autofree char* canonical = g_canonicalize_filename(config_path, NULL);
    g_autofree char* contents = NULL;
    gsize length = 0;

    struct stat directory_stat;
    if (g_lstat(directory, &directory_stat) != 0) {
        g_set_error(error, G_FILE_ERROR, g_file_error_from_errno(errno),
                    "could not inspect Gnoblin state directory: %s", g_strerror(errno));
        return NULL;
    }
    if (!S_ISDIR(directory_stat.st_mode) || directory_stat.st_uid != getuid() ||
        (directory_stat.st_mode & 0077) != 0) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_ACCES,
                            "Gnoblin state directory is not private to this user");
        return NULL;
    }

    struct stat cache_stat;
    if (g_lstat(cache_path, &cache_stat) != 0) {
        g_set_error(error, G_FILE_ERROR, g_file_error_from_errno(errno),
                    "could not inspect saved Gnoblin configuration: %s", g_strerror(errno));
        return NULL;
    }
    if (!S_ISREG(cache_stat.st_mode) || cache_stat.st_uid != getuid() ||
        (cache_stat.st_mode & 0077) != 0) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_ACCES,
                            "saved Gnoblin configuration is not a private regular file");
        return NULL;
    }
    if (!g_file_get_contents(cache_path, &contents, &length, error))
        return NULL;
    if (length == 0 || length > LAST_GOOD_DOCUMENT_MAX_BYTES) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                            "saved Gnoblin configuration has an invalid size");
        return NULL;
    }

    g_autoptr(GBytes) bytes = g_bytes_new_take(g_steal_pointer(&contents), length);
    g_autoptr(GVariant) envelope =
        g_variant_ref_sink(g_variant_new_from_bytes(G_VARIANT_TYPE_VARDICT, bytes, FALSE));
    if (!g_variant_is_normal_form(envelope)) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                            "saved Gnoblin configuration is malformed");
        return NULL;
    }
    guint32 version = 0;
    const char* saved_path = NULL;
    g_autoptr(GVariant) document =
        g_variant_lookup_value(envelope, "document", G_VARIANT_TYPE_VARDICT);
    if (!g_variant_lookup(envelope, "version", "u", &version) || version != 1 ||
        !g_variant_lookup(envelope, "source", "&s", &saved_path) ||
        !g_str_equal(saved_path, canonical) || !document) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                            "saved Gnoblin configuration does not match this config path");
        return NULL;
    }
    if (!gnoblin_config_validate_document(document, error))
        return NULL;
    if (document_has_deprecated_enum_values(document)) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                            "saved Gnoblin configuration uses setting value names from an older "
                            "Gnoblin and was not applied");
        return NULL;
    }
    return g_steal_pointer(&document);
}

gboolean gnoblin_config_save_last_good_document(const char* config_path, GVariant* document,
                                                GError** error) {
    if (!config_path || !document || !g_variant_is_of_type(document, G_VARIANT_TYPE_VARDICT) ||
        !gnoblin_config_validate_document(document, error))
        return FALSE;

    GVariantBuilder envelope_builder;
    g_variant_builder_init(&envelope_builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&envelope_builder, "{sv}", "version", g_variant_new_uint32(1));
    g_autofree char* canonical = g_canonicalize_filename(config_path, NULL);
    g_variant_builder_add(&envelope_builder, "{sv}", "source", g_variant_new_string(canonical));
    g_variant_builder_add(&envelope_builder, "{sv}", "document", document);
    g_autoptr(GVariant) envelope = g_variant_ref_sink(g_variant_builder_end(&envelope_builder));
    g_autoptr(GBytes) bytes = g_variant_get_data_as_bytes(envelope);
    gsize length = 0;
    gconstpointer data = g_bytes_get_data(bytes, &length);

    g_autofree char* cache_path = last_good_document_path();
    g_autofree char* directory = g_path_get_dirname(cache_path);
    if (g_mkdir_with_parents(directory, 0700) != 0 || g_chmod(directory, 0700) != 0) {
        g_set_error(error, G_FILE_ERROR, g_file_error_from_errno(errno),
                    "could not secure Gnoblin state directory %s: %s", directory,
                    g_strerror(errno));
        return FALSE;
    }
    return g_file_set_contents_full(cache_path, data, (gssize)length,
                                    G_FILE_SET_CONTENTS_CONSISTENT, 0600, error);
}
