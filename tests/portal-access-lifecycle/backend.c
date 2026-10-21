/* SPDX-License-Identifier: LGPL-2.1-or-later */
/* Real Access handler and GTK dialog. Only external parenting is a substitute; tests use an
 * isolated Broadway display and private bus. */
#include <stdio.h>
#include "access.c"

GxdpExternalWindow* gxdp_external_window_new_from_handle(const char* handle) {
    g_assert_not_reached();
}

void gxdp_external_window_set_parent_of(GxdpExternalWindow* window, GdkSurface* surface) {
    g_assert_not_reached();
}

static void control(GDBusConnection* bus, const char* sender, const char* path,
                    const char* interface, const char* method, GVariant* parameters,
                    GDBusMethodInvocation* invocation, gpointer data) {
    GListModel* windows = gtk_window_get_toplevels();
    guint mapped = 0;
    guint count = g_list_model_get_n_items(windows);
    if (g_str_equal(method, "Respond")) {
        const char* response;
        g_variant_get(parameters, "(&s)", &response);
        for (guint i = 0; i < count; i++) {
            g_autoptr(GtkWidget) window = g_list_model_get_item(windows, i);
            if (ADW_IS_MESSAGE_DIALOG(window) && gtk_widget_get_mapped(window)) {
                adw_message_dialog_response(ADW_MESSAGE_DIALOG(window), response);
                break;
            }
        }
        g_dbus_method_invocation_return_value(invocation, NULL);
        return;
    }
    for (guint i = 0; i < count; i++) {
        g_autoptr(GtkWidget) window = g_list_model_get_item(windows, i);
        mapped += gtk_widget_get_mapped(window);
    }
    g_dbus_method_invocation_return_value(invocation, g_variant_new("(uu)", mapped, count));
}

static gboolean screenshot_unused(XdpImplScreenshot* impl, GDBusMethodInvocation* invocation,
                                  const char* handle, const char* app, const char* parent,
                                  GVariant* options, gpointer data) {
    g_print("UNEXPECTED SCREENSHOT\n");
    xdp_impl_screenshot_complete_screenshot(impl, invocation, 2,
                                            g_variant_new_array(G_VARIANT_TYPE("{sv}"), NULL, 0));
    return TRUE;
}

int main(void) {
    const char* xml =
        "<node><interface name='org.gnoblin.AccessTest'>"
        "<method name='Stats'><arg type='u' direction='out'/><arg type='u' "
        "direction='out'/></method>"
        "<method name='Respond'><arg type='s' direction='in'/></method></interface></node>";
    setvbuf(stdout, NULL, _IOLBF, 0);
    adw_init();
    g_autoptr(GError) error = NULL;
    g_autoptr(GDBusConnection) bus = g_bus_get_sync(G_BUS_TYPE_SESSION, NULL, &error);
    g_assert_no_error(error);
    g_autoptr(GDBusNodeInfo) info = g_dbus_node_info_new_for_xml(xml, &error);
    const GDBusInterfaceVTable vtable = {.method_call = control};
    g_dbus_connection_register_object(bus, "/org/gnoblin/AccessTest", info->interfaces[0], &vtable,
                                      NULL, NULL, &error);
    g_assert_no_error(error);
    g_assert_true(access_init(bus, &error));
    g_assert_no_error(error);
    XdpImplScreenshot* screenshot = xdp_impl_screenshot_skeleton_new();
    xdp_impl_screenshot_set_version(screenshot, 2);
    g_signal_connect(screenshot, "handle-screenshot", G_CALLBACK(screenshot_unused), NULL);
    g_assert_true(g_dbus_interface_skeleton_export(G_DBUS_INTERFACE_SKELETON(screenshot), bus,
                                                   DESKTOP_PORTAL_OBJECT_PATH, &error));
    g_assert_no_error(error);
    g_bus_own_name_on_connection(bus, "org.freedesktop.impl.portal.desktop.gnoblin",
                                 G_BUS_NAME_OWNER_FLAGS_NONE, NULL, NULL, NULL, NULL);
    g_print("READY\n");
    g_autoptr(GMainLoop) loop = g_main_loop_new(NULL, FALSE);
    g_main_loop_run(loop);
    return 0;
}
