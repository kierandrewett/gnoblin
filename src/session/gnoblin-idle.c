// SPDX-License-Identifier: GPL-2.0-or-later
// Standalone session idle policy and freedesktop screen saver inhibition.

#include <gio/gio.h>
#include <gio/gunixfdlist.h>
#include <unistd.h>

#define SCREENSAVER_NAME "org.freedesktop.ScreenSaver"
#define SCREENSAVER_PATH "/org/freedesktop/ScreenSaver"
#define MUTTER_NAME "org.gnome.Mutter.IdleMonitor"
#define MUTTER_PATH "/org/gnome/Mutter/IdleMonitor/Core"
#define MUTTER_INTERFACE "org.gnome.Mutter.IdleMonitor"
#define GNOME_SCREENSAVER_NAME "org.gnome.ScreenSaver"
#define GNOME_SCREENSAVER_PATH "/org/gnome/ScreenSaver"
#define SESSION_IDLE_PATH "/org/gnoblin/SessionIdle"
#define SESSION_IDLE_INTERFACE "org.gnoblin.SessionIdle"

static const char introspection_xml[] =
    "<node><interface name='org.freedesktop.ScreenSaver'>"
    "<method name='Inhibit'><arg type='s' direction='in'/>"
    "<arg type='s' direction='in'/><arg type='u' direction='out'/></method>"
    "<method name='UnInhibit'><arg type='u' direction='in'/></method>"
    "<method name='GetActive'><arg type='b' direction='out'/></method>"
    "<method name='SetActive'><arg type='b' direction='in'/></method>"
    "<method name='Lock'/>"
    "<signal name='ActiveChanged'><arg type='b'/></signal>"
    "</interface></node>";

typedef struct {
    char* sender;
    int logind_fd;
} Inhibitor;

typedef struct {
    GDBusConnection* session_bus;
    GDBusConnection* system_bus;
    GDBusProxy* idle_monitor;
    GSettings* session_settings;
    GHashTable* inhibitors;
    GMainLoop* loop;
    guint next_cookie;
    guint idle_watch;
    guint idle_name_watch;
    guint dbus_owner_signal;
    guint screensaver_signal;
    gboolean name_lost;
} IdleService;

static void maybe_activate_idle(IdleService* service);

static void inhibitor_free(gpointer data) {
    Inhibitor* inhibitor = data;
    if (inhibitor->logind_fd >= 0)
        close(inhibitor->logind_fd);
    g_free(inhibitor->sender);
    g_free(inhibitor);
}

static GVariant* call_screen_saver(IdleService* service, const char* method, GVariant* parameters,
                                   const GVariantType* reply_type, GError** error) {
    return g_dbus_connection_call_sync(service->session_bus, GNOME_SCREENSAVER_NAME,
                                       GNOME_SCREENSAVER_PATH, GNOME_SCREENSAVER_NAME, method,
                                       parameters, reply_type, G_DBUS_CALL_FLAGS_NONE, 5000, NULL,
                                       error);
}

static void maybe_activate_idle(IdleService* service) {
    g_autoptr(GVariant) reply = NULL;
    g_autoptr(GError) error = NULL;
    guint delay = g_settings_get_uint(service->session_settings, "idle-delay");
    guint64 elapsed = 0;

    if (!service->idle_monitor || delay == 0 || g_hash_table_size(service->inhibitors) != 0)
        return;

    reply = g_dbus_proxy_call_sync(service->idle_monitor, "GetIdletime", NULL,
                                   G_DBUS_CALL_FLAGS_NONE, 5000, NULL, &error);
    if (!reply) {
        g_warning("Could not read idle time: %s", error->message);
        return;
    }
    g_variant_get(reply, "(t)", &elapsed);
    if (elapsed >= (guint64)delay * 1000) {
        /* Shell owns the fade, wake-up and lock delay. Its normal presence
         * handler consumes this private signal in the standalone session. */
        if (!g_dbus_connection_emit_signal(service->session_bus, NULL, SESSION_IDLE_PATH,
                                           SESSION_IDLE_INTERFACE, "Idle", NULL, &error))
            g_warning("Could not deliver idle event: %s", error->message);
    }
}

