/* System drop-ins: /usr/share/gnoblin/conf.d style directories loaded by the runtime.
 *
 * Order under test:
 *   no user config  -> built-in init.lua, then the drop-ins
 *   user config     -> the drop-ins, then the user's init.lua (the user always wins)
 * Drop-ins are searched in XDG_DATA_DIRS from the lowest priority entry to the highest. */
#include "gnoblin-config.h"
#include <glib/gstdio.h>
#include <string.h>

static void write_file(const char* directory, const char* name, const char* contents) {
    g_autoptr(GError) error = NULL;
    g_autofree char* path = g_build_filename(directory, name, NULL);
    g_autofree char* parent = g_path_get_dirname(path);
    g_assert_cmpint(g_mkdir_with_parents(parent, 0700), ==, 0);
    g_assert_true(g_file_set_contents(path, contents, -1, &error));
    g_assert_no_error(error);
}

static void remove_tree(const char* path) {
    g_autoptr(GDir) dir = g_dir_open(path, 0, NULL);
    if (dir) {
        const char* name;
        while ((name = g_dir_read_name(dir))) {
            g_autofree char* child = g_build_filename(path, name, NULL);
            remove_tree(child);
        }
    }
    g_remove(path);
}

typedef struct {
    char* root;
    char* high; /* first XDG_DATA_DIRS entry: wins */
    char* low;  /* last XDG_DATA_DIRS entry: loaded first */
    char* user_config;
} Fixture;

static char* conf_d(const char* data_dir) {
    return g_build_filename(data_dir, "gnoblin", "conf.d", NULL);
}

static void fixture_init(Fixture* fixture) {
    g_autoptr(GError) error = NULL;
    fixture->root = g_dir_make_tmp("gnoblin-dropins-XXXXXX", &error);
    g_assert_no_error(error);
    fixture->high = g_build_filename(fixture->root, "share-high", NULL);
    fixture->low = g_build_filename(fixture->root, "share-low", NULL);
    fixture->user_config = g_build_filename(fixture->root, "home", "init.lua", NULL);
    g_autofree char* data_dirs = g_strdup_printf("%s:%s", fixture->high, fixture->low);
    g_assert_true(g_setenv("XDG_DATA_DIRS", data_dirs, TRUE));
}

static void fixture_clear(Fixture* fixture) {
    remove_tree(fixture->root);
    g_free(fixture->root);
    g_free(fixture->high);
    g_free(fixture->low);
    g_free(fixture->user_config);
}

static void dropin(const char* data_dir, const char* name, const char* source) {
    g_autofree char* directory = conf_d(data_dir);
    write_file(directory, name, source);
}

static GVariant* load_document(Fixture* fixture, GPtrArray** paths, GPtrArray** directories,
                               GPtrArray** ignored) {
    g_autoptr(GError) error = NULL;
    GVariant* document = gnoblin_config_load_runtime_reporting(fixture->user_config, paths,
                                                               directories, ignored, &error);
    g_assert_no_error(error);
    g_assert_nonnull(document);
    gnoblin_config_finish_load(TRUE);
    return document;
}

static const char* window_string(GVariant* document, const char* key) {
    g_autoptr(GVariant) window =
        g_variant_lookup_value(document, "window-management", G_VARIANT_TYPE_VARDICT);
    g_assert_nonnull(window);
    const char* value = NULL;
    g_assert_true(g_variant_lookup(window, key, "&s", &value));
    return value;
}

static gint64 window_integer(GVariant* document, const char* key) {
    g_autoptr(GVariant) window =
        g_variant_lookup_value(document, "window-management", G_VARIANT_TYPE_VARDICT);
    g_assert_nonnull(window);
    g_autoptr(GVariant) value = g_variant_lookup_value(window, key, NULL);
    g_assert_nonnull(value);
    if (g_variant_is_of_type(value, G_VARIANT_TYPE_DOUBLE))
        return (gint64)g_variant_get_double(value);
    if (g_variant_is_of_type(value, G_VARIANT_TYPE_INT32))
        return g_variant_get_int32(value);
    g_assert_true(g_variant_is_of_type(value, G_VARIANT_TYPE_INT64));
    return g_variant_get_int64(value);
}

