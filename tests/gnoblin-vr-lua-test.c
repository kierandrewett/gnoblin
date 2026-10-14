#include "gnoblin-config.h"

#include <glib/gstdio.h>

static void assert_lua_fails(const char* config, const char* source) {
    g_autoptr(GError) error = NULL;
    g_assert_true(g_file_set_contents(config, source, -1, &error));
    g_assert_no_error(error);
    g_autoptr(GVariant) invalid = gnoblin_config_load_document(config, NULL, NULL, &error);
    g_assert_null(invalid);
    g_assert_nonnull(error);
    g_clear_error(&error);
}

int main(void) {
    g_autoptr(GError) error = NULL;
    g_autofree char* runtime = g_dir_make_tmp("gnoblin-vr-no-headset-XXXXXX", &error);
    g_assert_no_error(error);
    g_assert_nonnull(runtime);
    g_setenv("XDG_RUNTIME_DIR", runtime, TRUE);
    g_setenv("XR_RUNTIME_JSON", "/nonexistent/gnoblin-vr-test-runtime.json", TRUE);

    g_autofree char* config = g_build_filename(runtime, "init.lua", NULL);
    const char* source = "assert(type(gnoblin.spatial) == 'table')\n"
                         "assert(type(gnoblin.spatial.snapshot) == 'function')\n"
                         "assert(type(gnoblin.spatial.anchor) == 'function')\n"
                         "assert(type(gnoblin.spatial.place) == 'function')\n"
                         "assert(type(gnoblin.spatial.attach) == 'function')\n"
                         "assert(type(gnoblin.spatial.surface) == 'function')\n"
                         "assert(type(gnoblin.spatial.move) == 'function')\n"
                         "assert(type(gnoblin.spatial.raise) == 'function')\n"
                         "assert(type(gnoblin.spatial.update_pose) == 'function')\n"
                         "assert(gnoblin.outputs.by_id('xr:headset') == nil)\n";
    g_assert_true(g_file_set_contents(config, source, -1, &error));
    g_assert_no_error(error);
    g_autoptr(GVariant) result = gnoblin_config_load_document(config, NULL, NULL, &error);
    g_assert_no_error(error);
    g_assert_nonnull(result);

    const char* invalid_calls[] = {
        "gnoblin.spatial.anchor({id='a'})",
        "gnoblin.spatial.place({surface_id='s', anchor_id='a', pose={}, size={width=1,height=1}})",
        "gnoblin.spatial.attach({surface_id='s', viewport_id='v', pose={}, "
        "size={width=1,height=1}})",
        "gnoblin.spatial.move({surface_id='s', hit={}, pose={}})",
        "gnoblin.spatial.raise({surface_id='s', unexpected=true})",
        "gnoblin.spatial.update_pose({surface_id='s', pose={}})",
        "gnoblin.spatial.snapshot('unexpected')",
    };
    for (guint i = 0; i < G_N_ELEMENTS(invalid_calls); i++) {
        g_autofree char* source = g_strdup_printf("%s\n", invalid_calls[i]);
        assert_lua_fails(config, source);
    }

    const char* valid_runtime_call =
        "local ok, err = pcall(function()\n"
        "  gnoblin.spatial.place({surface_id='s', anchor_id='a', "
        "pose={position={x=0,y=0,z=-1}, rotation={x=0,y=0,z=0,w=1}}, "
        "size={width=1,height=0.6}})\n"
        "end)\n"
        "assert(not ok and string.find(err, 'available only at runtime'))\n";
    g_assert_true(g_file_set_contents(config, valid_runtime_call, -1, &error));
    g_assert_no_error(error);
    g_autoptr(GVariant) validated = gnoblin_config_load_document(config, NULL, NULL, &error);
    g_assert_no_error(error);
    g_assert_nonnull(validated);

    return 0;
}
