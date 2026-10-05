#include "../src/native-control/gnoblin-runtime-protocol.h"

#include <sys/socket.h>
#include <unistd.h>

static GVariant* sample_payload(void) {
    GVariantBuilder builder;
    g_variant_builder_init(&builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&builder, "{sv}", "event", g_variant_new_string("window.created"));
    g_variant_builder_add(&builder, "{sv}", "window_id", g_variant_new_uint64(42));
    return g_variant_ref_sink(g_variant_builder_end(&builder));
}

static void test_round_trip(void) {
    int sockets[2];
    g_assert_cmpint(socketpair(AF_UNIX, SOCK_SEQPACKET | SOCK_CLOEXEC, 0, sockets), ==, 0);

    g_autoptr(GVariant) sent = sample_payload();
    g_autoptr(GError) error = NULL;
    g_autoptr(GnoblinRuntimeWriter) writer = gnoblin_runtime_writer_new();
    g_autoptr(GnoblinRuntimeReader) reader = gnoblin_runtime_reader_new();
    g_assert_true(gnoblin_runtime_writer_queue(writer, GNOBLIN_RUNTIME_PACKET_CONFIG_RESULT,
                                               0x1020304050607080, sent, &error));
    g_assert_true(gnoblin_runtime_writer_flush(writer, sockets[0], &error));
    g_assert_no_error(error);

    GnoblinRuntimePacket received = {0};
    gboolean available = FALSE;
    g_assert_true(
        gnoblin_runtime_reader_receive(reader, sockets[1], &received, &available, &error));
    g_assert_true(available);
    g_assert_no_error(error);
    g_assert_cmpint(received.type, ==, GNOBLIN_RUNTIME_PACKET_CONFIG_RESULT);
    g_assert_cmpuint(received.request_id, ==, G_GUINT64_CONSTANT(0x1020304050607080));
    g_assert_true(g_variant_equal(sent, received.payload));
    gnoblin_runtime_packet_clear(&received);
    close(sockets[0]);
    close(sockets[1]);
}

static void test_host_autostart_packet_round_trip(void) {
    int sockets[2];
    g_assert_cmpint(socketpair(AF_UNIX, SOCK_SEQPACKET | SOCK_CLOEXEC, 0, sockets), ==, 0);

    GVariantBuilder entries;
    g_variant_builder_init(&entries, G_VARIANT_TYPE("av"));
    GVariantBuilder payload_builder;
    g_variant_builder_init(&payload_builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&payload_builder, "{sv}", "entries", g_variant_builder_end(&entries));
    g_autoptr(GVariant) payload = g_variant_ref_sink(g_variant_builder_end(&payload_builder));
    g_autoptr(GError) error = NULL;
    g_autoptr(GnoblinRuntimeWriter) writer = gnoblin_runtime_writer_new();
    g_autoptr(GnoblinRuntimeReader) reader = gnoblin_runtime_reader_new();
    g_assert_true(gnoblin_runtime_writer_queue(writer, GNOBLIN_RUNTIME_PACKET_HOST_AUTOSTART, 0,
                                               payload, &error));
    g_assert_true(gnoblin_runtime_writer_flush(writer, sockets[0], &error));
    g_assert_no_error(error);

    GnoblinRuntimePacket received = {0};
    gboolean available = FALSE;
    g_assert_true(
        gnoblin_runtime_reader_receive(reader, sockets[1], &received, &available, &error));
    g_assert_true(available);
    g_assert_no_error(error);
    g_assert_cmpint(received.type, ==, GNOBLIN_RUNTIME_PACKET_HOST_AUTOSTART);
    g_assert_cmpuint(received.request_id, ==, 0);
    g_assert_true(g_variant_equal(payload, received.payload));
    gnoblin_runtime_packet_clear(&received);
    close(sockets[0]);
    close(sockets[1]);
}

