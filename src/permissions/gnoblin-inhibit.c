// SPDX-License-Identifier: LGPL-2.1-or-later
// Portal request leases backed by the standalone screen saver and logind.

#include "config.h"

#include <gio/gio.h>
#include <gio/gunixfdlist.h>
#include <unistd.h>

#include "xdg-desktop-portal-dbus.h"
#include "request.h"
#include "session.h"
#include "gnoblin-inhibit.h"

#define INHIBIT_SUSPEND (1u << 2)
#define INHIBIT_IDLE (1u << 3)
#define INHIBIT_ALL ((1u << 4) - 1)

typedef struct {
    GDBusConnection* session_bus;
    guint idle_cookie;
    guint gnome_cookie;
    int sleep_fd;
    guint frontend_watch;
    Request* request;
} InhibitLease;

static GDBusInterfaceSkeleton* inhibit;
static GDBusConnection* portal_bus;
static GList* monitors;
static gboolean screensaver_active;

typedef struct {
    Session parent;
} InhibitMonitor;

typedef struct {
    SessionClass parent_class;
} InhibitMonitorClass;

G_DEFINE_TYPE(InhibitMonitor, inhibit_monitor, session_get_type())

static void inhibit_monitor_close(Session* session) {
    monitors = g_list_remove(monitors, session);
}

static void inhibit_monitor_class_init(InhibitMonitorClass* klass) {
    ((SessionClass*)klass)->close = inhibit_monitor_close;
}

static void inhibit_monitor_init(InhibitMonitor* monitor) {}

static void emit_monitor_state(Session* session) {
    GVariantBuilder state;

    g_variant_builder_init(&state, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&state, "{sv}", "screensaver-active",
                          g_variant_new_boolean(screensaver_active));
    g_variant_builder_add(&state, "{sv}", "session-state", g_variant_new_uint32(1));
    xdp_impl_inhibit_emit_state_changed(XDP_IMPL_INHIBIT(inhibit), session->id,
                                        g_variant_builder_end(&state));
}

static gboolean read_screensaver_active(GError** error) {
    g_autoptr(GVariant) reply = NULL;

    reply = g_dbus_connection_call_sync(
        portal_bus, "org.freedesktop.ScreenSaver", "/org/freedesktop/ScreenSaver",
        "org.freedesktop.ScreenSaver", "GetActive", NULL, G_VARIANT_TYPE("(b)"),
        G_DBUS_CALL_FLAGS_NO_AUTO_START, 5000, NULL, error);
    if (!reply)
        return FALSE;
    g_variant_get(reply, "(b)", &screensaver_active);
    return TRUE;
}

static void on_screensaver_signal(GDBusConnection* connection, const char* sender, const char* path,
                                  const char* interface, const char* signal, GVariant* parameters,
                                  gpointer data) {
    GList* item;

    g_variant_get(parameters, "(b)", &screensaver_active);
    for (item = monitors; item; item = item->next)
        emit_monitor_state(item->data);
}

static void lease_free(gpointer data) {
    InhibitLease* lease = data;
    g_autoptr(GVariant) reply = NULL;
    g_autoptr(GError) error = NULL;

    if (lease->frontend_watch)
        g_bus_unwatch_name(lease->frontend_watch);
    if (lease->idle_cookie) {
        reply = g_dbus_connection_call_sync(
            lease->session_bus, "org.freedesktop.ScreenSaver", "/org/freedesktop/ScreenSaver",
            "org.freedesktop.ScreenSaver", "UnInhibit", g_variant_new("(u)", lease->idle_cookie),
            NULL, G_DBUS_CALL_FLAGS_NO_AUTO_START, 5000, NULL, &error);
        if (!reply)
            g_warning("Could not release idle inhibitor: %s", error->message);
    }
    if (lease->gnome_cookie) {
        g_clear_pointer(&reply, g_variant_unref);
        g_clear_error(&error);
        reply = g_dbus_connection_call_sync(
            lease->session_bus, "org.gnome.SessionManager", "/org/gnome/SessionManager",
            "org.gnome.SessionManager", "Uninhibit", g_variant_new("(u)", lease->gnome_cookie),
            NULL, G_DBUS_CALL_FLAGS_NO_AUTO_START, 5000, NULL, &error);
        if (!reply)
            g_warning("Could not release GNOME session inhibitor: %s", error->message);
    }
    if (lease->sleep_fd >= 0)
        close(lease->sleep_fd);
    g_clear_object(&lease->session_bus);
    g_free(lease);
}

static gboolean gnome_session_is_running(GDBusConnection* connection) {
    g_auto(GStrv) desktops = NULL;
    g_autoptr(GVariant) reply = NULL;
    gboolean owned = FALSE;

    /* The user bus can outlive a graphical login. A leftover GNOME session
     * owner must not redirect inhibitors from Gnoblin's standalone session. */
    desktops = g_strsplit(g_getenv("XDG_CURRENT_DESKTOP") ?: "", ":", -1);
    if (!g_strv_contains((const char* const*)desktops, "GNOME"))
        return FALSE;

    reply = g_dbus_connection_call_sync(
        connection, "org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus",
        "NameHasOwner", g_variant_new("(s)", "org.gnome.SessionManager"), G_VARIANT_TYPE("(b)"),
        G_DBUS_CALL_FLAGS_NONE, 5000, NULL, NULL);
    if (reply)
        g_variant_get(reply, "(b)", &owned);
    return owned;
}

