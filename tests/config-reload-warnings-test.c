#include "gnoblin-runtime-internal.h"

/* Exercise the production reload request, commit, response and event builders.
 * Use a private socket pair and temporary config, state and notice directories.
 * No compositor or session is launched. */
static char* config_path;
static GVariant* response;
static int peer_fd;

static void runtime_init(Runtime* runtime) {
    int sockets[2];
    g_assert_cmpint(socketpair(AF_UNIX, SOCK_SEQPACKET | SOCK_CLOEXEC, 0, sockets), ==, 0);
    runtime->channel_fd = sockets[0];
    peer_fd = sockets[1];
    runtime->writer = gnoblin_runtime_writer_new();
}

static void runtime_clear(Runtime* runtime) {
    g_assert_cmpuint(runtime->write_watch_id, ==, 0);
    gnoblin_runtime_writer_free(runtime->writer);
    close(runtime->channel_fd);
    close(peer_fd);
}

static void read_response(Runtime* runtime) {
    g_autoptr(GnoblinRuntimeReader) reader = gnoblin_runtime_reader_new();
    g_autoptr(GError) error = NULL;
    for (guint i = 0; i < 1000 && !response; i++) {
        gboolean flushed =
            gnoblin_runtime_writer_flush(runtime->writer, runtime->channel_fd, &error);
        if (!flushed) {
            g_assert_error(error, G_IO_ERROR, G_IO_ERROR_WOULD_BLOCK);
            g_clear_error(&error);
        } else if (runtime->write_watch_id) {
            g_source_remove(runtime->write_watch_id);
            runtime->write_watch_id = 0;
        }
        GnoblinRuntimePacket packet = {0};
        gboolean available = FALSE;
        g_assert_true(gnoblin_runtime_reader_receive(reader, peer_fd, &packet, &available, &error));
        g_assert_no_error(error);
        if (available && packet.type == GNOBLIN_RUNTIME_PACKET_API_RESPONSE) {
            g_assert_cmpuint(packet.request_id, ==, 42);
            g_clear_pointer(&response, g_variant_unref);
            response = g_variant_ref(packet.payload);
        }
        gnoblin_runtime_packet_clear(&packet);
    }
}

gboolean restart_active_portal_service(GError** error) {
    g_assert_not_reached();
    return FALSE;
}

gboolean runtime_load_startup_config(gpointer data) {
    g_assert_not_reached();
    return G_SOURCE_REMOVE;
}

void runtime_schedule_deferred_packets(Runtime* runtime) {}

static void request_reload(Runtime* runtime, const char* source, gboolean commit) {
    g_autoptr(GError) error = NULL;
    g_autofree char* with_event_probe = g_strconcat(
        source,
        "\nlocal warnings_seen\n"
        "gnoblin.events.on('gnoblin.config.reloaded', function(e) warnings_seen = e.warnings end)\n"
        "gnoblin.events.on('test.check-warnings', function(e)\n"
        " assert(warnings_seen ~= nil, 'reload event has no warnings')\n"
        " assert(#warnings_seen == #e.expected)\n"
        " for i, warning in ipairs(e.expected) do assert(warnings_seen[i] == warning) end\n"
        "end)\n",
        NULL);
    g_assert_true(g_file_set_contents(config_path, with_event_probe, -1, &error));
    g_assert_no_error(error);
    g_clear_pointer(&response, g_variant_unref);
    g_autoptr(GVariant) request = g_variant_ref_sink(g_variant_new_parsed(
        "{'kind': <'reload'>, 'method': <'runtime.reload_config'>, 'arguments': <@a{sv} {}>}"));
    g_assert_true(handle_api_request(runtime, 42, request, &error));
    g_assert_no_error(error);
    if (runtime->pending_reload) {
        /* Native acceptance is simulated; production commit/rollback code is real. */
        g_assert_null(response);
        g_source_remove(runtime->reload_progress_id);
        runtime->reload_progress_id = 0;
        runtime->pending_reload->revision = gnoblin_config_settings_revision() + 1;
        runtime_reload_finish(runtime, commit, commit ? NULL : "native test rejection");
    }
    read_response(runtime);
    g_assert_nonnull(response);
    g_assert_false(runtime->failed);
}

