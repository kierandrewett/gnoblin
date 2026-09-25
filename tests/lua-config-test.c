#include "gnoblin-config.h"
#include <glib/gstdio.h>

static GVariant* load(const char* path, GPtrArray** paths, GError** error) {
    return gnoblin_config_load_document(path, paths, NULL, error);
}

static void test_runtime_document_ownership(const char* path) {
    g_autoptr(GError) error = NULL;
    GVariant* document = gnoblin_config_load_runtime(path, NULL, NULL, &error);
    g_assert_no_error(error);
    g_assert_nonnull(document);
    g_assert_false(g_variant_is_floating(document));

    /* Match the Mutter wrapper: place the returned config in a result variant,
     * then replace the runtime before releasing that result. */
    GVariantBuilder builder;
    g_variant_builder_init(&builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&builder, "{sv}", "document", document);
    g_autoptr(GVariant) result = g_variant_ref_sink(g_variant_builder_end(&builder));
    gnoblin_config_finish_load(TRUE);
    g_variant_unref(document);

    document = gnoblin_config_load_runtime(path, NULL, NULL, &error);
    g_assert_no_error(error);
    g_assert_nonnull(document);
    gnoblin_config_finish_load(TRUE);
    g_variant_unref(document);
    g_variant_unref(g_steal_pointer(&result));
}

