// SPDX-License-Identifier: GPL-2.0-or-later
// Session activity monitoring and freedesktop screen saver inhibition.

#include <gio/gio.h>
#include <gio/gunixfdlist.h>
#include <json-glib/json-glib.h>
#include <string.h>
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
#define ACTIVITY_THRESHOLD_MS 120000

static const char introspection_xml[] =
    "<node><interface name='org.freedesktop.ScreenSaver'>"
    "<method name='Inhibit'><arg type='s' direction='in'/>"
    "<arg type='s' direction='in'/><arg type='u' direction='out'/></method>"
    "<method name='UnInhibit'><arg type='u' direction='in'/></method>"
    "<method name='GetActive'><arg type='b' direction='out'/></method>"
    "<method name='SetActive'><arg type='b' direction='in'/></method>"
    "<method name='Lock'/>"
    "<signal name='ActiveChanged'><arg type='b'/></signal>"
    "</interface>"
    "<interface name='org.gnoblin.SessionIdle'>"
    "<method name='GetActivity'>"
    "<arg name='available' type='b' direction='out'/>"
    "<arg name='idle' type='b' direction='out'/>"
    "<arg name='threshold_ms' type='t' direction='out'/>"
    "<arg name='idle_for_ms' type='t' direction='out'/>"
    "</method>"
    "<signal name='ActivityChanged'>"
    "<arg name='available' type='b'/><arg name='idle' type='b'/>"
    "<arg name='threshold_ms' type='t'/><arg name='idle_for_ms' type='t'/>"
    "</signal>"
    "</interface></node>";

typedef struct {
    char* sender;
    int logind_fd;
} Inhibitor;

typedef struct {
    GDBusConnection* session_bus;
    GDBusConnection* system_bus;
    GDBusProxy* idle_monitor;
    GHashTable* inhibitors;
    GMainLoop* loop;
    guint next_cookie;
    guint activity_idle_watch;
    guint activity_active_watch;
    guint idle_name_watch;
    guint dbus_owner_signal;
    guint screensaver_signal;
    guint logind_lock_signal;
    guint screensaver_registration;
    guint activity_registration;
    gboolean name_lost;
    gboolean activity_available;
    gboolean activity_idle;
    guint64 activity_idle_for_ms;
} IdleService;

static void update_activity(IdleService* service, gboolean publish);

static void publish_activity(IdleService* service, gboolean available, gboolean idle,
                             guint64 idle_for_ms) {
    gboolean changed = service->activity_available != available || service->activity_idle != idle;
    service->activity_available = available;
    service->activity_idle = available && idle;
    service->activity_idle_for_ms = available ? idle_for_ms : 0;
    if (!changed)
        return;

    g_autoptr(GError) error = NULL;
    if (!g_dbus_connection_emit_signal(
            service->session_bus, NULL, SESSION_IDLE_PATH, SESSION_IDLE_INTERFACE,
            "ActivityChanged",
            g_variant_new("(bbtt)", service->activity_available, service->activity_idle,
                          (guint64)ACTIVITY_THRESHOLD_MS, service->activity_idle_for_ms),
            &error))
        g_warning("Could not deliver session activity event: %s", error->message);
}

static GVariant* activity_state(IdleService* service) {
    return g_variant_new("(bbtt)", service->activity_available, service->activity_idle,
                         (guint64)ACTIVITY_THRESHOLD_MS, service->activity_idle_for_ms);
}

static void update_activity(IdleService* service, gboolean publish) {
    if (!service->idle_monitor) {
        if (publish)
            publish_activity(service, FALSE, FALSE, 0);
        else {
            service->activity_available = FALSE;
            service->activity_idle = FALSE;
            service->activity_idle_for_ms = 0;
        }
        return;
    }

    g_autoptr(GError) error = NULL;
    g_autoptr(GVariant) reply = g_dbus_proxy_call_sync(service->idle_monitor, "GetIdletime", NULL,
                                                       G_DBUS_CALL_FLAGS_NONE, 5000, NULL, &error);
    if (!reply) {
        g_warning("Could not read session activity: %s", error->message);
        if (publish)
            publish_activity(service, FALSE, FALSE, 0);
        return;
    }
    guint64 idle_for_ms = 0;
    g_variant_get(reply, "(t)", &idle_for_ms);
    publish_activity(service, TRUE, idle_for_ms >= ACTIVITY_THRESHOLD_MS, idle_for_ms);
}

