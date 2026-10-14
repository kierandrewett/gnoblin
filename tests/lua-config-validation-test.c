#include "gnoblin-config.h"
#include <glib/gstdio.h>
#include <string.h>

typedef struct {
    const char* name;
    const char* source;
    const char* diagnostic;
} InvalidConfig;

static const InvalidConfig invalid_configs[] = {
    {"protocol-type", "gnoblin.configure {protocols = {wlr_screencopy = 'false'}}",
     "wlr-screencopy"},
    {"protocol-table", "gnoblin.configure {protocols = false}", "protocols"},
    {"protocol-array", "gnoblin.configure {protocols = {false}}", "protocols"},
    {"nested-setting", "gnoblin.configure {compositor = {not_a_setting = false}}", "not-a-setting"},
    {"compositor-debug-type", "gnoblin.configure {compositor = {debug = 'true'}}",
     "\"debug\" has an unsupported name or value"},
    {"touchpad-enum", "gnoblin.configure {input = {touchpad = {tap_button_map = 'xyz'}}}",
     "tap-button-map"},
    {"removed-shell", "gnoblin.configure {shell = {}}", "shell settings are no longer supported"},
};

static char* directory;
static char* path;

static void write_source(const char* source) {
    g_autoptr(GError) error = NULL;
    g_assert_true(g_file_set_contents(path, source, -1, &error));
    g_assert_no_error(error);
}

static GVariant* load_valid(const char* source) {
    write_source(source);
    g_autoptr(GError) error = NULL;
    GVariant* document = gnoblin_config_load_document(path, NULL, NULL, &error);
    g_assert_no_error(error);
    g_assert_nonnull(document);
    return document;
}

static void assert_invalid(gconstpointer data) {
    const InvalidConfig* test = data;
    write_source(test->source);
    g_autoptr(GError) error = NULL;
    g_autoptr(GVariant) document = gnoblin_config_load_document(path, NULL, NULL, &error);
    g_assert_null(document);
    g_assert_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL);
    g_assert_nonnull(strstr(error->message, test->diagnostic));
}

static void test_known_protocols(void) {
    const char* names[] = {
        "wlr_layer_shell",
        "ext_foreign_toplevel_list",
        "wlr_foreign_toplevel_management",
        "xdg_decoration",
        "kde_server_decoration",
        "window_frame_renderer",
        "wlr_screencopy",
        "ext_background_effect_v1",
        "blur_fade",
        "ext_data_control",
        "ext_idle_notify",
        "wlr_gamma_control",
        "wlr_output_power_management",
        "ext_session_lock",
    };
    for (guint i = 0; i < G_N_ELEMENTS(names); i++) {
        for (guint enabled = 0; enabled < 2; enabled++) {
            g_autofree char* source = g_strdup_printf("gnoblin.configure {protocols = {%s = %s}}",
                                                      names[i], enabled ? "true" : "false");
            g_autoptr(GVariant) document = load_valid(source);
            g_autoptr(GVariant) protocols =
                g_variant_lookup_value(document, "protocols", G_VARIANT_TYPE_VARDICT);
            g_autofree char* key = g_strdelimit(g_strdup(names[i]), "_", '-');
            gboolean actual;
            g_assert_true(g_variant_lookup(protocols, key, "b", &actual));
            g_assert_cmpint(actual, ==, enabled);
        }
    }
    g_autoptr(GVariant) empty = load_valid("gnoblin.configure {protocols = {}}");
}

static void test_aliases_and_extensions(void) {
    g_autoptr(GVariant) document =
        load_valid("gnoblin.configure {"
                   " window_management = {focus_mode = 'sloppy', focus_new_windows = 'strict'},"
                   " input = {touchpad = {tap_button_map = 'lrm', left_handed = 'mouse'}},"
                   " frame_renderers = {['my_renderer-v2'] = {'renderer', '--test'}},"
                   " portals = {default = {'gtk'}, interfaces = {"
                   "  ['org.freedesktop.impl.portal.Test_Interface'] = {'gtk'}}},"
                   " shortcuts = {['my_shortcut-v2'] = {binding = '<Super>t', command = {'true'}}},"
                   " autostart = {['my_service-v2'] = {command = {'true'}}},"
                   " workspaces = {{id = 'main', name = 'Main'}},"
                   " window_rules = {}, animations = {},"
                   " keybindings = {keyboard = {test_key = {binding = '<Super>y',"
                   "  callback = function() return 'forward' end}}}"
                   "}\n"
                   "gnoblin.config.cursor = {size = 32}\n"
                   "gnoblin.window_rule {match = {app_id = 'test'}, opacity = 0.9}\n");
    g_autoptr(GVariant) window =
        g_variant_lookup_value(document, "window-management", G_VARIANT_TYPE_VARDICT);
    const char* value;
    g_assert_true(g_variant_lookup(window, "focus-mode", "&s", &value));
    g_assert_cmpstr(value, ==, "hover");
    g_assert_true(g_variant_lookup(window, "focus-new-windows", "&s", &value));
    g_assert_cmpstr(value, ==, "prevent");
    g_autoptr(GVariant) input = g_variant_lookup_value(document, "input", G_VARIANT_TYPE_VARDICT);
    g_autoptr(GVariant) touchpad =
        g_variant_lookup_value(input, "touchpad", G_VARIANT_TYPE_VARDICT);
    g_assert_true(g_variant_lookup(touchpad, "tap-button-map", "&s", &value));
    g_assert_cmpstr(value, ==, "left-right-middle");
    g_assert_true(g_variant_lookup(touchpad, "left-handed", "&s", &value));
    g_assert_cmpstr(value, ==, "follow-mouse");
    g_autoptr(GVariant) handlers =
        g_variant_lookup_value(document, "input-handlers", G_VARIANT_TYPE("av"));
    g_assert_nonnull(handlers);
    g_assert_cmpuint(g_variant_n_children(handlers), ==, 1);
    g_autoptr(GVariant) renderers =
        g_variant_lookup_value(document, "frame-renderers", G_VARIANT_TYPE_VARDICT);
    g_autoptr(GVariant) renderer = g_variant_lookup_value(renderers, "my_renderer-v2", NULL);
    g_assert_nonnull(renderer);
}

