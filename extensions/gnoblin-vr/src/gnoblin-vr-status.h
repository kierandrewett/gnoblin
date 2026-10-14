#pragma once

#include <glib.h>
#include <glib/gstdio.h>
#include <errno.h>
#include <sys/stat.h>

static inline char* gnoblin_vr_status_path(void) {
    return g_build_filename(g_get_user_runtime_dir(), "gnoblin-vr", "status.ini", NULL);
}

static inline gboolean gnoblin_vr_status_write(const char* state, const char* diagnostic,
                                               guint view_count, const guint32* widths,
                                               const guint32* heights, GError** error) {
    g_autofree char* directory = g_build_filename(g_get_user_runtime_dir(), "gnoblin-vr", NULL);
    if (g_mkdir_with_parents(directory, 0700) != 0) {
        g_set_error(error, G_FILE_ERROR, g_file_error_from_errno(errno),
                    "cannot create VR status directory: %s", g_strerror(errno));
        return FALSE;
    }
    if (g_chmod(directory, 0700) != 0) {
        g_set_error(error, G_FILE_ERROR, g_file_error_from_errno(errno),
                    "cannot secure VR status directory: %s", g_strerror(errno));
        return FALSE;
    }

    GKeyFile* key_file = g_key_file_new();
    g_key_file_set_string(key_file, "vr", "state", state);
    if (diagnostic && *diagnostic)
        g_key_file_set_string(key_file, "vr", "diagnostic", diagnostic);
    g_key_file_set_integer(key_file, "vr", "view_count", (gint)MIN(view_count, 4));
    for (guint i = 0; i < MIN(view_count, 4); i++) {
        g_autofree char* width_key = g_strdup_printf("view_%u_width", i);
        g_autofree char* height_key = g_strdup_printf("view_%u_height", i);
        g_key_file_set_uint64(key_file, "vr", width_key, widths[i]);
        g_key_file_set_uint64(key_file, "vr", height_key, heights[i]);
    }
    gsize length = 0;
    g_autofree char* contents = g_key_file_to_data(key_file, &length, NULL);
    g_key_file_unref(key_file);
    g_autofree char* path = gnoblin_vr_status_path();
    if (!g_file_set_contents(path, contents, length, error))
        return FALSE;
    (void)g_chmod(path, 0600);
    return TRUE;
}

static inline GVariant* gnoblin_vr_status_record(void) {
    g_autofree char* path = gnoblin_vr_status_path();
    g_autoptr(GKeyFile) key_file = g_key_file_new();
    gboolean loaded = g_key_file_load_from_file(key_file, path, G_KEY_FILE_NONE, NULL);
    g_autofree char* state = loaded ? g_key_file_get_string(key_file, "vr", "state", NULL) : NULL;
    if (!state || (!g_str_equal(state, "absent") && !g_str_equal(state, "stopped") &&
                   !g_str_equal(state, "running") && !g_str_equal(state, "error"))) {
        g_clear_pointer(&state, g_free);
        state = g_strdup("stopped");
    }

    GVariantBuilder record;
    g_variant_builder_init(&record, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&record, "{sv}", "id", g_variant_new_string("xr:headset"));
    g_variant_builder_add(&record, "{sv}", "kind", g_variant_new_string("xr"));
    g_variant_builder_add(&record, "{sv}", "name", g_variant_new_string("OpenXR headset"));
    g_variant_builder_add(&record, "{sv}", "status", g_variant_new_string(state));
    g_variant_builder_add(&record, "{sv}", "available",
                          g_variant_new_boolean(g_str_equal(state, "running")));
    g_variant_builder_add(&record, "{sv}", "enabled",
                          g_variant_new_boolean(g_str_equal(state, "running")));
    if (loaded) {
        g_autofree char* diagnostic = g_key_file_get_string(key_file, "vr", "diagnostic", NULL);
        if (diagnostic && *diagnostic)
            g_variant_builder_add(&record, "{sv}", "diagnostic", g_variant_new_string(diagnostic));
        gint count = g_key_file_get_integer(key_file, "vr", "view_count", NULL);
        GVariantBuilder views;
        g_variant_builder_init(&views, G_VARIANT_TYPE("aa{sv}"));
        for (gint i = 0; i < CLAMP(count, 0, 4); i++) {
            g_autofree char* width_key = g_strdup_printf("view_%d_width", i);
            g_autofree char* height_key = g_strdup_printf("view_%d_height", i);
            guint64 width = g_key_file_get_uint64(key_file, "vr", width_key, NULL);
            guint64 height = g_key_file_get_uint64(key_file, "vr", height_key, NULL);
            GVariantBuilder view;
            g_variant_builder_init(&view, G_VARIANT_TYPE_VARDICT);
            g_variant_builder_add(&view, "{sv}", "eye",
                                  g_variant_new_string(i == 0   ? "left"
                                                       : i == 1 ? "right"
                                                                : "view"));
            g_variant_builder_add(&view, "{sv}", "width",
                                  g_variant_new_uint32((guint32)MIN(width, G_MAXUINT32)));
            g_variant_builder_add(&view, "{sv}", "height",
                                  g_variant_new_uint32((guint32)MIN(height, G_MAXUINT32)));
            g_variant_builder_add_value(&views, g_variant_builder_end(&view));
        }
        g_variant_builder_add(&record, "{sv}", "views", g_variant_builder_end(&views));
    } else {
        GVariantBuilder views;
        g_variant_builder_init(&views, G_VARIANT_TYPE("aa{sv}"));
        g_variant_builder_add(&record, "{sv}", "views", g_variant_builder_end(&views));
    }
    return g_variant_ref_sink(g_variant_builder_end(&record));
}
