#include "../src/session/gnoblin-autostart.h"

#include <sys/socket.h>
#include <unistd.h>

static GVariant* environment_variant(const char* wayland, const char* display,
                                     const char* xauthority, const char* extra_key,
                                     const char* extra_value) {
    GVariantBuilder builder;
    g_variant_builder_init(&builder, G_VARIANT_TYPE("a{ss}"));
    if (wayland)
        g_variant_builder_add(&builder, "{ss}", "WAYLAND_DISPLAY", wayland);
    if (display)
        g_variant_builder_add(&builder, "{ss}", "DISPLAY", display);
    if (xauthority)
        g_variant_builder_add(&builder, "{ss}", "XAUTHORITY", xauthority);
    if (extra_key)
        g_variant_builder_add(&builder, "{ss}", extra_key, extra_value);
    return g_variant_ref_sink(g_variant_builder_end(&builder));
}

static GVariant* empty_entries(void) {
    return g_variant_ref_sink(g_variant_new_array(G_VARIANT_TYPE_VARIANT, NULL, 0));
}

static gboolean receive_payload(GVariant* payload, gboolean expected_success, GVariant** entries,
                                GVariant** environment, GError** error) {
    int sockets[2];
    g_assert_cmpint(socketpair(AF_UNIX, SOCK_SEQPACKET | SOCK_CLOEXEC, 0, sockets), ==, 0);
    g_autoptr(GnoblinRuntimeWriter) writer = gnoblin_runtime_writer_new();
    g_autoptr(GnoblinRuntimeReader) reader = gnoblin_runtime_reader_new();
    gboolean received = FALSE;
    gboolean sent = gnoblin_runtime_writer_queue(writer, GNOBLIN_RUNTIME_PACKET_HOST_AUTOSTART, 0,
                                                 payload, error) &&
                    gnoblin_runtime_writer_flush(writer, sockets[0], error);
    if (sent) {
        shutdown(sockets[0], SHUT_WR);
        sent = gnoblin_autostart_receive_packet(sockets[1], reader, &received, entries, environment,
                                                error);
    }
    close(sockets[0]);
    close(sockets[1]);
    g_assert_cmpint(sent, ==, expected_success);
    g_assert_cmpint(received, ==, expected_success);
    return sent;
}

static GVariant* valid_payload(GVariant* environment) {
    GVariantBuilder builder;
    g_variant_builder_init(&builder, G_VARIANT_TYPE_VARDICT);
    g_autoptr(GVariant) entries = empty_entries();
    g_variant_builder_add(&builder, "{sv}", "entries", entries);
    g_variant_builder_add(&builder, "{sv}", "environment", environment);
    return g_variant_ref_sink(g_variant_builder_end(&builder));
}

static void test_packet_and_child_environment(void) {
    g_autoptr(GVariant) source_environment =
        environment_variant("wayland-7", ":8", "/run/user/1000/Xauthority", NULL, NULL);
    g_autoptr(GVariant) payload = valid_payload(source_environment);
    g_autoptr(GVariant) entries = NULL;
    g_autoptr(GVariant) received_environment = NULL;
    g_autoptr(GError) error = NULL;
    g_assert_true(receive_payload(payload, TRUE, &entries, &received_environment, &error));
    g_assert_no_error(error);
    g_auto(GStrv) child_environment =
        gnoblin_autostart_build_environment(received_environment, &error);
    g_assert_no_error(error);
    g_assert_nonnull(child_environment);
    g_assert_cmpstr(g_environ_getenv(child_environment, "WAYLAND_DISPLAY"), ==, "wayland-7");
    g_assert_cmpstr(g_environ_getenv(child_environment, "DISPLAY"), ==, ":8");
    g_assert_cmpstr(g_environ_getenv(child_environment, "XAUTHORITY"), ==,
                    "/run/user/1000/Xauthority");
    g_assert_cmpuint(g_variant_n_children(entries), ==, 0);
}

static void test_packet_rejects_unknown_top_level_field(void) {
    g_autoptr(GVariant) env = environment_variant("wayland-7", NULL, NULL, NULL, NULL);
    g_autoptr(GVariant) entries = empty_entries();
    GVariantBuilder builder;
    g_variant_builder_init(&builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&builder, "{sv}", "entries", entries);
    g_variant_builder_add(&builder, "{sv}", "environment", env);
    g_variant_builder_add(&builder, "{sv}", "unexpected", g_variant_new_string("value"));
    g_autoptr(GVariant) payload = g_variant_ref_sink(g_variant_builder_end(&builder));
    g_autoptr(GVariant) received_entries = NULL;
    g_autoptr(GVariant) received_environment = NULL;
    g_autoptr(GError) error = NULL;
    g_assert_false(
        receive_payload(payload, FALSE, &received_entries, &received_environment, &error));
    g_assert_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA);
}

static void test_environment_requires_wayland_display(void) {
    g_autoptr(GVariant) env = environment_variant(NULL, ":8", NULL, NULL, NULL);
    g_autoptr(GError) error = NULL;
    g_auto(GStrv) child_environment = gnoblin_autostart_build_environment(env, &error);
    g_assert_null(child_environment);
    g_assert_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA);
}

static void test_environment_rejects_unlisted_variable(void) {
    g_autoptr(GVariant) env =
        environment_variant("wayland-7", NULL, NULL, "LD_PRELOAD", "/tmp/injected.so");
    g_autoptr(GError) error = NULL;
    g_auto(GStrv) child_environment = gnoblin_autostart_build_environment(env, &error);
    g_assert_null(child_environment);
    g_assert_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA);
}

static void test_environment_rejects_control_characters(void) {
    g_autoptr(GVariant) env = environment_variant("wayland-7\nDISPLAY=:8", NULL, NULL, NULL, NULL);
    g_autoptr(GError) error = NULL;
    g_auto(GStrv) child_environment = gnoblin_autostart_build_environment(env, &error);
    g_assert_null(child_environment);
    g_assert_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA);
}

static void test_environment_clears_stale_optional_values(void) {
    g_autoptr(GVariant) env = environment_variant("wayland-9", NULL, NULL, NULL, NULL);
    g_autoptr(GError) error = NULL;
    g_setenv("DISPLAY", ":stale", TRUE);
    g_setenv("XAUTHORITY", "/stale", TRUE);
    g_auto(GStrv) child_environment = gnoblin_autostart_build_environment(env, &error);
    g_assert_no_error(error);
    g_assert_nonnull(child_environment);
    g_assert_null(g_environ_getenv(child_environment, "DISPLAY"));
    g_assert_null(g_environ_getenv(child_environment, "XAUTHORITY"));
    g_unsetenv("DISPLAY");
    g_unsetenv("XAUTHORITY");
}

int main(int argc, char** argv) {
    g_test_init(&argc, &argv, NULL);
    g_test_add_func("/autostart-protocol/packet-and-child-environment",
                    test_packet_and_child_environment);
    g_test_add_func("/autostart-protocol/reject-unknown-packet-field",
                    test_packet_rejects_unknown_top_level_field);
    g_test_add_func("/autostart-protocol/require-wayland-display",
                    test_environment_requires_wayland_display);
    g_test_add_func("/autostart-protocol/reject-unlisted-environment",
                    test_environment_rejects_unlisted_variable);
    g_test_add_func("/autostart-protocol/reject-control-characters",
                    test_environment_rejects_control_characters);
    g_test_add_func("/autostart-protocol/clear-stale-optional-values",
                    test_environment_clears_stale_optional_values);
    return g_test_run();
}