static gboolean take_gnome_inhibitor(InhibitLease* lease, const char* app_id, const char* reason,
                                     guint flags, GError** error) {
    g_autoptr(GVariant) reply = NULL;

    reply = g_dbus_connection_call_sync(
        lease->session_bus, "org.gnome.SessionManager", "/org/gnome/SessionManager",
        "org.gnome.SessionManager", "Inhibit", g_variant_new("(susu)", app_id, 0u, reason, flags),
        G_VARIANT_TYPE("(u)"), G_DBUS_CALL_FLAGS_NO_AUTO_START, 5000, NULL, error);
    if (!reply)
        return FALSE;
    g_variant_get(reply, "(u)", &lease->gnome_cookie);
    return TRUE;
}

static gboolean take_idle_inhibitor(InhibitLease* lease, const char* app_id, const char* reason,
                                    GError** error) {
    g_autoptr(GVariant) reply = NULL;

    reply = g_dbus_connection_call_sync(
        lease->session_bus, "org.freedesktop.ScreenSaver", "/org/freedesktop/ScreenSaver",
        "org.freedesktop.ScreenSaver", "Inhibit", g_variant_new("(ss)", app_id, reason),
        G_VARIANT_TYPE("(u)"), G_DBUS_CALL_FLAGS_NO_AUTO_START, 5000, NULL, error);
    if (!reply)
        return FALSE;
    g_variant_get(reply, "(u)", &lease->idle_cookie);
    return TRUE;
}

static gboolean take_sleep_inhibitor(InhibitLease* lease, const char* app_id, const char* reason,
                                     GError** error) {
    g_autoptr(GDBusConnection) system_bus = NULL;
    g_autoptr(GUnixFDList) fd_list = NULL;
    g_autoptr(GVariant) reply = NULL;
    gint index;

    system_bus = g_bus_get_sync(G_BUS_TYPE_SYSTEM, NULL, error);
    if (!system_bus)
        return FALSE;

    reply = g_dbus_connection_call_with_unix_fd_list_sync(
        system_bus, "org.freedesktop.login1", "/org/freedesktop/login1",
        "org.freedesktop.login1.Manager", "Inhibit",
        g_variant_new("(ssss)", "sleep", app_id, reason, "block"), G_VARIANT_TYPE("(h)"),
        G_DBUS_CALL_FLAGS_NONE, 5000, NULL, &fd_list, NULL, error);
    if (!reply)
        return FALSE;
    if (!fd_list) {
        g_set_error_literal(error, G_DBUS_ERROR, G_DBUS_ERROR_INVALID_ARGS,
                            "logind did not return an inhibitor file descriptor");
        return FALSE;
    }
    g_variant_get(reply, "(h)", &index);
    lease->sleep_fd = g_unix_fd_list_get(fd_list, index, error);
    return lease->sleep_fd >= 0;
}

static void handle_frontend_disappeared(GDBusConnection* connection, const char* name,
                                        gpointer data) {
    InhibitLease* lease = data;
    g_autoptr(Request) request = g_object_ref(lease->request);

    g_object_set_data(G_OBJECT(request), "gnoblin-inhibit-lease", NULL);
    if (request->exported)
        request_unexport(request);
}

static gboolean handle_close(XdpImplRequest* object, GDBusMethodInvocation* invocation,
                             gpointer data) {
    g_autoptr(Request) request = g_object_ref((Request*)object);

    if (!g_str_equal(g_dbus_method_invocation_get_sender(invocation), request->sender)) {
        g_dbus_method_invocation_return_error_literal(
            invocation, G_DBUS_ERROR, G_DBUS_ERROR_ACCESS_DENIED,
            "Only the portal frontend can close this request");
        return TRUE;
    }

    /* Releasing the lease closes the logind FD and revokes the screen saver
     * cookie before the frontend observes Close completing. */
    g_object_set_data(G_OBJECT(request), "gnoblin-inhibit-lease", NULL);
    xdp_impl_request_complete_close(object, invocation);
    if (request->exported)
        request_unexport(request);
    return TRUE;
}

static gboolean handle_create_monitor(XdpImplInhibit* object, GDBusMethodInvocation* invocation,
                                      const char* handle, const char* session_handle,
                                      const char* app_id, const char* window) {
    g_autoptr(GError) error = NULL;
    Session* session;

    if (!read_screensaver_active(&error)) {
        g_warning("Could not read screen saver state: %s", error->message);
        xdp_impl_inhibit_complete_create_monitor(object, invocation, 2);
        return TRUE;
    }

    session = g_object_new(inhibit_monitor_get_type(), "id", session_handle, NULL);
    if (!session_export(session, g_dbus_method_invocation_get_connection(invocation), &error)) {
        g_warning("Could not create inhibit monitor: %s", error->message);
        g_object_unref(session);
        xdp_impl_inhibit_complete_create_monitor(object, invocation, 2);
        return TRUE;
    }

    monitors = g_list_prepend(monitors, session);
    xdp_impl_inhibit_complete_create_monitor(object, invocation, 0);
    emit_monitor_state(session);
    return TRUE;
}