static gboolean window_boolean(GVariant* document, const char* key) {
    g_autoptr(GVariant) window =
        g_variant_lookup_value(document, "window-management", G_VARIANT_TYPE_VARDICT);
    g_assert_nonnull(window);
    gboolean value = FALSE;
    g_assert_true(g_variant_lookup(window, key, "b", &value));
    return value;
}

/* No user init.lua: the built-in init.lua runs first, the drop-ins change what it set. */
static void test_no_user_config(void) {
    Fixture fixture;
    fixture_init(&fixture);

    /* Neither drop-in directory exists: only the built-in defaults apply. */
    g_autoptr(GPtrArray) directories = NULL;
    g_autoptr(GPtrArray) ignored = NULL;
    g_autoptr(GVariant) defaults = load_document(&fixture, NULL, &directories, &ignored);
    g_assert_null(ignored);
    g_assert_cmpstr(window_string(defaults, "focus-mode"), ==, "click");
    g_assert_true(window_boolean(defaults, "raise-on-click"));
    g_assert_false(gnoblin_config_has_system_dropins());
    g_clear_pointer(&directories, g_ptr_array_unref);
    g_clear_pointer(&defaults, g_variant_unref);

    /* Low priority loads first; filename order inside a directory; the high entry wins. */
    dropin(fixture.low, "20-second.lua",
           "gnoblin.configure {window_management = {auto_raise_delay = 300,"
           " focus_mode = 'hover'}}\n");
    dropin(fixture.low, "10-first.lua",
           "gnoblin.configure {window_management = {auto_raise_delay = 200,"
           " raise_on_click = false}}\n"
           "gnoblin.load('parts/extra.lua')\n");
    dropin(fixture.low, "parts/extra.lua",
           "gnoblin.configure {window_management = {check_alive_timeout = 1234}}\n");
    dropin(fixture.high, "10-high.lua",
           "gnoblin.configure {window_management = {focus_mode = 'hover-strict'}}\n");
    dropin(fixture.high, ".hidden.lua",
           "gnoblin.configure {window_management = {focus_mode = 'click'}}\n");
    dropin(fixture.high, "notes.txt", "not lua at all (\n");
    g_assert_true(gnoblin_config_has_system_dropins());

    g_autoptr(GPtrArray) paths = NULL;
    g_autoptr(GVariant) document = load_document(&fixture, &paths, &directories, &ignored);
    g_assert_null(ignored);
    g_assert_cmpstr(window_string(document, "focus-mode"), ==, "hover-strict");
    g_assert_cmpint(window_integer(document, "auto-raise-delay"), ==, 300);
    g_assert_cmpint(window_integer(document, "check-alive-timeout"), ==, 1234);
    g_assert_false(window_boolean(document, "raise-on-click"));
    /* Built-in values the drop-ins left alone stay. */
    g_assert_true(window_boolean(document, "edge-tiling"));

    /* The files and directories are recorded as dependencies of this load. */
    g_autofree char* low_dir = conf_d(fixture.low);
    g_autofree char* high_dir = conf_d(fixture.high);
    g_autofree char* second = g_build_filename(low_dir, "20-second.lua", NULL);
    gboolean found_second = FALSE, found_high = FALSE, found_low = FALSE;
    for (guint i = 0; i < paths->len; i++)
        found_second |= g_str_equal(g_ptr_array_index(paths, i), second);
    for (guint i = 0; i < directories->len; i++) {
        found_high |= g_str_equal(g_ptr_array_index(directories, i), high_dir);
        found_low |= g_str_equal(g_ptr_array_index(directories, i), low_dir);
    }
    g_assert_true(found_second);
    g_assert_true(found_high);
    g_assert_true(found_low);
    fixture_clear(&fixture);
}

