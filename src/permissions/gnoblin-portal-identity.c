/* SPDX-License-Identifier: LGPL-2.1-or-later */
#include "gnoblin-portal-identity.h"
#include <unistd.h>

#define PORTAL_DESKTOP_NAME "org.freedesktop.portal.Desktop"
#define REQUEST_PATH_PREFIX "/org/freedesktop/portal/desktop/request/"

static GVariant* call_bus_method(GDBusConnection* connection, const char* method,
                                 GVariant* parameters, const GVariantType* reply_type,
                                 GError** error) {
    return g_dbus_connection_call_sync(connection, "org.freedesktop.DBus", "/org/freedesktop/DBus",
                                       "org.freedesktop.DBus", method, parameters, reply_type,
                                       G_DBUS_CALL_FLAGS_NONE, 2000, NULL, error);
}

static char* portal_frontend_owner(GDBusConnection* connection) {
    g_autoptr(GError) error = NULL;
    g_autoptr(GVariant) reply = NULL;
    const char* owner;

    reply = call_bus_method(connection, "GetNameOwner", g_variant_new("(s)", PORTAL_DESKTOP_NAME),
                            G_VARIANT_TYPE("(s)"), &error);
    if (!reply) {
        g_debug("gnoblin: cannot resolve the desktop portal owner: %s", error->message);
        return NULL;
    }

    g_variant_get(reply, "(&s)", &owner);
    return g_strdup(owner);
}

static char* request_sender_from_handle(const char* request_handle) {
    const char* component;
    const char* component_end;
    g_autofree char* decoded = NULL;
    char* separator;

    if (!request_handle || !g_str_has_prefix(request_handle, REQUEST_PATH_PREFIX))
        return NULL;

    component = request_handle + strlen(REQUEST_PATH_PREFIX);
    component_end = strchr(component, '/');
    if (!component_end || component_end == component || component_end[1] == '\0')
        return NULL;

    decoded = g_strndup(component, component_end - component);
    separator = strchr(decoded, '_');
    if (!separator || strchr(separator + 1, '_'))
        return NULL;

    for (const char* p = decoded; *p; p++) {
        if (p == separator)
            continue;
        if (!g_ascii_isdigit(*p))
            return NULL;
    }

    if (separator == decoded || separator[1] == '\0')
        return NULL;

    *separator = '.';
    return g_strconcat(":", decoded, NULL);
}

static gboolean connection_matches_user(GDBusConnection* connection, const char* sender) {
    g_autoptr(GError) error = NULL;
    g_autoptr(GVariant) reply = NULL;
    guint32 uid;

    reply = call_bus_method(connection, "GetConnectionUnixUser", g_variant_new("(s)", sender),
                            G_VARIANT_TYPE("(u)"), &error);
    if (!reply) {
        g_debug("gnoblin: cannot resolve portal requester %s: %s", sender, error->message);
        return FALSE;
    }

    g_variant_get(reply, "(u)", &uid);
    return uid == getuid();
}

static char* host_executable_identity(GDBusConnection* connection, const char* sender) {
    g_autoptr(GError) error = NULL;
    g_autoptr(GVariant) reply = NULL;
    g_autofree char* proc_link = NULL;
    g_autofree char* exe_path = NULL;
    g_autofree char* canonical_path = NULL;
    guint32 pid;

    reply = call_bus_method(connection, "GetConnectionUnixProcessID", g_variant_new("(s)", sender),
                            G_VARIANT_TYPE("(u)"), &error);
    if (!reply) {
        g_debug("gnoblin: cannot resolve host portal requester %s: %s", sender, error->message);
        return NULL;
    }

    g_variant_get(reply, "(u)", &pid);
    if (pid == 0)
        return NULL;

    proc_link = g_strdup_printf("/proc/%u/exe", pid);
    exe_path = g_file_read_link(proc_link, &error);
    if (!exe_path) {
        g_debug("gnoblin: cannot read host portal requester executable: %s", error->message);
        return NULL;
    }

    if (!g_path_is_absolute(exe_path))
        return NULL;

    canonical_path = g_canonicalize_filename(exe_path, NULL);
    if (!g_file_test(canonical_path, G_FILE_TEST_IS_REGULAR))
        return NULL;

    return g_strconcat("host-exe:", canonical_path, NULL);
}

char* gnoblin_portal_requester_identity(GDBusConnection* connection, const char* backend_sender,
                                        const char* app_id, const char* request_handle) {
    g_autofree char* frontend_owner = NULL;
    g_autofree char* request_sender = NULL;

    g_return_val_if_fail(G_IS_DBUS_CONNECTION(connection), NULL);

    frontend_owner = portal_frontend_owner(connection);
    if (!frontend_owner || g_strcmp0(frontend_owner, backend_sender) != 0) {
        g_debug("gnoblin: refusing permission identity from a direct backend caller");
        return NULL;
    }

    request_sender = request_sender_from_handle(request_handle);
    if (!request_sender || !connection_matches_user(connection, request_sender)) {
        g_debug("gnoblin: refusing permission identity with an invalid or stale request handle");
        return NULL;
    }

    if (app_id && *app_id) {
        if (!g_utf8_validate(app_id, -1, NULL))
            return NULL;
        return g_strconcat("app-id:", app_id, NULL);
    }

    return host_executable_identity(connection, request_sender);
}

const char* gnoblin_portal_identity_name(const char* identity) {
    const char* separator;

    if (!identity)
        return NULL;

    separator = strchr(identity, ':');
    return separator ? separator + 1 : NULL;
}