static GVariant* successful_warnings(void) {
    gboolean ok = FALSE;
    g_assert_true(g_variant_lookup(response, "ok", "b", &ok));
    g_assert_true(ok);
    g_autoptr(GVariant) result = g_variant_lookup_value(response, "result", G_VARIANT_TYPE_VARDICT);
    g_assert_nonnull(result);
    g_assert_true(g_variant_lookup(result, "ok", "b", &ok));
    g_assert_true(ok);
    GVariant* warnings = g_variant_lookup_value(result, "warnings", G_VARIANT_TYPE_STRING_ARRAY);
    g_assert_nonnull(warnings);
    /* Compare the actual Lua event payload with the wire response. */
    g_autoptr(GError) error = NULL;
    GVariantBuilder probe;
    g_variant_builder_init(&probe, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&probe, "{sv}", "expected", warnings);
    g_autoptr(GVariant) payload = g_variant_ref_sink(g_variant_builder_end(&probe));
    g_autoptr(GVariant) changed =
        gnoblin_config_dispatch_event("test.check-warnings", payload, &error);
    g_assert_no_error(error);
    g_assert_nonnull(changed);
    gnoblin_config_finish_event(TRUE);
    g_autofree char* callback_error = gnoblin_config_take_runtime_callback_error();
    g_assert_null(callback_error);
    return warnings;
}

static void test_public_warnings(void) {
    Runtime runtime = {.host_control_fd = -1, .ready_fd = -1};
    runtime_init(&runtime);
    const char* source =
        "gnoblin.configure {not_a_section = {}, touchpad = {tap_button_map = 'lrm'},"
        " protocols = {not_a_protocol = false, wlr_screencopy = false}, cursor = {size = 31}}";
    for (guint repeat = 0; repeat < 2; repeat++) {
        request_reload(&runtime, source, TRUE);
        g_autoptr(GVariant) warnings = successful_warnings();
        g_assert_cmpuint(g_variant_n_children(warnings), ==, 3);
        g_autofree char* notice = config_recovery_notice();
        g_assert_nonnull(notice);
        const char* expected[] = {"unknown protocol \"not-a-protocol\"",
                                  "unknown section \"not-a-section\"",
                                  "unknown section \"touchpad\""};
        for (guint i = 0; i < G_N_ELEMENTS(expected); i++) {
            g_autoptr(GVariant) warning = g_variant_get_child_value(warnings, i);
            g_assert_nonnull(strstr(g_variant_get_string(warning, NULL), expected[i]));
            g_assert_nonnull(strstr(notice, expected[i]));
        }
        g_autoptr(GVariant) current = gnoblin_config_current_document();
        g_autoptr(GVariant) cursor =
            g_variant_lookup_value(current, "cursor", G_VARIANT_TYPE_VARDICT);
        gint64 size = 0;
        g_assert_true(g_variant_lookup(cursor, "size", "x", &size));
        g_assert_cmpint(size, ==, 31);
    }

    /* Schema errors and native rejection still retain the complete old config. */
    g_autoptr(GVariant) previous = gnoblin_config_current_document();
    guint64 revision = gnoblin_config_settings_revision();
    guint64 generation = gnoblin_config_runtime_generation();
    const char* rejected_sources[] = {
        "gnoblin.configure {cursor = {size = 0}, not_a_section = {}}",
        "gnoblin.configure {cursor = {size = 44}, not_a_section = {}}",
    };
    for (guint i = 0; i < G_N_ELEMENTS(rejected_sources); i++) {
        request_reload(&runtime, rejected_sources[i], FALSE);
        gboolean ok = TRUE;
        g_assert_true(g_variant_lookup(response, "ok", "b", &ok));
        g_assert_false(ok);
        g_autoptr(GVariant) current = gnoblin_config_current_document();
        g_assert_true(g_variant_equal(previous, current));
        g_assert_cmpuint(gnoblin_config_settings_revision(), ==, revision);
        g_assert_cmpuint(gnoblin_config_runtime_generation(), ==, generation);
    }
    request_reload(&runtime, "gnoblin.configure {cursor = {size = 32}}", TRUE);
    g_autoptr(GVariant) clean = successful_warnings();
    g_assert_cmpuint(g_variant_n_children(clean), ==, 0);
    g_autofree char* notice = config_recovery_notice();
    g_assert_null(notice);
    runtime_clear(&runtime);
}