static void replace_activity_watches(IdleService* service) {
    if (!service->idle_monitor)
        return;

    if (service->activity_idle_watch) {
        g_dbus_proxy_call_sync(service->idle_monitor, "RemoveWatch",
                               g_variant_new("(u)", service->activity_idle_watch),
                               G_DBUS_CALL_FLAGS_NONE, 5000, NULL, NULL);
        service->activity_idle_watch = 0;
    }
    if (service->activity_active_watch) {
        g_dbus_proxy_call_sync(service->idle_monitor, "RemoveWatch",
                               g_variant_new("(u)", service->activity_active_watch),
                               G_DBUS_CALL_FLAGS_NONE, 5000, NULL, NULL);
        service->activity_active_watch = 0;
    }

    g_autoptr(GError) error = NULL;
    g_autoptr(GVariant) idle_reply = g_dbus_proxy_call_sync(
        service->idle_monitor, "AddIdleWatch", g_variant_new("(t)", (guint64)ACTIVITY_THRESHOLD_MS),
        G_DBUS_CALL_FLAGS_NONE, 5000, NULL, &error);
    if (!idle_reply) {
        g_warning("Could not register session activity idle watch: %s", error->message);
        publish_activity(service, FALSE, FALSE, 0);
        return;
    }
    g_variant_get(idle_reply, "(u)", &service->activity_idle_watch);

    g_clear_error(&error);
    g_autoptr(GVariant) active_reply =
        g_dbus_proxy_call_sync(service->idle_monitor, "AddUserActiveWatch", NULL,
                               G_DBUS_CALL_FLAGS_NONE, 5000, NULL, &error);
    if (!active_reply) {
        g_warning("Could not register session activity active watch: %s", error->message);
        if (service->activity_idle_watch) {
            g_dbus_proxy_call_sync(service->idle_monitor, "RemoveWatch",
                                   g_variant_new("(u)", service->activity_idle_watch),
                                   G_DBUS_CALL_FLAGS_NONE, 5000, NULL, NULL);
            service->activity_idle_watch = 0;
        }
        publish_activity(service, FALSE, FALSE, 0);
        return;
    }
    g_variant_get(active_reply, "(u)", &service->activity_active_watch);
    update_activity(service, TRUE);
}

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

/* Gnoblin has no lock screen of its own. A lock request goes to a shell that subscribed to
 * gnoblin.session.lock-requested, through the same control request as `gnoblinctl session lock`.
 * Running gnoblinctl keeps one request path and one place that knows the control protocol. */
static char* gnoblinctl_path(void) {
    g_autofree char* self = g_file_read_link("/proc/self/exe", NULL);
    if (self) {
        g_autofree char* libexec = g_path_get_dirname(self);
        g_autofree char* candidate = g_build_filename(libexec, "..", "bin", "gnoblinctl", NULL);
        if (g_file_test(candidate, G_FILE_TEST_IS_EXECUTABLE))
            return g_steal_pointer(&candidate);
    }
    return g_strdup("gnoblinctl");
}

static gboolean run_gnoblinctl(const char* first, const char* second, char** standard_output,
                               GError** error) {
    g_autofree char* path = gnoblinctl_path();
    const char* argv[] = {path, "--timeout", "5", first, second, NULL};
    g_autofree char* output = NULL;
    g_autofree char* failure = NULL;
    gint status = 0;

    if (!g_spawn_sync(NULL, (char**)argv, NULL, G_SPAWN_SEARCH_PATH, NULL, NULL, &output, &failure,
                      &status, error))
        return FALSE;
    if (!g_spawn_check_wait_status(status, NULL)) {
        /* gnoblinctl prints "gnoblinctl: <reason>". Pass the reason on to the D-Bus caller. */
        const char* reason = failure ? g_strstrip(failure) : "";
        if (g_str_has_prefix(reason, "gnoblinctl: "))
            reason += strlen("gnoblinctl: ");
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_FAILED,
                            *reason ? reason : "gnoblinctl failed");
        return FALSE;
    }
    if (standard_output)
        *standard_output = g_steal_pointer(&output);
    return TRUE;
}

/* TRUE when the compositor holds any lock state other than unlocked, so a locker is or was in
 * charge. A second lock request then has nothing to do, as in GNOME's screen shield. */
static gboolean session_lock_active(GError** error) {
    g_autofree char* output = NULL;
    g_autoptr(JsonParser) parser = json_parser_new();

    if (!run_gnoblinctl("--json", "status", &output, error))
        return FALSE;
    if (!json_parser_load_from_data(parser, output, -1, error))
        return FALSE;
    JsonNode* root = json_parser_get_root(parser);
    if (!root || !JSON_NODE_HOLDS_OBJECT(root)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                            "gnoblinctl returned an invalid status");
        return FALSE;
    }
    const char* state = json_object_get_string_member_with_default(json_node_get_object(root),
                                                                   "lock_state", "unlocked");
    return !g_str_equal(state, "unlocked");
}