static void replace_idle_watch(IdleService* service) {
    g_autoptr(GVariant) reply = NULL;
    g_autoptr(GError) error = NULL;
    guint delay = g_settings_get_uint(service->session_settings, "idle-delay");

    if (!service->idle_monitor)
        return;
    if (service->idle_watch)
        g_dbus_proxy_call_sync(service->idle_monitor, "RemoveWatch",
                               g_variant_new("(u)", service->idle_watch), G_DBUS_CALL_FLAGS_NONE,
                               5000, NULL, NULL);
    service->idle_watch = 0;
    if (delay == 0)
        return;

    reply = g_dbus_proxy_call_sync(service->idle_monitor, "AddIdleWatch",
                                   g_variant_new("(t)", (guint64)delay * 1000),
                                   G_DBUS_CALL_FLAGS_NONE, 5000, NULL, &error);
    if (!reply) {
        g_warning("Could not register idle watch: %s", error->message);
        return;
    }
    g_variant_get(reply, "(u)", &service->idle_watch);
}

static void on_idle_signal(GDBusProxy* proxy, const char* sender, const char* signal,
                           GVariant* parameters, gpointer data) {
    IdleService* service = data;
    guint watch;

    if (!g_str_equal(signal, "WatchFired"))
        return;
    g_variant_get(parameters, "(u)", &watch);
    if (watch == service->idle_watch)
        maybe_activate_idle(service);
}

static void on_idle_appeared(GDBusConnection* connection, const char* name, const char* owner,
                             gpointer data) {
    IdleService* service = data;
    g_autoptr(GError) error = NULL;

    g_clear_object(&service->idle_monitor);
    service->idle_monitor =
        g_dbus_proxy_new_sync(connection, G_DBUS_PROXY_FLAGS_DO_NOT_AUTO_START, NULL, MUTTER_NAME,
                              MUTTER_PATH, MUTTER_INTERFACE, NULL, &error);
    if (!service->idle_monitor) {
        g_warning("Could not connect to Mutter idle monitor: %s", error->message);
        return;
    }
    g_signal_connect(service->idle_monitor, "g-signal", G_CALLBACK(on_idle_signal), service);
    replace_idle_watch(service);
}

static void on_idle_vanished(GDBusConnection* connection, const char* name, gpointer data) {
    IdleService* service = data;
    service->idle_watch = 0;
    g_clear_object(&service->idle_monitor);
}

static void on_idle_setting_changed(GSettings* settings, const char* key, gpointer data) {
    replace_idle_watch(data);
}

static gboolean remove_disconnected_inhibitor(gpointer key, gpointer value, gpointer data) {
    Inhibitor* inhibitor = value;
    return g_str_equal(inhibitor->sender, data);
}

static void on_name_owner_changed(GDBusConnection* connection, const char* sender, const char* path,
                                  const char* interface, const char* signal, GVariant* parameters,
                                  gpointer data) {
    IdleService* service = data;
    const char *name, *old_owner, *new_owner;

    g_variant_get(parameters, "(&s&s&s)", &name, &old_owner, &new_owner);
    if (name[0] != ':' || new_owner[0] != '\0')
        return;
    if (g_hash_table_foreach_remove(service->inhibitors, remove_disconnected_inhibitor,
                                    (gpointer)name) != 0)
        maybe_activate_idle(service);
}

