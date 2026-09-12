#include "gnoblin-config.h"
#include <glib/gstdio.h>

static GVariant* load(const char* path, GPtrArray** paths, GError** error) {
    return gnoblin_config_load_document(path, paths, NULL, error);
}

int main(void) {
    g_autoptr(GError) error = NULL;
    g_autofree char* dir = g_dir_make_tmp("gnoblin-lua-test-XXXXXX", &error);
    g_autofree char* conf = g_build_filename(dir, "conf.d", NULL);
    g_autofree char* root = g_build_filename(dir, "init.lua", NULL);
    g_autofree char* module = g_build_filename(dir, "module.lua", NULL);
    g_autofree char* toml = g_build_filename(dir, "fragment.toml", NULL);
    g_autofree char* nested = g_build_filename(dir, "nested.lua", NULL);
    g_assert_no_error(error);
    g_assert_cmpint(g_mkdir(conf, 0700), ==, 0);
    g_assert_true(g_file_set_contents(module, "return { name = 'module' }\n", -1, &error));
    g_assert_true(g_file_set_contents(
        nested, "local g=require('gnoblin'); g.config.shell.from_nested=true\n", -1, &error));
    g_assert_true(g_file_set_contents(
        toml, "source = 'nested.lua'\n[shell]\nnotifications = false\n", -1, &error));
    g_autofree char* fragment = g_build_filename(conf, "10-bingux.lua", NULL);
    g_assert_true(g_file_set_contents(
        fragment, "return { shortcuts = {{ name = 'search', command = {'binguxctl'} }} }\n", -1,
        &error));
    g_assert_true(g_file_set_contents(
        root,
        "local g=require('gnoblin'); local a=require('module'); local b=require('module')\n"
        "if a~=b then error('require cache') end\n"
        "g.set { shell={osd=false}, shortcuts={} }; g.config.autostart={}\n"
        "g.config.keybindings={shell={['show-screenshot-ui']={}}}\n"
        "g.load('fragment.toml'); g.load('conf.d/**/*.lua')\n",
        -1, &error));
    g_assert_no_error(error);
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
    g_autoptr(GVariant) keybindings =
        g_variant_lookup_value(document, "keybindings", G_VARIANT_TYPE_VARDICT);
    g_autoptr(GVariant) shell_bindings =
        g_variant_lookup_value(keybindings, "shell", G_VARIANT_TYPE_VARDICT);
    g_autoptr(GVariant) screenshot_actions =
        g_variant_lookup_value(shell_bindings, "show-screenshot-ui", G_VARIANT_TYPE("av"));
    g_assert_cmpuint(g_variant_n_children(screenshot_actions), ==, 0);
    g_assert_cmpuint(paths->len, >=, 5);

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
    document = load(root, NULL, &error);
    g_assert_null(document);
    g_assert_nonnull(error);
    g_clear_error(&error);

    g_unlink(fragment);
    g_unlink(nested);
    g_unlink(toml);
    g_unlink(module);
    g_unlink(root);
    g_rmdir(conf);
    g_rmdir(dir);
    g_print("PASS: Lua config, direct values, load, glob, TOML and errors\n");
    return 0;
}