static gboolean request_session_lock(GError** error) {
    g_autoptr(GError) state_error = NULL;

    if (session_lock_active(&state_error))
        return TRUE;
    if (state_error)
        g_debug("Could not read the lock state, asking for a lock anyway: %s",
                state_error->message);
    return run_gnoblinctl("session", "lock", NULL, error);
}

static gboolean gnome_screensaver_owned(IdleService* service) {
    g_autoptr(GVariant) reply = g_dbus_connection_call_sync(
        service->session_bus, "org.freedesktop.DBus", "/org/freedesktop/DBus",
        "org.freedesktop.DBus", "NameHasOwner", g_variant_new("(s)", GNOME_SCREENSAVER_NAME),
        G_VARIANT_TYPE("(b)"), G_DBUS_CALL_FLAGS_NONE, 1000, NULL, NULL);
    gboolean owned = FALSE;

    if (reply)
        g_variant_get(reply, "(b)", &owned);
    return owned;
}

/* The graphical session of this user, as logind names it. This service runs in the user manager,
 * outside the session scope, so GetSessionByPID cannot find it. */
static char* logind_display_session_path(IdleService* service) {
    g_autoptr(GVariant) user = g_dbus_connection_call_sync(
        service->system_bus, "org.freedesktop.login1", "/org/freedesktop/login1",
        "org.freedesktop.login1.Manager", "GetUser", g_variant_new("(u)", (guint)getuid()),
        G_VARIANT_TYPE("(o)"), G_DBUS_CALL_FLAGS_NONE, 2000, NULL, NULL);
    const char* user_path = NULL;

    if (!user)
        return NULL;
    g_variant_get(user, "(&o)", &user_path);
    g_autoptr(GVariant) property = g_dbus_connection_call_sync(
        service->system_bus, "org.freedesktop.login1", user_path, "org.freedesktop.DBus.Properties",
        "Get", g_variant_new("(ss)", "org.freedesktop.login1.User", "Display"),
        G_VARIANT_TYPE("(v)"), G_DBUS_CALL_FLAGS_NONE, 2000, NULL, NULL);
    if (!property)
        return NULL;
    g_autoptr(GVariant) boxed = NULL;
    g_variant_get(property, "(v)", &boxed);
    const char* id = NULL;
    const char* path = NULL;
    g_variant_get(boxed, "(&s&o)", &id, &path);
    return g_strdup(path);
}

static void on_logind_lock(GDBusConnection* connection, const char* sender, const char* path,
                           const char* interface, const char* signal, GVariant* parameters,
                           gpointer data) {
    IdleService* service = data;
    g_autoptr(GError) error = NULL;
    g_autofree char* display = NULL;

    /* A GNOME Shell that owns the screen saver name also listens to logind. Leave the request to
     * it. */
    if (gnome_screensaver_owned(service))
        return;
    display = logind_display_session_path(service);
    if (!display || !g_str_equal(display, path))
        return;
    if (!request_session_lock(&error))
        g_warning("logind asked for a session lock, but it could not be requested: %s",
                  error->message);
}

static void on_idle_signal(GDBusProxy* proxy, const char* sender, const char* signal,
                           GVariant* parameters, gpointer data) {
    IdleService* service = data;
    guint watch;

    if (!g_str_equal(signal, "WatchFired"))
        return;
    g_variant_get(parameters, "(u)", &watch);
    if (watch == service->activity_idle_watch) {
        service->activity_idle_watch = 0;
        update_activity(service, TRUE);
    } else if (watch == service->activity_active_watch) {
        service->activity_active_watch = 0;
        replace_activity_watches(service);
    }
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
    replace_activity_watches(service);
}