static int take_logind_idle_lock(IdleService* service, const char* application, const char* reason,
                                 GError** error) {
    g_autoptr(GUnixFDList) fd_list = NULL;
    g_autoptr(GVariant) reply = NULL;
    gint index;

    reply = g_dbus_connection_call_with_unix_fd_list_sync(
        service->system_bus, "org.freedesktop.login1", "/org/freedesktop/login1",
        "org.freedesktop.login1.Manager", "Inhibit",
        g_variant_new("(ssss)", "idle", application, reason, "block"), G_VARIANT_TYPE("(h)"),
        G_DBUS_CALL_FLAGS_NONE, 5000, NULL, &fd_list, NULL, error);
    if (!reply)
        return -1;
    g_variant_get(reply, "(h)", &index);
    if (!fd_list) {
        g_set_error_literal(error, G_DBUS_ERROR, G_DBUS_ERROR_INVALID_ARGS,
                            "logind did not return an inhibitor file descriptor");
        return -1;
    }
    return g_unix_fd_list_get(fd_list, index, error);
}

static void handle_method_call(GDBusConnection* connection, const char* sender, const char* path,
                               const char* interface, const char* method, GVariant* parameters,
                               GDBusMethodInvocation* invocation, gpointer data) {
    IdleService* service = data;
    g_autoptr(GError) error = NULL;
    g_autoptr(GVariant) reply = NULL;

    if (g_str_equal(method, "Inhibit")) {
        const char *application, *reason;
        Inhibitor* inhibitor;
        guint cookie;
        gint fd;

        g_variant_get(parameters, "(&s&s)", &application, &reason);
        fd = take_logind_idle_lock(service, application[0] ? application : sender, reason, &error);
        if (fd < 0) {
            g_dbus_method_invocation_return_gerror(invocation, error);
            return;
        }
        do
            cookie = ++service->next_cookie;
        while (cookie == 0 || g_hash_table_contains(service->inhibitors, GUINT_TO_POINTER(cookie)));
        inhibitor = g_new0(Inhibitor, 1);
        inhibitor->sender = g_strdup(sender);
        inhibitor->logind_fd = fd;
        g_hash_table_insert(service->inhibitors, GUINT_TO_POINTER(cookie), inhibitor);
        g_dbus_method_invocation_return_value(invocation, g_variant_new("(u)", cookie));
        return;
    }

    if (g_str_equal(method, "UnInhibit")) {
        Inhibitor* inhibitor;
        guint cookie;
        g_variant_get(parameters, "(u)", &cookie);
        inhibitor = g_hash_table_lookup(service->inhibitors, GUINT_TO_POINTER(cookie));
        if (!inhibitor || !g_str_equal(inhibitor->sender, sender)) {
            g_dbus_method_invocation_return_error_literal(
                invocation, G_DBUS_ERROR, G_DBUS_ERROR_ACCESS_DENIED,
                "No inhibitor owned by this caller has that cookie");
            return;
        }
        g_hash_table_remove(service->inhibitors, GUINT_TO_POINTER(cookie));
        g_dbus_method_invocation_return_value(invocation, NULL);
        maybe_activate_idle(service);
        return;
    }

    if (g_str_equal(method, "GetActive")) {
        reply = call_screen_saver(service, "GetActive", NULL, G_VARIANT_TYPE("(b)"), &error);
    } else if (g_str_equal(method, "SetActive")) {
        reply = call_screen_saver(service, "SetActive", parameters, NULL, &error);
    } else if (g_str_equal(method, "Lock")) {
        reply = call_screen_saver(service, "Lock", NULL, NULL, &error);
    }
    if (!reply) {
        g_dbus_method_invocation_return_gerror(invocation, error);
        return;
    }
    g_dbus_method_invocation_return_value(invocation, g_steal_pointer(&reply));
}

static const GDBusInterfaceVTable interface_vtable = {
    .method_call = handle_method_call,
};

static void on_screen_saver_active_changed(GDBusConnection* connection, const char* sender,
                                           const char* path, const char* interface,
                                           const char* signal, GVariant* parameters,
                                           gpointer data) {
    IdleService* service = data;
    gboolean active;
    g_variant_get(parameters, "(b)", &active);
    g_dbus_connection_emit_signal(service->session_bus, NULL, SCREENSAVER_PATH, SCREENSAVER_NAME,
                                  "ActiveChanged", g_variant_new("(b)", active), NULL);
}