static void test_recovery_failed_packet_round_trip(void) {
    int sockets[2];
    g_assert_cmpint(socketpair(AF_UNIX, SOCK_SEQPACKET | SOCK_CLOEXEC, 0, sockets), ==, 0);

    g_autoptr(GVariant) payload =
        g_variant_ref_sink(g_variant_new_array(G_VARIANT_TYPE("{sv}"), NULL, 0));
    g_autoptr(GError) error = NULL;
    g_autoptr(GnoblinRuntimeWriter) writer = gnoblin_runtime_writer_new();
    g_autoptr(GnoblinRuntimeReader) reader = gnoblin_runtime_reader_new();
    g_assert_true(gnoblin_runtime_writer_queue(writer, GNOBLIN_RUNTIME_PACKET_RECOVERY_FAILED, 0,
                                               payload, &error));
    g_assert_true(gnoblin_runtime_writer_flush(writer, sockets[0], &error));
    g_assert_no_error(error);

    GnoblinRuntimePacket received = {0};
    gboolean available = FALSE;
    g_assert_true(
        gnoblin_runtime_reader_receive(reader, sockets[1], &received, &available, &error));
    g_assert_true(available);
    g_assert_no_error(error);
    g_assert_cmpint(received.type, ==, GNOBLIN_RUNTIME_PACKET_RECOVERY_FAILED);
    g_assert_cmpuint(received.request_id, ==, 0);
    g_assert_true(g_variant_equal(payload, received.payload));
    gnoblin_runtime_packet_clear(&received);
    close(sockets[0]);
    close(sockets[1]);
}

static void test_session_state_changed_packet_round_trip(void) {
    int sockets[2];
    g_assert_cmpint(socketpair(AF_UNIX, SOCK_SEQPACKET | SOCK_CLOEXEC, 0, sockets), ==, 0);

    GVariantBuilder builder;
    g_variant_builder_init(&builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&builder, "{sv}", "state", g_variant_new_string("starting"));
    g_variant_builder_add(&builder, "{sv}", "revision", g_variant_new_uint64(3));
    g_autoptr(GVariant) payload = g_variant_ref_sink(g_variant_builder_end(&builder));
    g_autoptr(GError) error = NULL;
    g_autoptr(GnoblinRuntimeWriter) writer = gnoblin_runtime_writer_new();
    g_autoptr(GnoblinRuntimeReader) reader = gnoblin_runtime_reader_new();
    g_assert_true(gnoblin_runtime_writer_queue(writer, GNOBLIN_RUNTIME_PACKET_SESSION_STATE_CHANGED,
                                               0, payload, &error));
    g_assert_true(gnoblin_runtime_writer_flush(writer, sockets[0], &error));
    g_assert_no_error(error);

    GnoblinRuntimePacket received = {0};
    gboolean available = FALSE;
    g_assert_true(
        gnoblin_runtime_reader_receive(reader, sockets[1], &received, &available, &error));
    g_assert_true(available);
    g_assert_no_error(error);
    g_assert_cmpint(received.type, ==, GNOBLIN_RUNTIME_PACKET_SESSION_STATE_CHANGED);
    g_assert_cmpuint(received.request_id, ==, 0);
    g_assert_true(g_variant_equal(payload, received.payload));
    gnoblin_runtime_packet_clear(&received);
    close(sockets[0]);
    close(sockets[1]);
}

static void test_session_state_published_packet_round_trip(void) {
    int sockets[2];
    g_assert_cmpint(socketpair(AF_UNIX, SOCK_SEQPACKET | SOCK_CLOEXEC, 0, sockets), ==, 0);

    g_autoptr(GVariant) payload =
        g_variant_ref_sink(g_variant_new_array(G_VARIANT_TYPE("{sv}"), NULL, 0));
    g_autoptr(GError) error = NULL;
    g_autoptr(GnoblinRuntimeWriter) writer = gnoblin_runtime_writer_new();
    g_autoptr(GnoblinRuntimeReader) reader = gnoblin_runtime_reader_new();
    g_assert_true(gnoblin_runtime_writer_queue(
        writer, GNOBLIN_RUNTIME_PACKET_SESSION_STATE_PUBLISHED, 3, payload, &error));
    g_assert_true(gnoblin_runtime_writer_flush(writer, sockets[0], &error));
    g_assert_no_error(error);

    GnoblinRuntimePacket received = {0};
    gboolean available = FALSE;
    g_assert_true(
        gnoblin_runtime_reader_receive(reader, sockets[1], &received, &available, &error));
    g_assert_true(available);
    g_assert_no_error(error);
    g_assert_cmpint(received.type, ==, GNOBLIN_RUNTIME_PACKET_SESSION_STATE_PUBLISHED);
    g_assert_cmpuint(received.request_id, ==, 3);
    g_assert_true(g_variant_equal(payload, received.payload));
    gnoblin_runtime_packet_clear(&received);
    close(sockets[0]);
    close(sockets[1]);
}

