#include "gnoblin-config.h"
#include <glib/gstdio.h>

int main(void) {
    g_autoptr(GError) error = NULL;
    g_autofree char* dir = g_dir_make_tmp("gnoblin-glob-test-XXXXXX", &error);
    g_assert_no_error(error);
    g_autofree char* root = g_build_filename(dir, "init.lua", NULL);
    g_autofree char* nested = g_build_filename(dir, "conf.d", "nested", NULL);
    g_autofree char* first = g_build_filename(dir, "conf.d", "10-first.lua", NULL);
    g_autofree char* second = g_build_filename(nested, "20-second.lua", NULL);
    g_autofree char* hidden = g_build_filename(nested, ".hidden.lua", NULL);
    g_mkdir_with_parents(nested, 0700);
    g_file_set_contents(first, "return {}", -1, NULL);
    g_file_set_contents(second, "return {}", -1, NULL);
    g_file_set_contents(hidden, "return {}", -1, NULL);
    g_autoptr(GPtrArray) watched = g_ptr_array_new_with_free_func(g_free);
    g_autoptr(GPtrArray) matches =
        gnoblin_config_expand_paths(root, "conf.d/**/*.lua", watched, &error);
    g_assert_no_error(error);
    g_assert_cmpuint(matches->len, ==, 2);
    g_assert_cmpstr(g_ptr_array_index(matches, 0), ==, first);
    g_assert_cmpstr(g_ptr_array_index(matches, 1), ==, second);
    g_assert_cmpuint(watched->len, ==, 2);
    g_clear_pointer(&matches, g_ptr_array_unref);
    matches = gnoblin_config_expand_paths(root, "missing/**/*.lua", watched, &error);
    g_assert_no_error(error);
    g_assert_cmpuint(matches->len, ==, 0);
    g_assert_cmpuint(watched->len, ==, 3);
    g_clear_pointer(&matches, g_ptr_array_unref);
    matches = gnoblin_config_expand_paths(root, "conf.d/[12]*.lua", watched, &error);
    g_assert_no_error(error);
    g_assert_cmpuint(matches->len, ==, 1);
    g_clear_pointer(&matches, g_ptr_array_unref);
    matches = gnoblin_config_expand_paths(root, "absent.lua", watched, &error);
    g_assert_no_error(error);
    g_assert_cmpuint(matches->len, ==, 1);
    g_unlink(hidden);
    g_unlink(second);
    g_unlink(first);
    g_rmdir(nested);
    g_autofree char* conf = g_build_filename(dir, "conf.d", NULL);
    g_rmdir(conf);
    g_rmdir(dir);
    g_print("PASS: sorted recursive includes, patterns, hidden files and missing directories\n");
    return 0;
}