static void test_warnings_preserve_crash_notice(void) {
    Runtime runtime = {.host_control_fd = -1, .ready_fd = -1};
    runtime_init(&runtime);
    set_config_fallback_marker(FALSE, FALSE, NULL);
    record_compositor_crash(SIGSEGV, 0);
    report_previous_crash();
    g_autofree char* marker =
        g_build_filename(g_get_user_runtime_dir(), "gnoblin", "config-fallback", NULL);
    g_autofree char* original = NULL;
    g_assert_true(g_file_get_contents(marker, &original, NULL, NULL));
    g_assert_true(g_str_has_prefix(original, "crash\n"));

    for (guint repeat = 0; repeat < 3; repeat++) {
        gboolean clean = repeat == 2;
        request_reload(&runtime,
                       clean ? "gnoblin.configure {cursor = {size = 32}}"
                             : "gnoblin.configure {not_a_section = {}}",
                       TRUE);
        g_autoptr(GVariant) warnings = successful_warnings();
        g_assert_cmpuint(g_variant_n_children(warnings), ==, clean ? 0 : 1);
        g_autofree char* current = NULL;
        g_assert_true(g_file_get_contents(marker, &current, NULL, NULL));
        g_assert_cmpstr(current, ==, original);
    }
    /* The existing next-session cleanup still owns the crash notice lifetime. */
    discard_stale_crash_notice();
    g_assert_false(g_file_test(marker, G_FILE_TEST_EXISTS));
    runtime_clear(&runtime);
}

static void test_warning_bounds(void) {
    Runtime runtime = {.host_control_fd = -1, .ready_fd = -1};
    runtime_init(&runtime);
    g_autoptr(GString) source = g_string_new("gnoblin.configure {cursor = {size = 32}}");
    /* Direct compatibility writes are accepted, and now get public feedback too. */
    for (guint i = 0; i < 100; i++)
        g_string_append_printf(source, "\ngnoblin.config[string.rep('x', 4096) .. '-%u'] = {}", i);
    request_reload(&runtime, source->str, TRUE);
    g_autoptr(GVariant) warnings = successful_warnings();
    g_assert_cmpuint(g_variant_n_children(warnings), ==, 65);
    g_autoptr(GVariant) last = g_variant_get_child_value(warnings, 64);
    g_assert_nonnull(strstr(g_variant_get_string(last, NULL), "omitted"));
    g_autofree char* notice = config_recovery_notice();
    g_assert_nonnull(notice);
    g_assert_cmpuint(strlen(notice), <, 16384);

    request_reload(&runtime, "gnoblin.config[string.rep('x', 4096)] = {}", TRUE);
    g_autoptr(GVariant) long_name = successful_warnings();
    g_assert_cmpuint(g_variant_n_children(long_name), ==, 1);
    g_autoptr(GVariant) warning = g_variant_get_child_value(long_name, 0);
    g_assert_cmpuint(strlen(g_variant_get_string(warning, NULL)), <=, 1024);
    runtime_clear(&runtime);
}

int main(int argc, char** argv) {
    g_test_init(&argc, &argv, NULL);
    g_log_set_always_fatal(G_LOG_LEVEL_ERROR | G_LOG_LEVEL_CRITICAL);
    g_autoptr(GError) error = NULL;
    g_autofree char* directory = g_dir_make_tmp("gnoblin-reload-warnings-XXXXXX", &error);
    g_assert_no_error(error);
    /* Real cache and portal helpers may only access this disposable directory. */
    g_setenv("XDG_STATE_HOME", directory, TRUE);
    g_setenv("XDG_CONFIG_HOME", directory, TRUE);
    g_setenv("XDG_RUNTIME_DIR", directory, TRUE);
    config_path = g_build_filename(directory, "init.lua", NULL);
    g_setenv("GNOBLIN_CONFIG", config_path, TRUE);
    g_test_add_func("/reload/public-warnings", test_public_warnings);
    g_test_add_func("/reload/warning-bounds", test_warning_bounds);
    g_test_add_func("/reload/warnings-preserve-crash-notice", test_warnings_preserve_crash_notice);
    int status = g_test_run();
    g_autofree char* cache = g_build_filename(directory, "gnoblin", "last-good-config.gv", NULL);
    g_autofree char* state_dir = g_path_get_dirname(cache);
    g_unlink(cache);
    g_autofree char* marker = g_build_filename(state_dir, "config-fallback", NULL);
    g_unlink(marker);
    g_rmdir(state_dir);
    g_unlink(config_path);
    g_rmdir(directory);
    g_free(config_path);
    g_clear_pointer(&response, g_variant_unref);
    return status;
}