static void test_reject_non_dictionary(void) {
    int sockets[2];
    g_assert_cmpint(socketpair(AF_UNIX, SOCK_SEQPACKET | SOCK_CLOEXEC, 0, sockets), ==, 0);
    g_autoptr(GVariant) payload = g_variant_ref_sink(g_variant_new_string("not a dictionary"));
    g_autoptr(GError) error = NULL;
    g_autoptr(GnoblinRuntimeWriter) writer = gnoblin_runtime_writer_new();
    g_assert_false(
        gnoblin_runtime_writer_queue(writer, GNOBLIN_RUNTIME_PACKET_EVENT, 1, payload, &error));
    g_assert_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT);
    close(sockets[0]);
    close(sockets[1]);
}

static void test_reject_incompatible_header(void) {
    int sockets[2];
    g_assert_cmpint(socketpair(AF_UNIX, SOCK_SEQPACKET | SOCK_CLOEXEC, 0, sockets), ==, 0);
    const guint8 packet[32] = {
        'G', 'N',
        'R', 'T',
        0,   1, /* Previous unfragmented wire version. */
        0,   0, /* Minor. */
        0,   GNOBLIN_RUNTIME_PACKET_HELLO,
        0,   3, /* FIRST | LAST. */
        0,   0,
        0,   0, /* Empty chunk. */
        0,   0,
        0,   0,
        0,   0,
        0,   1, /* Request id. */
        0,   0,
        0,   0, /* Total payload. */
        0,   0,
        0,   0, /* Offset. */
    };
    g_assert_cmpint(send(sockets[0], packet, sizeof packet, 0), ==, sizeof packet);
    GnoblinRuntimePacket received = {0};
    g_autoptr(GError) error = NULL;
    g_autoptr(GnoblinRuntimeReader) reader = gnoblin_runtime_reader_new();
    gboolean available = FALSE;
    g_assert_false(
        gnoblin_runtime_reader_receive(reader, sockets[1], &received, &available, &error));
    g_assert_error(error, G_IO_ERROR, G_IO_ERROR_NOT_SUPPORTED);
    close(sockets[0]);
    close(sockets[1]);
}

static void test_receive_does_not_wait_on_blocking_fd(void) {
    int sockets[2];
    g_assert_cmpint(socketpair(AF_UNIX, SOCK_SEQPACKET | SOCK_CLOEXEC, 0, sockets), ==, 0);
    GnoblinRuntimePacket received = {0};
    g_autoptr(GError) error = NULL;
    g_autoptr(GnoblinRuntimeReader) reader = gnoblin_runtime_reader_new();
    gboolean available = TRUE;
    g_assert_true(
        gnoblin_runtime_reader_receive(reader, sockets[1], &received, &available, &error));
    g_assert_false(available);
    g_assert_no_error(error);
    close(sockets[0]);
    close(sockets[1]);
}

static void test_send_does_not_wait_on_blocking_fd(void) {
    int sockets[2];
    g_assert_cmpint(socketpair(AF_UNIX, SOCK_SEQPACKET | SOCK_CLOEXEC, 0, sockets), ==, 0);
    int buffer_size = 4096;
    g_assert_cmpint(setsockopt(sockets[0], SOL_SOCKET, SO_SNDBUF, &buffer_size, sizeof buffer_size),
                    ==, 0);
    g_autofree guint8* data = g_malloc0(8 * 1024 * 1024);
    GVariant* bytes = g_variant_new_fixed_array(G_VARIANT_TYPE_BYTE, data, 8 * 1024 * 1024, 1);
    GVariantBuilder builder;
    g_variant_builder_init(&builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&builder, "{sv}", "document", bytes);
    g_autoptr(GVariant) payload = g_variant_ref_sink(g_variant_builder_end(&builder));
    g_autoptr(GnoblinRuntimeWriter) writer = gnoblin_runtime_writer_new();
    g_assert_true(
        gnoblin_runtime_writer_queue(writer, GNOBLIN_RUNTIME_PACKET_EVENT, 4, payload, NULL));
    g_autoptr(GError) error = NULL;
    g_assert_false(gnoblin_runtime_writer_flush(writer, sockets[0], &error));
    g_assert_error(error, G_IO_ERROR, G_IO_ERROR_WOULD_BLOCK);
    close(sockets[0]);
    close(sockets[1]);
}