static gboolean handle_query_end_response(XdpImplInhibit* object, GDBusMethodInvocation* invocation,
                                          const char* session_handle) {
    Session* session = lookup_session(session_handle);

    if (!session || !G_TYPE_CHECK_INSTANCE_TYPE(session, inhibit_monitor_get_type())) {
        g_dbus_method_invocation_return_error_literal(
            invocation, G_DBUS_ERROR, G_DBUS_ERROR_ACCESS_DENIED, "Unknown inhibit monitor");
        return TRUE;
    }
    xdp_impl_inhibit_complete_query_end_response(object, invocation);
    return TRUE;
}

static gboolean handle_inhibit(XdpImplInhibit* object, GDBusMethodInvocation* invocation,
                               const char* handle, const char* app_id, const char* window,
                               guint flags, GVariant* options) {
    g_autoptr(Request) request = NULL;
    g_autoptr(GError) error = NULL;
    InhibitLease* lease;
    const char* reason = "";
    const char* sender = g_dbus_method_invocation_get_sender(invocation);
    const char* application = app_id[0] ? app_id : sender;
    gboolean use_gnome_session;

    if (flags == 0 || (flags & ~INHIBIT_ALL) != 0) {
        g_dbus_method_invocation_return_error_literal(
            invocation, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT, "Invalid inhibit flags");
        return TRUE;
    }

    g_variant_lookup(options, "reason", "&s", &reason);
    use_gnome_session =
        gnome_session_is_running(g_dbus_method_invocation_get_connection(invocation));
    if (!use_gnome_session && (flags & ~(INHIBIT_IDLE | INHIBIT_SUSPEND)) != 0) {
        g_dbus_method_invocation_return_error_literal(
            invocation, G_IO_ERROR, G_IO_ERROR_NOT_SUPPORTED,
            "This session supports idle and suspend inhibition only");
        return TRUE;
    }

    lease = g_new0(InhibitLease, 1);
    lease->session_bus = g_object_ref(g_dbus_method_invocation_get_connection(invocation));
    lease->sleep_fd = -1;

    if (use_gnome_session) {
        if (!take_gnome_inhibitor(lease, application, reason, flags, &error))
            goto failed;
    } else {
        if ((flags & INHIBIT_IDLE) && !take_idle_inhibitor(lease, application, reason, &error))
            goto failed;
        if ((flags & INHIBIT_SUSPEND) && !take_sleep_inhibitor(lease, application, reason, &error))
            goto failed;
    }

    request = request_new(sender, app_id, handle);
    g_signal_connect(request, "handle-close", G_CALLBACK(handle_close), NULL);
    if (!g_dbus_interface_skeleton_export(G_DBUS_INTERFACE_SKELETON(request), lease->session_bus,
                                          handle, &error))
        goto failed;
    request->exported = TRUE;
    g_object_ref(request);
    lease->request = request;
    g_object_set_data_full(G_OBJECT(request), "gnoblin-inhibit-lease", lease, lease_free);
    lease->frontend_watch =
        g_bus_watch_name_on_connection(lease->session_bus, sender, G_BUS_NAME_WATCHER_FLAGS_NONE,
                                       NULL, handle_frontend_disappeared, lease, NULL);
    xdp_impl_inhibit_complete_inhibit(object, invocation);
    return TRUE;

failed:
    lease_free(lease);
    g_dbus_method_invocation_return_gerror(invocation, error);
    return TRUE;
}

gboolean gnoblin_inhibit_init(GDBusConnection* connection, GError** error) {
    portal_bus = g_object_ref(connection);
    inhibit = G_DBUS_INTERFACE_SKELETON(xdp_impl_inhibit_skeleton_new());
    g_signal_connect(inhibit, "handle-inhibit", G_CALLBACK(handle_inhibit), NULL);
    g_signal_connect(inhibit, "handle-create-monitor", G_CALLBACK(handle_create_monitor), NULL);
    g_signal_connect(inhibit, "handle-query-end-response", G_CALLBACK(handle_query_end_response),
                     NULL);
    if (!g_dbus_interface_skeleton_export(inhibit, connection, "/org/freedesktop/portal/desktop",
                                          error))
        return FALSE;
    g_dbus_connection_signal_subscribe(connection, "org.freedesktop.ScreenSaver",
                                       "org.freedesktop.ScreenSaver", "ActiveChanged",
                                       "/org/freedesktop/ScreenSaver", NULL,
                                       G_DBUS_SIGNAL_FLAGS_NONE, on_screensaver_signal, NULL, NULL);
    g_debug("providing %s", g_dbus_interface_skeleton_get_info(inhibit)->name);
    return TRUE;
}