int main(void) {
    g_log_set_always_fatal(G_LOG_LEVEL_ERROR | G_LOG_LEVEL_CRITICAL);
    g_autoptr(GError) error = NULL;
    g_autofree char* dir = g_dir_make_tmp("gnoblin-lua-test-XXXXXX", &error);
    g_autofree char* conf = g_build_filename(dir, "conf.d", NULL);
    g_autofree char* root = g_build_filename(dir, "init.lua", NULL);
    g_autofree char* example_root = g_build_filename(dir, "example.lua", NULL);
    g_autofree char* explicit_root = g_build_filename(dir, "personal.lua", NULL);
    g_autofree char* runtime_root = g_build_filename(dir, "runtime.lua", NULL);
    g_autofree char* module = g_build_filename(dir, "module.lua", NULL);
    g_autofree char* nested = g_build_filename(dir, "nested.lua", NULL);
    g_assert_no_error(error);
    g_assert_cmpint(g_mkdir(conf, 0700), ==, 0);
    g_assert_true(g_file_set_contents(module, "return { name = 'module' }\n", -1, &error));
    g_assert_true(g_file_set_contents(
        nested,
        "local g=require('gnoblin'); g.set {shell={notifications=false,from_nested=true}}\n", -1,
        &error));
    g_autofree char* fragment = g_build_filename(conf, "10-bingux.lua", NULL);
    g_assert_true(g_file_set_contents(
        fragment, "return { shortcuts = {{ name = 'search', command = {'binguxctl'} }} }\n", -1,
        &error));
    g_assert_true(g_file_set_contents(
        root,
        "local g=require('gnoblin'); local a=require('module'); local b=require('module')\n"
        "if a~=b then error('require cache') end\n"
        "g.set { shell={osd=false}, shortcuts={} }; g.config.autostart={}\n"
        "g.animation { name='test-open', event='open', duration=240, from={scale_x=0.8}, "
        "to={scale_x=1} }\n"
        "g.animation { name='test-open', duration=260 }\n"
        "g.animation { name='removed', event='close', from={opacity=1}, to={opacity=0} }\n"
        "g.animation { name='removed', enable=false }\n"
        "g.config.keybindings={shell={show_screenshot_ui={}}}\n"
        "g.load('nested.lua'); g.load('conf.d/**/*.lua')\n",
        -1, &error));
    g_assert_no_error(error);
    g_assert_true(g_file_set_contents(runtime_root, "return { shell={osd=false} }\n", -1, &error));
    test_runtime_document_ownership(runtime_root);

    g_autoptr(GPtrArray) paths = NULL;
    g_autoptr(GVariant) document = load(root, &paths, &error);
    g_assert_no_error(error);
    g_assert_nonnull(document);
    g_assert_false(g_variant_is_floating(document));
    g_autoptr(GVariant) shell = g_variant_lookup_value(document, "shell", G_VARIANT_TYPE_VARDICT);
    g_autoptr(GVariant) osd = g_variant_lookup_value(shell, "osd", G_VARIANT_TYPE_BOOLEAN);
    g_autoptr(GVariant) notifications =
        g_variant_lookup_value(shell, "notifications", G_VARIANT_TYPE_BOOLEAN);
    g_autoptr(GVariant) nested_value =
        g_variant_lookup_value(shell, "from_nested", G_VARIANT_TYPE_BOOLEAN);
    g_assert_false(g_variant_get_boolean(osd));
    g_assert_false(g_variant_get_boolean(notifications));
    g_assert_true(g_variant_get_boolean(nested_value));
    g_autoptr(GVariant) shortcuts =
        g_variant_lookup_value(document, "shortcuts", G_VARIANT_TYPE("av"));
    g_assert_cmpuint(g_variant_n_children(shortcuts), ==, 1);
    g_autoptr(GVariant) animations =
        g_variant_lookup_value(document, "animations", G_VARIANT_TYPE("av"));
    g_assert_cmpuint(g_variant_n_children(animations), ==, 1);
    g_autoptr(GVariant) animation_box = g_variant_get_child_value(animations, 0);
    g_autoptr(GVariant) animation = g_variant_get_variant(animation_box);
    g_autoptr(GVariant) animation_name =
        g_variant_lookup_value(animation, "name", G_VARIANT_TYPE_STRING);
    g_autoptr(GVariant) animation_duration =
        g_variant_lookup_value(animation, "duration", G_VARIANT_TYPE_INT64);
    g_autoptr(GVariant) animation_from =
        g_variant_lookup_value(animation, "from", G_VARIANT_TYPE_VARDICT);
    g_autoptr(GVariant) animation_scale =
        g_variant_lookup_value(animation_from, "scale-x", G_VARIANT_TYPE_DOUBLE);
    g_assert_cmpstr(g_variant_get_string(animation_name, NULL), ==, "test-open");
    g_assert_cmpint(g_variant_get_int64(animation_duration), ==, 260);
    g_assert_cmpfloat(g_variant_get_double(animation_scale), ==, 0.8);
    g_autoptr(GVariant) keybindings =
        g_variant_lookup_value(document, "keybindings", G_VARIANT_TYPE_VARDICT);
    g_autoptr(GVariant) shell_bindings =
        g_variant_lookup_value(keybindings, "shell", G_VARIANT_TYPE_VARDICT);
    g_autoptr(GVariant) screenshot_actions =
        g_variant_lookup_value(shell_bindings, "show_screenshot_ui", G_VARIANT_TYPE("av"));
    g_assert_cmpuint(g_variant_n_children(screenshot_actions), ==, 0);
    g_assert_cmpuint(paths->len, >=, 4);

    const char* source_root = g_getenv("GNOBLIN_TEST_SOURCE_ROOT");
    if (source_root) {
        g_autofree char* example =
            g_build_filename(source_root, "src", "data", "init.lua.example", NULL);
        g_autofree char* example_source = NULL;
        g_assert_true(g_file_get_contents(example, &example_source, NULL, &error));
        g_assert_no_error(error);
        g_assert_true(g_file_set_contents(example_root, example_source, -1, &error));
        g_assert_no_error(error);
        g_clear_pointer(&document, g_variant_unref);
        document = load(example_root, NULL, &error);
        g_assert_no_error(error);
        g_assert_nonnull(document);
        g_autoptr(GVariant) example_shortcuts =
            g_variant_lookup_value(document, "shortcuts", G_VARIANT_TYPE("av"));
        g_assert_nonnull(example_shortcuts);
        /* The seed has ten commands, one built-in action and one shell shortcut. */
        g_assert_cmpuint(g_variant_n_children(example_shortcuts), ==, 12);
    }

    g_assert_true(g_file_set_contents(explicit_root, "return { shell={osd=true} }\n", -1, &error));
    g_clear_pointer(&document, g_variant_unref);
    document = load(explicit_root, NULL, &error);
    g_assert_no_error(error);
    g_assert_nonnull(document);
    g_autoptr(GVariant) explicit_shell =
        g_variant_lookup_value(document, "shell", G_VARIANT_TYPE_VARDICT);
    g_autoptr(GVariant) explicit_osd =
        g_variant_lookup_value(explicit_shell, "osd", G_VARIANT_TYPE_BOOLEAN);
    g_assert_true(g_variant_get_boolean(explicit_osd));

    g_assert_true(g_file_set_contents(nested, "return 1\n", -1, &error));
    g_clear_pointer(&document, g_variant_unref);
    document = load(root, NULL, &error);
    g_assert_null(document);
    g_assert_nonnull(error);
    g_clear_error(&error);

    const char* failures[] = {
        "error({})\n",
        "while true do end\n",
        "local x = string.rep('x', 16 * 1024 * 1024)\n",
        "local g=require('gnoblin'); g.config=42\n",
        "local g=require('gnoblin'); g.config=g.array{1}\n",
        "local g=require('gnoblin'); g.config=nil; setmetatable(g,{__index=function() error({}) "
        "end})\n",
        "return {1}\n",
        NULL,
    };
    for (guint i = 0; failures[i]; i++) {
        g_assert_true(g_file_set_contents(root, failures[i], -1, &error));
        document = load(root, NULL, &error);
        g_assert_null(document);
        g_assert_nonnull(error);
        g_clear_error(&error);
    }
    g_assert_true(g_file_set_contents(nested, "local g=require('gnoblin'); g.load('init.lua')\n",
                                      -1, &error));
    g_assert_true(g_file_set_contents(root, "local g=require('gnoblin'); g.load('nested.lua')\n",
                                      -1, &error));
    document = load(root, NULL, &error);
    g_assert_null(document);
    g_assert_nonnull(error);
    g_clear_error(&error);

    g_unlink(fragment);
    g_unlink(nested);
    g_unlink(module);
    g_unlink(root);
    g_unlink(explicit_root);
    g_unlink(runtime_root);
    g_rmdir(conf);
    g_rmdir(dir);
    g_print("PASS: Lua config, direct values, load, glob and errors\n");
    return 0;
}