static void on_idle_vanished(GDBusConnection* connection, const char* name, gpointer data) {
    IdleService* service = data;
    service->activity_idle_watch = 0;
    service->activity_active_watch = 0;
    g_clear_object(&service->idle_monitor);
    publish_activity(service, FALSE, FALSE, 0);
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
    g_hash_table_foreach_remove(service->inhibitors, remove_disconnected_inhibitor, (gpointer)name);
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

    if (g_str_equal(interface, SESSION_IDLE_INTERFACE) && g_str_equal(method, "GetActivity")) {
        update_activity(service, TRUE);
        g_dbus_method_invocation_return_value(invocation, activity_state(service));
        return;
    }

    if (g_str_equal(interface, "org.freedesktop.ScreenSaver") && g_str_equal(method, "Inhibit")) {
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

    if (g_str_equal(interface, "org.freedesktop.ScreenSaver") && g_str_equal(method, "UnInhibit")) {
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
        return;
    }

    if (g_str_equal(interface, "org.freedesktop.ScreenSaver") &&
        !gnome_screensaver_owned(service)) {
        if (g_str_equal(method, "GetActive")) {
            gboolean active = session_lock_active(&error);
            if (error) {
                g_dbus_method_invocation_return_gerror(invocation, error);
                return;
            }
            g_dbus_method_invocation_return_value(invocation, g_variant_new("(b)", active));
            return;
        }
        if (g_str_equal(method, "Lock") || g_str_equal(method, "SetActive")) {
            gboolean lock = TRUE;
            if (g_str_equal(method, "SetActive"))
                g_variant_get(parameters, "(b)", &lock);
            if (!lock) {
                g_dbus_method_invocation_return_error_literal(
                    invocation, G_DBUS_ERROR, G_DBUS_ERROR_NOT_SUPPORTED,
                    "Only the lock client can unlock the session");
                return;
            }
            if (!request_session_lock(&error)) {
                g_dbus_method_invocation_return_error_literal(invocation, G_DBUS_ERROR,
                                                              G_DBUS_ERROR_FAILED, error->message);
                return;
            }
            g_dbus_method_invocation_return_value(invocation, NULL);
            return;
        }
    }

    if (g_str_equal(interface, "org.freedesktop.ScreenSaver") && g_str_equal(method, "GetActive")) {
        reply = call_screen_saver(service, "GetActive", NULL, G_VARIANT_TYPE("(b)"), &error);
    } else if (g_str_equal(interface, "org.freedesktop.ScreenSaver") &&
               g_str_equal(method, "SetActive")) {
        reply = call_screen_saver(service, "SetActive", parameters, NULL, &error);
    } else if (g_str_equal(interface, "org.freedesktop.ScreenSaver") &&
               g_str_equal(method, "Lock")) {
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
    service.screensaver_registration = g_dbus_connection_register_object(
        service.session_bus, SCREENSAVER_PATH, node->interfaces[0], &interface_vtable, &service,
        NULL, &error);
    if (!service.screensaver_registration)
        g_error("Could not export screen saver interface: %s", error->message);
    service.activity_registration = g_dbus_connection_register_object(
        service.session_bus, SESSION_IDLE_PATH, node->interfaces[1], &interface_vtable, &service,
        NULL, &error);
    if (!service.activity_registration)
        g_error("Could not export session activity interface: %s", error->message);

    service.inhibitors = g_hash_table_new_full(g_direct_hash, g_direct_equal, NULL, inhibitor_free);
    service.dbus_owner_signal = g_dbus_connection_signal_subscribe(
        service.session_bus, "org.freedesktop.DBus", "org.freedesktop.DBus", "NameOwnerChanged",
        "/org/freedesktop/DBus", NULL, G_DBUS_SIGNAL_FLAGS_NONE, on_name_owner_changed, &service,
        NULL);
    service.screensaver_signal = g_dbus_connection_signal_subscribe(
        service.session_bus, GNOME_SCREENSAVER_NAME, GNOME_SCREENSAVER_NAME, "ActiveChanged",
        GNOME_SCREENSAVER_PATH, NULL, G_DBUS_SIGNAL_FLAGS_NONE, on_screen_saver_active_changed,
        &service, NULL);
    service.logind_lock_signal = g_dbus_connection_signal_subscribe(
        service.system_bus, "org.freedesktop.login1", "org.freedesktop.login1.Session", "Lock",
        NULL, NULL, G_DBUS_SIGNAL_FLAGS_NONE, on_logind_lock, &service, NULL);
    service.loop = g_main_loop_new(NULL, FALSE);
    name_id = g_bus_own_name_on_connection(service.session_bus, SCREENSAVER_NAME,
                                           G_BUS_NAME_OWNER_FLAGS_NONE, on_name_acquired,
                                           on_name_lost, &service, NULL);
    g_main_loop_run(service.loop);
    g_bus_unown_name(name_id);
    if (service.idle_name_watch)
        g_bus_unwatch_name(service.idle_name_watch);
    if (service.screensaver_registration)
        g_dbus_connection_unregister_object(service.session_bus, service.screensaver_registration);
    if (service.activity_registration)
        g_dbus_connection_unregister_object(service.session_bus, service.activity_registration);
    g_dbus_connection_signal_unsubscribe(service.session_bus, service.dbus_owner_signal);
    g_dbus_connection_signal_unsubscribe(service.session_bus, service.screensaver_signal);
    g_dbus_connection_signal_unsubscribe(service.system_bus, service.logind_lock_signal);
    g_hash_table_unref(service.inhibitors);
    g_clear_object(&service.idle_monitor);
    g_clear_object(&service.system_bus);
    g_clear_object(&service.session_bus);
    g_main_loop_unref(service.loop);
    return service.name_lost ? 1 : 0;
}