/* A user init.lua exists: it runs after the drop-ins and its values win. */
static void test_user_config_wins(void) {
    Fixture fixture;
    fixture_init(&fixture);
    dropin(fixture.low, "10-package.lua",
           "gnoblin.configure {window_management = {focus_mode = 'hover',"
           " auto_raise_delay = 700, raise_on_click = false}}\n");
    /* The user's own copy of a default layer, as `gnoblinctl init` writes it, is part of the user
     * config and overrides the drop-in. It is not loaded a second time by the runtime. */
    write_file(fixture.root, "home/config/10-copy.lua",
               "gnoblin.configure {window_management = {raise_on_click = true}}\n");
    write_file(fixture.root, "home/init.lua",
               "gnoblin.load('config/*.lua')\n"
               "gnoblin.configure {window_management = {auto_raise_delay = 900}}\n");

    g_autoptr(GPtrArray) ignored = NULL;
    g_autoptr(GVariant) document = load_document(&fixture, NULL, NULL, &ignored);
    g_assert_null(ignored);
    /* Set by the drop-in only. */
    g_assert_cmpstr(window_string(document, "focus-mode"), ==, "hover");
    /* Set by the drop-in, then by the user's init. */
    g_assert_cmpint(window_integer(document, "auto-raise-delay"), ==, 900);
    /* Set by the drop-in, then by the user's copy of the defaults. */
    g_assert_true(window_boolean(document, "raise-on-click"));
    g_clear_pointer(&document, g_variant_unref);

    /* An init.lua that does not load config/*.lua gets no hidden built-in base. */
    write_file(fixture.root, "home/init.lua", "gnoblin.configure {}\n");
    document = load_document(&fixture, NULL, NULL, &ignored);
    g_assert_cmpstr(window_string(document, "focus-mode"), ==, "hover");
    g_assert_cmpint(window_integer(document, "auto-raise-delay"), ==, 700);
    g_assert_false(window_boolean(document, "raise-on-click"));
    g_autoptr(GVariant) workspaces = g_variant_lookup_value(document, "workspaces", NULL);
    g_assert_null(workspaces);
    fixture_clear(&fixture);
}

/* A drop-in that fails is reported; the other drop-ins and the user config still load. */
static void test_broken_dropin(void) {
    Fixture fixture;
    fixture_init(&fixture);
    dropin(fixture.low, "10-syntax.lua", "gnoblin.configure {\n");
    dropin(fixture.low, "20-good.lua",
           "gnoblin.configure {window_management = {focus_mode = 'hover'}}\n");
    dropin(fixture.high, "10-runtime.lua", "error('broken on purpose')\n");
    dropin(fixture.high, "20-good.lua",
           "gnoblin.configure {window_management = {auto_raise_delay = 321}}\n");
    write_file(fixture.root, "home/init.lua",
               "gnoblin.configure {window_management = {check_alive_timeout = 4321}}\n");

    for (int with_user_config = 1; with_user_config >= 0; with_user_config--) {
        if (!with_user_config)
            g_assert_cmpint(g_unlink(fixture.user_config), ==, 0);
        g_autoptr(GPtrArray) ignored = NULL;
        g_autoptr(GVariant) document = load_document(&fixture, NULL, NULL, &ignored);
        g_assert_nonnull(ignored);
        g_assert_cmpuint(ignored->len, ==, 2);
        g_assert_nonnull(strstr(g_ptr_array_index(ignored, 0), "10-syntax.lua"));
        g_assert_nonnull(strstr(g_ptr_array_index(ignored, 1), "10-runtime.lua"));
        g_assert_nonnull(strstr(g_ptr_array_index(ignored, 1), "broken on purpose"));
        g_assert_cmpstr(window_string(document, "focus-mode"), ==, "hover");
        g_assert_cmpint(window_integer(document, "auto-raise-delay"), ==, 321);
        if (with_user_config)
            g_assert_cmpint(window_integer(document, "check-alive-timeout"), ==, 4321);
    }
    fixture_clear(&fixture);
}

/* The embedded init.lua that `gnoblinctl init` copies must not load the drop-ins itself. */
static void test_default_tree_does_not_load_dropins(void) {
    gsize count = 0;
    const GnoblinConfigDefaultFile* files = gnoblin_config_default_files(&count);
    g_assert_cmpuint(count, >, 0);
    for (gsize i = 0; i < count; i++) {
        g_assert_null(strstr(files[i].contents, "conf.d"));
        g_assert_null(strstr(files[i].contents, "/usr/share/gnoblin"));
    }
}

int main(void) {
    g_autoptr(GError) error = NULL;
    /* GLib reads XDG_STATE_HOME once; keep the last-good-config file out of the real home. */
    g_autofree char* state_home = g_dir_make_tmp("gnoblin-dropins-state-XXXXXX", &error);
    g_assert_no_error(error);
    g_assert_true(g_setenv("XDG_STATE_HOME", state_home, TRUE));
    test_no_user_config();
    test_user_config_wins();
    test_broken_dropin();
    test_default_tree_does_not_load_dropins();
    remove_tree(state_home);
    return 0;
}
