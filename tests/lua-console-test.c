#include "gnoblin-console-lua.h"
#include <glib.h>
#include <glib/gstdio.h>
static GVariant *run(const char *source) {
    GVariant *result = gnoblin_console_lua("eval", source);
    const char *error = NULL;
    g_variant_lookup(result, "error", "&s", &error);
    if (error) g_error("%s: %s", source, error);
    return result;
}
static void expect(const char *source, const char *expected) {
    g_autoptr(GVariant) result = run(source);
    g_auto(GStrv) values = NULL;
    g_variant_lookup(result, "values", "^as", &values);
    g_assert_cmpstr(values[0], ==, expected);
}
int main(void) {
    g_setenv("GNOBLIN_CONFIG", "/tmp/gnoblin-console-absent.lua", TRUE);
    expect("21 * 2", "42");
    g_variant_unref(run("counter = 9"));
    expect("counter + 1", "10");
    expect("require('gnoblin').set { shell = { test = 7 } }; return gnoblin.config.shell.test", "7");
    g_autoptr(GVariant) printed = run("print('hello', 42)");
    g_auto(GStrv) lines = NULL;
    g_variant_lookup(printed, "lines", "^as", &lines);
    g_assert_cmpstr(lines[0], ==, "hello\t42");
    g_autoptr(GVariant) failed = gnoblin_console_lua("eval", "error('probe')");
    g_assert_nonnull(g_variant_lookup_value(failed, "error", NULL));
    g_autoptr(GVariant) limited = gnoblin_console_lua("eval", "while true do end");
    g_assert_nonnull(g_variant_lookup_value(limited, "error", NULL));
    expect("counter", "9");
    g_autofree char *directory = g_dir_make_tmp("gnoblin-lua-console-XXXXXX", NULL);
    g_autofree char *script = g_build_filename(directory, "probe.lua", NULL);
    g_file_set_contents(script, "script_counter = 73", -1, NULL);
    g_autofree char *load = g_strdup_printf("gnoblin.load('%s'); return script_counter", script);
    expect(load, "73");
    g_unlink(script);
    g_rmdir(directory);
    g_autoptr(GVariant) completion = gnoblin_console_lua("complete", "math.sq");
    g_auto(GStrv) choices = NULL;
    g_variant_lookup(completion, "lines", "^as", &choices);
    g_assert_cmpstr(choices[0], ==, "sqrt");
    g_variant_unref(gnoblin_console_lua("reset", ""));
    expect("counter", "nil");
    g_variant_unref(gnoblin_console_lua("reset", ""));
    g_print("PASS: persistent Lua console, config API, print, completion, limits and reset\n");
}
