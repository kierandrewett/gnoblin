// SPDX-License-Identifier: GPL-2.0-or-later
// Exercise the actual method handler on a private bus, with explicit test-only shutdown.
#define main gnoblin_idle_program_main
#include GNOBLIN_IDLE_SOURCE
#undef main

static void quit_method(GDBusConnection* connection, const char* sender, const char* path,
                        const char* interface, const char* method, GVariant* parameters,
                        GDBusMethodInvocation* invocation, gpointer data) {
    g_dbus_method_invocation_return_value(invocation, NULL);
    g_main_loop_quit(data);
}

int main(void) {
    g_autoptr(GError) error = NULL;
    g_autoptr(GDBusConnection) connection =
        g_dbus_connection_new_for_address_sync(g_getenv("DBUS_SESSION_BUS_ADDRESS"),
                                               G_DBUS_CONNECTION_FLAGS_AUTHENTICATION_CLIENT |
                                                   G_DBUS_CONNECTION_FLAGS_MESSAGE_BUS_CONNECTION,
                                               NULL, NULL, &error);
    g_assert_no_error(error);
    g_assert_nonnull(connection);
    g_dbus_connection_set_exit_on_close(connection, FALSE);
    g_autoptr(GMainLoop) loop = g_main_loop_new(NULL, FALSE);
    IdleService service = {.session_bus = connection, .loop = loop};
    g_autoptr(GDBusNodeInfo) node = g_dbus_node_info_new_for_xml(introspection_xml, &error);
    g_assert_no_error(error);
    guint registration =
        g_dbus_connection_register_object(connection, SCREENSAVER_PATH, node->interfaces[0],
                                          &interface_vtable, &service, NULL, &error);
    g_assert_no_error(error);
    g_assert_cmpuint(registration, >, 0);
    g_autoptr(GDBusNodeInfo) control = g_dbus_node_info_new_for_xml(
        "<node><interface name='org.gnoblin.Test'><method name='Quit'/></interface></node>",
        &error);
    g_assert_no_error(error);
    const GDBusInterfaceVTable test_vtable = {.method_call = quit_method};
    guint quit_registration = g_dbus_connection_register_object(
        connection, "/org/gnoblin/Test", control->interfaces[0], &test_vtable, loop, NULL, &error);
    g_assert_no_error(error);
    g_assert_cmpuint(quit_registration, >, 0);
    g_autoptr(GVariant) owned = g_dbus_connection_call_sync(
        connection, "org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus",
        "RequestName", g_variant_new("(su)", SCREENSAVER_NAME, 4), G_VARIANT_TYPE("(u)"),
        G_DBUS_CALL_FLAGS_NONE, 2000, NULL, &error);
    g_assert_no_error(error);
    guint result;
    g_variant_get(owned, "(u)", &result);
    g_assert_cmpuint(result, ==, 1);
    g_main_loop_run(loop);

    g_dbus_connection_unregister_object(connection, registration);
    g_dbus_connection_unregister_object(connection, quit_registration);
    g_assert_true(g_dbus_connection_flush_sync(connection, NULL, &error));
    g_assert_no_error(error);
    g_assert_true(g_dbus_connection_close_sync(connection, NULL, &error));
    g_assert_no_error(error);
    g_clear_object(&connection);
    while (g_main_context_iteration(NULL, FALSE)) {
    }
    return 0;
}
