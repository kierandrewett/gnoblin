#include "gnoblin-autostart.h"

#include <string.h>

gboolean gnoblin_autostart_receive_packet(int fd, GnoblinRuntimeReader* reader,
                                          gboolean* received_packet, GVariant** entries,
                                          GVariant** environment, GError** error) {
    g_return_val_if_fail(reader && received_packet && entries && environment, FALSE);
    g_return_val_if_fail(error == NULL || *error == NULL, FALSE);

    for (;;) {
        GnoblinRuntimePacket packet = {0};
        gboolean available = FALSE;
        g_autoptr(GError) receive_error = NULL;
        if (!gnoblin_runtime_reader_receive(reader, fd, &packet, &available, &receive_error)) {
            if (*received_packet && g_error_matches(receive_error, G_IO_ERROR, G_IO_ERROR_CLOSED))
                return TRUE;
            g_propagate_error(error, g_steal_pointer(&receive_error));
            return FALSE;
        }
        if (!available)
            return TRUE;

        g_autoptr(GVariant) payload = g_variant_ref(packet.payload);
        gboolean valid = packet.type == GNOBLIN_RUNTIME_PACKET_HOST_AUTOSTART &&
                         packet.request_id == 0 && !*received_packet;
        guint entries_fields = 0;
        guint environment_fields = 0;
        GVariantIter payload_iter;
        const char* payload_key;
        GVariant* payload_value;
        g_variant_iter_init(&payload_iter, payload);
        while (g_variant_iter_next(&payload_iter, "{&sv}", &payload_key, &payload_value)) {
            if (g_str_equal(payload_key, "entries"))
                entries_fields++;
            else if (g_str_equal(payload_key, "environment"))
                environment_fields++;
            else
                valid = FALSE;
            g_variant_unref(payload_value);
        }
        g_autoptr(GVariant) packet_entries =
            valid ? g_variant_lookup_value(payload, "entries", G_VARIANT_TYPE("av")) : NULL;
        g_autoptr(GVariant) packet_environment =
            valid ? g_variant_lookup_value(payload, "environment", G_VARIANT_TYPE("a{ss}")) : NULL;
        valid = valid && entries_fields == 1 && environment_fields == 1 && packet_entries &&
                packet_environment;
        gnoblin_runtime_packet_clear(&packet);
        if (!valid) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                                "Lua runtime sent an invalid initial autostart packet");
            return FALSE;
        }
        *entries = g_steal_pointer(&packet_entries);
        *environment = g_steal_pointer(&packet_environment);
        *received_packet = TRUE;
    }
}

static gboolean valid_environment_value(const char* value) {
    if (!value || !*value || strlen(value) > 4096 || !g_utf8_validate(value, -1, NULL))
        return FALSE;
    for (const char* cursor = value; *cursor; cursor = g_utf8_next_char(cursor)) {
        if (g_unichar_iscntrl(g_utf8_get_char(cursor)))
            return FALSE;
    }
    return TRUE;
}

char** gnoblin_autostart_build_environment(GVariant* environment, GError** error) {
    g_return_val_if_fail(error == NULL || *error == NULL, NULL);
    if (!environment || !g_variant_is_of_type(environment, G_VARIANT_TYPE("a{ss}"))) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                            "Lua runtime sent an invalid session environment");
        return NULL;
    }
    const char* wayland_display = NULL;
    const char* display = NULL;
    const char* xauthority = NULL;
    GVariantIter iter;
    const char* key;
    const char* value;
    g_variant_iter_init(&iter, environment);
    while (g_variant_iter_next(&iter, "{&s&s}", &key, &value)) {
        const char** destination = NULL;
        if (g_str_equal(key, "WAYLAND_DISPLAY"))
            destination = &wayland_display;
        else if (g_str_equal(key, "DISPLAY"))
            destination = &display;
        else if (g_str_equal(key, "XAUTHORITY"))
            destination = &xauthority;
        if (!destination || *destination || !valid_environment_value(value)) {
            g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                        "Mutter HELLO contains an invalid session environment variable: %s", key);
            return NULL;
        }
        *destination = value;
    }
    if (!wayland_display) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                            "Mutter HELLO is missing required WAYLAND_DISPLAY");
        return NULL;
    }

    char** child_environment = g_get_environ();
    child_environment = g_environ_unsetenv(child_environment, "WAYLAND_DISPLAY");
    child_environment = g_environ_unsetenv(child_environment, "DISPLAY");
    child_environment = g_environ_unsetenv(child_environment, "XAUTHORITY");
    child_environment =
        g_environ_setenv(child_environment, "WAYLAND_DISPLAY", wayland_display, TRUE);
    if (display)
        child_environment = g_environ_setenv(child_environment, "DISPLAY", display, TRUE);
    if (xauthority)
        child_environment = g_environ_setenv(child_environment, "XAUTHORITY", xauthority, TRUE);
    return child_environment;
}