static void test_large_fragmented_transfer(void) {
    int sockets[2];
    g_assert_cmpint(socketpair(AF_UNIX, SOCK_SEQPACKET | SOCK_CLOEXEC, 0, sockets), ==, 0);
    g_autofree guint8* data = g_malloc0(8 * 1024 * 1024);
    for (gsize i = 0; i < 8 * 1024 * 1024; i += 4093)
        data[i] = (guint8)(i >> 12);
    GVariant* bytes = g_variant_new_fixed_array(G_VARIANT_TYPE_BYTE, data, 8 * 1024 * 1024, 1);
    GVariantBuilder builder;
    g_variant_builder_init(&builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&builder, "{sv}", "document", bytes);
    g_autoptr(GVariant) payload = g_variant_ref_sink(g_variant_builder_end(&builder));
    g_autoptr(GnoblinRuntimeWriter) writer = gnoblin_runtime_writer_new();
    g_autoptr(GnoblinRuntimeReader) reader = gnoblin_runtime_reader_new();
    g_autoptr(GError) error = NULL;
    g_assert_true(
        gnoblin_runtime_writer_queue(writer, GNOBLIN_RUNTIME_PACKET_CONFIG, 9, payload, &error));
    GnoblinRuntimePacket packet = {0};
    gboolean available = FALSE;
    for (guint i = 0; i < 10000 && !available; i++) {
        g_autoptr(GError) flush_error = NULL;
        if (!gnoblin_runtime_writer_flush(writer, sockets[0], &flush_error))
            g_assert_error(flush_error, G_IO_ERROR, G_IO_ERROR_WOULD_BLOCK);
        g_assert_true(
            gnoblin_runtime_reader_receive(reader, sockets[1], &packet, &available, &error));
        g_assert_no_error(error);
    }
    g_assert_true(available);
    g_assert_cmpint(packet.type, ==, GNOBLIN_RUNTIME_PACKET_CONFIG);
    g_assert_cmpuint(packet.request_id, ==, 9);
    g_assert_true(g_variant_equal(payload, packet.payload));
    gnoblin_runtime_packet_clear(&packet);
    close(sockets[0]);
    close(sockets[1]);
}

int main(int argc, char** argv) {
    g_test_init(&argc, &argv, NULL);
    g_test_add_func("/runtime-protocol/round-trip", test_round_trip);
    g_test_add_func("/runtime-protocol/host-autostart-packet-round-trip",
                    test_host_autostart_packet_round_trip);
    g_test_add_func("/runtime-protocol/recovery-failed-packet-round-trip",
                    test_recovery_failed_packet_round_trip);
    g_test_add_func("/runtime-protocol/session-state-changed-packet-round-trip",
                    test_session_state_changed_packet_round_trip);
    g_test_add_func("/runtime-protocol/session-state-published-packet-round-trip",
                    test_session_state_published_packet_round_trip);
    g_test_add_func("/runtime-protocol/reject-non-dictionary", test_reject_non_dictionary);
    g_test_add_func("/runtime-protocol/reject-incompatible-header",
                    test_reject_incompatible_header);
    g_test_add_func("/runtime-protocol/nonblocking-receive-on-blocking-fd",
                    test_receive_does_not_wait_on_blocking_fd);
    g_test_add_func("/runtime-protocol/nonblocking-send-on-blocking-fd",
                    test_send_does_not_wait_on_blocking_fd);
    g_test_add_func("/runtime-protocol/large-fragmented-transfer", test_large_fragmented_transfer);
    return g_test_run();
}
