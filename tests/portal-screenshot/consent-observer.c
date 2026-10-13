/* SPDX-License-Identifier: LGPL-2.1-or-later */
/* Private-test UI observation/control only. No policy/capture interposition. */
#include <adwaita.h>
#include <unistd.h>
static void control(GDBusConnection* bus, const char* sender, const char* path, const char* iface,
                    const char* method, GVariant* params, GDBusMethodInvocation* inv,
                    gpointer data) {
    GListModel* windows = gtk_window_get_toplevels();
    GVariantBuilder result;
    g_variant_builder_init(&result, G_VARIANT_TYPE("a(ssb)"));
    const char* response = NULL;
    if (g_str_equal(method, "Respond"))
        g_variant_get(params, "(&s)", &response);
    for (guint i = 0; i < g_list_model_get_n_items(windows); i++) {
        g_autoptr(GtkWindow) w = g_list_model_get_item(windows, i);
        gboolean mapped = gtk_widget_get_mapped(GTK_WIDGET(w));
        const char* title = gtk_window_get_title(w);
        g_variant_builder_add(&result, "(ssb)", G_OBJECT_TYPE_NAME(w), title ? title : "", mapped);
        if (response && mapped) {
            if (ADW_IS_MESSAGE_DIALOG(w))
                adw_message_dialog_response(ADW_MESSAGE_DIALOG(w), response);
            else if (GTK_IS_DIALOG(w))
                gtk_dialog_response(GTK_DIALOG(w), g_str_equal(response, "grant")
                                                       ? GTK_RESPONSE_ACCEPT
                                                       : GTK_RESPONSE_CANCEL);
            break;
        }
    }
    if (response) {
        g_variant_builder_clear(&result);
        g_dbus_method_invocation_return_value(inv, NULL);
    } else
        g_dbus_method_invocation_return_value(
            inv, g_variant_new("(u@a(ssb))", getpid(), g_variant_builder_end(&result)));
}
static gboolean setup(gpointer data) {
    if (!gtk_is_initialized())
        return G_SOURCE_CONTINUE;
    GDBusConnection* bus = g_bus_get_sync(G_BUS_TYPE_SESSION, NULL, NULL);
    GDBusNodeInfo* info = g_dbus_node_info_new_for_xml(
        "<node><interface name='org.gnoblin.ConsentProbe'><method name='Stats'><arg type='u' "
        "direction='out'/><arg type='a(ssb)' direction='out'/></method><method name='Respond'><arg "
        "type='s' direction='in'/></method></interface></node>",
        NULL);
    static const GDBusInterfaceVTable vt = {.method_call = control};
    g_dbus_connection_register_object(bus, "/org/gnoblin/ConsentProbe", info->interfaces[0], &vt,
                                      NULL, NULL, NULL);
    return G_SOURCE_REMOVE;
}
__attribute__((constructor)) static void start(void) {
    g_timeout_add(20, setup, NULL);
}