static void on_name_acquired(GDBusConnection* connection, const char* name, gpointer data) {
    IdleService* service = data;
    service->idle_name_watch =
        g_bus_watch_name_on_connection(connection, MUTTER_NAME, G_BUS_NAME_WATCHER_FLAGS_NONE,
                                       on_idle_appeared, on_idle_vanished, service, NULL);
}

static void on_name_lost(GDBusConnection* connection, const char* name, gpointer data) {
    IdleService* service = data;
    g_warning("Could not own %s; another screen saver may be running", name);
    service->name_lost = TRUE;
    g_main_loop_quit(service->loop);
}

int main(int argc, char** argv) {
    IdleService service = {0};
    g_autoptr(GDBusNodeInfo) node = NULL;
    g_autoptr(GError) error = NULL;
    guint name_id;

    service.session_bus = g_bus_get_sync(G_BUS_TYPE_SESSION, NULL, &error);
    if (!service.session_bus)
        g_error("Could not connect to the session bus: %s", error->message);
    service.system_bus = g_bus_get_sync(G_BUS_TYPE_SYSTEM, NULL, &error);
    if (!service.system_bus)
        g_error("Could not connect to logind: %s", error->message);

    node = g_dbus_node_info_new_for_xml(introspection_xml, &error);
    if (!node)
        g_error("Could not define screen saver interface: %s", error->message);
    if (!g_dbus_connection_register_object(service.session_bus, SCREENSAVER_PATH,
                                           node->interfaces[0], &interface_vtable, &service, NULL,
                                           &error))
        g_error("Could not export screen saver interface: %s", error->message);

    service.inhibitors = g_hash_table_new_full(g_direct_hash, g_direct_equal, NULL, inhibitor_free);
    service.session_settings = g_settings_new("org.gnome.desktop.session");
    g_signal_connect(service.session_settings, "changed::idle-delay",
                     G_CALLBACK(on_idle_setting_changed), &service);
    service.dbus_owner_signal = g_dbus_connection_signal_subscribe(
        service.session_bus, "org.freedesktop.DBus", "org.freedesktop.DBus", "NameOwnerChanged",
        "/org/freedesktop/DBus", NULL, G_DBUS_SIGNAL_FLAGS_NONE, on_name_owner_changed, &service,
        NULL);
    service.screensaver_signal = g_dbus_connection_signal_subscribe(
        service.session_bus, GNOME_SCREENSAVER_NAME, GNOME_SCREENSAVER_NAME, "ActiveChanged",
        GNOME_SCREENSAVER_PATH, NULL, G_DBUS_SIGNAL_FLAGS_NONE, on_screen_saver_active_changed,
        &service, NULL);
    service.loop = g_main_loop_new(NULL, FALSE);
    name_id = g_bus_own_name_on_connection(service.session_bus, SCREENSAVER_NAME,
                                           G_BUS_NAME_OWNER_FLAGS_NONE, on_name_acquired,
                                           on_name_lost, &service, NULL);
    g_main_loop_run(service.loop);
    g_bus_unown_name(name_id);
    if (service.idle_name_watch)
        g_bus_unwatch_name(service.idle_name_watch);
    g_dbus_connection_signal_unsubscribe(service.session_bus, service.dbus_owner_signal);
    g_dbus_connection_signal_unsubscribe(service.session_bus, service.screensaver_signal);
    g_hash_table_unref(service.inhibitors);
    g_clear_object(&service.idle_monitor);
    g_clear_object(&service.session_settings);
    g_clear_object(&service.system_bus);
    g_clear_object(&service.session_bus);
    g_main_loop_unref(service.loop);
    return service.name_lost ? 1 : 0;
}
