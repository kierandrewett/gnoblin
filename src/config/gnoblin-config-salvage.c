#include "gnoblin-config-internal.h"

static gboolean salvage_is_container(GVariant* node) {
    return g_variant_is_of_type(node, G_VARIANT_TYPE_VARDICT) ||
           g_variant_is_of_type(node, G_VARIANT_TYPE("av"));
}

/* Unboxed child of a map or list node. Returns a new reference. */
static GVariant* salvage_child(GVariant* node, guint index) {
    g_autoptr(GVariant) item = g_variant_get_child_value(node, index);
    if (g_variant_is_of_type(node, G_VARIANT_TYPE_VARDICT)) {
        g_autoptr(GVariant) boxed = g_variant_get_child_value(item, 1);
        return g_variant_get_variant(boxed);
    }
    return g_variant_get_variant(item);
}

/* Copy a map or list node, skipping one child and/or replacing one child. */
static GVariant* salvage_rebuild(GVariant* node, guint skip, guint replace, GVariant* replacement) {
    gboolean is_map = g_variant_is_of_type(node, G_VARIANT_TYPE_VARDICT);
    GVariantBuilder builder;
    g_variant_builder_init(&builder, is_map ? G_VARIANT_TYPE_VARDICT : G_VARIANT_TYPE("av"));
    gsize count = g_variant_n_children(node);
    for (gsize i = 0; i < count; i++) {
        if (i == skip)
            continue;
        g_autoptr(GVariant) value =
            i == replace ? g_variant_ref(replacement) : salvage_child(node, i);
        if (is_map) {
            g_autoptr(GVariant) item = g_variant_get_child_value(node, i);
            g_autoptr(GVariant) key = g_variant_get_child_value(item, 0);
            g_variant_builder_add(&builder, "{sv}", g_variant_get_string(key, NULL), value);
        } else {
            g_variant_builder_add(&builder, "v", value);
        }
    }
    return g_variant_ref_sink(g_variant_builder_end(&builder));
}

/* Copy of `node` without the child that `path` names. */
static GVariant* salvage_without(GVariant* node, const guint* path, guint depth, guint length) {
    if (depth + 1 == length)
        return salvage_rebuild(node, path[depth], G_MAXUINT, NULL);
    g_autoptr(GVariant) child = salvage_child(node, path[depth]);
    if (!salvage_is_container(child))
        return NULL;
    g_autoptr(GVariant) changed = salvage_without(child, path, depth + 1, length);
    return changed ? salvage_rebuild(node, G_MAXUINT, path[depth], changed) : NULL;
}

/* NULL when the document is valid, otherwise the validation error text. */
static char* salvage_error_text(GVariant* document) {
    g_autoptr(GError) error = NULL;
    if (gnoblin_config_validate_document(document, &error))
        return NULL;
    return g_strdup(error && error->message ? error->message : "invalid configuration");
}

static gboolean salvage_find_culprit(GVariant* document, const char* error_text, GArray* path,
                                     guint* validations) {
    GVariant* node = g_variant_ref(document);
    while (salvage_is_container(node)) {
        gsize count = g_variant_n_children(node);
        gboolean descended = FALSE;
        for (gsize i = 0; i < count; i++) {
            if (*validations >= SALVAGE_MAX_VALIDATIONS) {
                g_variant_unref(node);
                return FALSE;
            }
            guint index = (guint)i;
            g_array_append_val(path, index);
            g_autoptr(GVariant) candidate =
                salvage_without(document, (guint*)path->data, 0, path->len);
            (*validations)++;
            g_autofree char* text = candidate ? salvage_error_text(candidate) : NULL;
            if (candidate && (!text || strcmp(text, error_text) != 0)) {
                GVariant* next = salvage_child(node, index);
                g_variant_unref(node);
                node = next;
                descended = TRUE;
                break;
            }
            g_array_set_size(path, path->len - 1);
        }
        if (!descended)
            break;
    }
    g_variant_unref(node);
    return path->len > 0;
}

static char* salvage_path_name(GVariant* document, const GArray* path) {
    GString* name = g_string_new(NULL);
    GVariant* node = g_variant_ref(document);
    for (guint i = 0; i < path->len; i++) {
        guint index = g_array_index(path, guint, i);
        if (g_variant_is_of_type(node, G_VARIANT_TYPE_VARDICT)) {
            g_autoptr(GVariant) item = g_variant_get_child_value(node, index);
            g_autoptr(GVariant) key = g_variant_get_child_value(item, 0);
            g_string_append_printf(name, "%s%s", name->len ? "." : "",
                                   g_variant_get_string(key, NULL));
        } else {
            g_string_append_printf(name, "[%u]", index + 1);
        }
        GVariant* next = salvage_child(node, index);
        g_variant_unref(node);
        node = next;
    }
    g_variant_unref(node);
    return g_string_free(name, FALSE);
}

GVariant* gnoblin_config_salvage_document(GVariant* document, GPtrArray** ignored, GError** error) {
    if (ignored)
        *ignored = NULL;
    if (!document || !g_variant_is_of_type(document, G_VARIANT_TYPE_VARDICT)) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL, "configuration is not a map");
        return NULL;
    }
    g_autoptr(GVariant) current = g_variant_ref(document);
    g_autoptr(GPtrArray) dropped = g_ptr_array_new_with_free_func(g_free);
    guint validations = 0;
    for (guint round = 0; round <= SALVAGE_MAX_DROPS; round++) {
        g_autofree char* text = salvage_error_text(current);
        if (!text) {
            if (ignored)
                *ignored = g_steal_pointer(&dropped);
            return g_steal_pointer(&current);
        }
        g_autoptr(GArray) path = g_array_new(FALSE, FALSE, sizeof(guint));
        if (round == SALVAGE_MAX_DROPS ||
            !salvage_find_culprit(current, text, path, &validations)) {
            g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL, "%s", text);
            return NULL;
        }
        g_autofree char* name = salvage_path_name(current, path);
        g_autoptr(GVariant) next = salvage_without(current, (guint*)path->data, 0, path->len);
        if (!next) {
            g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL, "%s", text);
            return NULL;
        }
        g_ptr_array_add(dropped, g_strdup_printf("ignored %s: %s", name, text));
        g_clear_pointer(&current, g_variant_unref);
        current = g_steal_pointer(&next);
    }
    g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL, "too many invalid settings");
    return NULL;
}

char* gnoblin_config_path(void) {
    const char* override = g_getenv("GNOBLIN_CONFIG");
    if (override && override[0])
        return g_canonicalize_filename(override, NULL);
    g_autofree char* directory = g_build_filename(g_get_user_config_dir(), "gnoblin", NULL);
    /* Detect old configs instead of silently shadowing them with a new default. */
    const char* names[] = {"init.lua", "gnoblin.toml", "gnoblin.conf"};
    for (guint i = 0; i < G_N_ELEMENTS(names); i++) {
        g_autofree char* candidate = g_build_filename(directory, names[i], NULL);
        if (g_file_test(candidate, G_FILE_TEST_EXISTS))
            return g_steal_pointer(&candidate);
    }
    return g_build_filename(directory, "init.lua", NULL);
}