static void assert_active_unchanged(GVariant* previous, guint64 revision, guint64 generation) {
    g_autoptr(GVariant) current = gnoblin_config_current_document();
    g_assert_nonnull(current);
    g_assert_true(g_variant_equal(previous, current));
    g_assert_cmpuint(gnoblin_config_settings_revision(), ==, revision);
    g_assert_cmpuint(gnoblin_config_runtime_generation(), ==, generation);
}

static void test_transaction_and_recovery(void) {
    write_source(
        "gnoblin.configure {cursor = {size = 31}, protocols = {wlr_screencopy = false}}\n"
        "gnoblin.events.on('test.valid', function() gnoblin.configure {cursor = {size = 32}} end)\n"
        "gnoblin.events.on('test.invalid', function()\n"
        " gnoblin.configure {cursor = {size = 0}} end)\n");
    g_autoptr(GError) error = NULL;
    g_autoptr(GVariant) previous = gnoblin_config_load_runtime(path, NULL, NULL, &error);
    g_assert_no_error(error);
    g_assert_nonnull(previous);
    gnoblin_config_finish_load(TRUE);
    guint64 revision = gnoblin_config_settings_revision();
    guint64 generation = gnoblin_config_runtime_generation();

    for (guint i = 0; i < G_N_ELEMENTS(invalid_configs); i++) {
        write_source(invalid_configs[i].source);
        g_autoptr(GVariant) rejected = gnoblin_config_load_runtime(path, NULL, NULL, &error);
        g_assert_null(rejected);
        g_assert_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL);
        g_assert_nonnull(strstr(error->message, invalid_configs[i].diagnostic));
        g_clear_error(&error);
        gnoblin_config_finish_load(FALSE);
        assert_active_unchanged(previous, revision, generation);
    }

    g_autoptr(GVariant) payload =
        g_variant_ref_sink(g_variant_new_array(G_VARIANT_TYPE("{sv}"), NULL, 0));
    g_autoptr(GVariant) rejected = gnoblin_config_dispatch_event("test.invalid", payload, &error);
    g_assert_null(rejected);
    g_assert_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL);
    g_assert_nonnull(strstr(error->message, "cursor"));
    g_clear_error(&error);
    assert_active_unchanged(previous, revision, generation);

    /* The original callbacks and Lua config table survive a failed reload/event. */
    g_autoptr(GVariant) changed = gnoblin_config_dispatch_event("test.valid", payload, &error);
    g_assert_no_error(error);
    g_assert_nonnull(changed);
    gnoblin_config_finish_event(TRUE);
    g_assert_cmpuint(gnoblin_config_settings_revision(), ==, revision + 1);
    g_autoptr(GVariant) cursor = g_variant_lookup_value(changed, "cursor", G_VARIANT_TYPE_VARDICT);
    gint64 size;
    g_assert_true(g_variant_lookup(cursor, "size", "x", &size));
    g_assert_cmpint(size, ==, 32);
}

int main(int argc, char** argv) {
    g_test_init(&argc, &argv, NULL);
    /* A baseline warning must not substitute for the rejection under test. */
    g_log_set_always_fatal(G_LOG_LEVEL_ERROR | G_LOG_LEVEL_CRITICAL);
    g_autoptr(GError) error = NULL;
    directory = g_dir_make_tmp("gnoblin-config-validation-XXXXXX", &error);
    g_assert_no_error(error);
    path = g_build_filename(directory, "init.lua", NULL);
    for (guint i = 0; i < G_N_ELEMENTS(invalid_configs); i++) {
        g_autofree char* name = g_strdup_printf("/config/reject/%s", invalid_configs[i].name);
        g_test_add_data_func(name, &invalid_configs[i], assert_invalid);
    }
    g_test_add_func("/config/known-protocols", test_known_protocols);
    g_test_add_func("/config/aliases-and-extensions", test_aliases_and_extensions);
    g_test_add_func("/config/transaction-and-recovery", test_transaction_and_recovery);
    int result = g_test_run();
    g_unlink(path);
    g_rmdir(directory);
    g_free(path);
    g_free(directory);
    return result;
}
