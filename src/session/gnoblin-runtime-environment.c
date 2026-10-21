#include "gnoblin-runtime-internal.h"

gboolean restart_active_portal_service(GError** error) {
    g_autoptr(GDBusConnection) bus = g_bus_get_sync(G_BUS_TYPE_SESSION, NULL, error);
    if (!bus)
        return FALSE;
    g_autoptr(GVariant) owner_reply = g_dbus_connection_call_sync(
        bus, "org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus",
        "GetNameOwner", g_variant_new("(s)", "org.freedesktop.portal.Desktop"),
        G_VARIANT_TYPE("(s)"), G_DBUS_CALL_FLAGS_NONE, 1000, NULL, error);
    if (!owner_reply) {
        if (error && *error &&
            g_error_matches(*error, G_DBUS_ERROR, G_DBUS_ERROR_NAME_HAS_NO_OWNER)) {
            g_clear_error(error);
            return TRUE;
        }
        return FALSE;
    }
    const char* owner = NULL;
    g_variant_get(owner_reply, "(&s)", &owner);
    g_autoptr(GVariant) pid_reply = g_dbus_connection_call_sync(
        bus, "org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus",
        "GetConnectionUnixProcessID", g_variant_new("(s)", owner), G_VARIANT_TYPE("(u)"),
        G_DBUS_CALL_FLAGS_NONE, 1000, NULL, error);
    if (!pid_reply)
        return FALSE;
    guint32 pid = 0;
    g_variant_get(pid_reply, "(u)", &pid);
    if (pid == 0 || pid == (guint32)getpid()) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                            "portal service returned an invalid process ID");
        return FALSE;
    }
    if (kill((pid_t)pid, SIGTERM) != 0 && errno != ESRCH) {
        g_set_error(error, G_IO_ERROR, g_io_error_from_errno(errno),
                    "could not stop the active portal service: %s", g_strerror(errno));
        return FALSE;
    }
    g_message("gnoblin: restarted the portal service after its routing configuration changed");
    return TRUE;
}

GVariant* startup_fallback_document(const char* path, const char* failure,
                                    gboolean* used_last_good) {
    if (used_last_good)
        *used_last_good = FALSE;
    g_warning("gnoblin: could not load %s: %s", path,
              failure ? failure : "configuration is invalid");

    g_autoptr(GError) cache_error = NULL;
    GVariant* saved = gnoblin_config_load_last_good_document(path, &cache_error);
    if (saved) {
        if (used_last_good)
            *used_last_good = TRUE;
        g_warning("gnoblin: starting with the last accepted configuration; fix the user config "
                  "and reload it to restore changes");
        return saved;
    }
    if (cache_error && !g_error_matches(cache_error, G_FILE_ERROR, G_FILE_ERROR_NOENT))
        g_warning("gnoblin: could not use the saved configuration: %s", cache_error->message);
    g_warning("gnoblin: no accepted configuration is available; starting with Gnoblin defaults");
    return gnoblin_config_default_document();
}

static char* session_prefix;

static gboolean session_systemd_available;

GDBusConnection* claim_session_lifecycle_bus(void) {
    g_autoptr(GError) error = NULL;
    g_autoptr(GDBusConnection) connection = g_bus_get_sync(G_BUS_TYPE_SESSION, NULL, &error);
    if (!connection) {
        g_debug("gnoblin: portal end-session notifications unavailable: %s", error->message);
        return NULL;
    }

    g_autoptr(GVariant) reply = g_dbus_connection_call_sync(
        connection, "org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus",
        "RequestName", g_variant_new("(su)", SESSION_SUPERVISOR_BUS_NAME, 4u),
        G_VARIANT_TYPE("(u)"), G_DBUS_CALL_FLAGS_NONE, 2000, NULL, &error);
    if (!reply) {
        g_debug("gnoblin: portal end-session notifications unavailable: %s", error->message);
        return NULL;
    }

    guint32 result = 0;
    g_variant_get(reply, "(u)", &result);
    if (result != 1) {
        g_debug("gnoblin: another session owns %s", SESSION_SUPERVISOR_BUS_NAME);
        return NULL;
    }

    return g_steal_pointer(&connection);
}

void notify_portal_session_ending(GDBusConnection* connection) {
    if (!connection)
        return;

    g_autoptr(GError) error = NULL;
    g_autoptr(GVariant) reply = g_dbus_connection_call_sync(
        connection, PORTAL_BACKEND_BUS_NAME, "/org/freedesktop/portal/desktop",
        PORTAL_LIFECYCLE_INTERFACE, "PrepareForEnd", NULL, G_VARIANT_TYPE("()"),
        G_DBUS_CALL_FLAGS_NONE, 2000, NULL, &error);
    if (!reply)
        g_debug("gnoblin: portal end-session notification was not completed: %s", error->message);
}

static gboolean run_command(const char* const argv[], gboolean required) {
    g_autoptr(GError) error = NULL;
    gint status = 0;
    if (!g_spawn_sync(NULL, (char**)argv, NULL, G_SPAWN_SEARCH_PATH, NULL, NULL, NULL, NULL,
                      &status, &error)) {
        if (required)
            g_printerr("gnoblin: could not run %s: %s\n", argv[0], error->message);
        return FALSE;
    }
    gboolean ok = WIFEXITED(status) && WEXITSTATUS(status) == 0;
    if (!ok && required)
        g_printerr("gnoblin: %s failed\n", argv[0]);
    return ok;
}

static gboolean run_activation_update(const char* const extra[]) {
    const char* argv[32];
    guint n = 0;
    argv[n++] = "dbus-update-activation-environment";
    if (session_systemd_available)
        argv[n++] = "--systemd";
    for (guint i = 0; extra[i] && n < G_N_ELEMENTS(argv) - 1; i++)
        argv[n++] = extra[i];
    argv[n] = NULL;
    if (!run_command(argv, FALSE))
        g_warning("gnoblin: D-Bus activation environment could not be updated; continuing session "
                  "startup");
    return TRUE;
}

static gboolean user_systemd_owner_once(void) {
    g_autoptr(GError) error = NULL;
    g_autoptr(GDBusConnection) bus = g_bus_get_sync(G_BUS_TYPE_SESSION, NULL, &error);
    if (!bus)
        return FALSE;
    g_autoptr(GVariant) owner = g_dbus_connection_call_sync(
        bus, "org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus",
        "GetNameOwner", g_variant_new("(s)", "org.freedesktop.systemd1"), G_VARIANT_TYPE("(s)"),
        G_DBUS_CALL_FLAGS_NONE, 1000, NULL, NULL);
    return owner != NULL;
}

/* On a cold first login the user manager and its dbus-broker start in parallel with this process,
 * so the session bus or the systemd1 name can show up a moment after we run. A single check lost
 * that race and left the session without gnoblin-session.target. When the system boots with
 * systemd, retry for up to 10 s; on other init systems check once so startup is not delayed. */
static gboolean user_systemd_available(void) {
    const guint attempts = g_file_test("/run/systemd/system", G_FILE_TEST_IS_DIR) ? 50 : 1;
    for (guint i = 0; i < attempts; i++) {
        if (user_systemd_owner_once())
            return TRUE;
        if (i + 1 < attempts)
            g_usleep(200 * 1000);
    }
    return FALSE;
}

/* D-Bus-activated services of the user manager that outlive the session. dbus-broker starts each as
 * a transient unit with no link to the session, and the service waits for a GNOME session manager
 * to end it. Gnoblin has none, so without this each login would leave one more process running in a
 * lingering user manager. */
static const char* const stale_service_patterns[] = {
    "dbus-*-org.gnome.OnlineAccounts@*.service",
    "dbus-*-org.gnome.Identity@*.service",
    "dbus-*-org.a11y.atspi.Registry@*.service",
    NULL,
};

static void stop_stale_dbus_services(void) {
    if (!session_systemd_available)
        return;
    g_autoptr(GPtrArray) list = g_ptr_array_new();
    const char* const list_head[] = {"systemctl", "--user",      "list-units", "--all",
                                     "--plain",   "--no-legend", NULL};
    for (guint i = 0; list_head[i]; i++)
        g_ptr_array_add(list, (gpointer)list_head[i]);
    for (guint i = 0; stale_service_patterns[i]; i++)
        g_ptr_array_add(list, (gpointer)stale_service_patterns[i]);
    g_ptr_array_add(list, NULL);
    gchar* output = NULL;
    gint status = 0;
    if (!g_spawn_sync(NULL, (char**)list->pdata, NULL, G_SPAWN_SEARCH_PATH, NULL, NULL, &output,
                      NULL, &status, NULL) ||
        !WIFEXITED(status) || WEXITSTATUS(status) != 0) {
        g_free(output);
        return;
    }
    g_auto(GStrv) lines = g_strsplit(output, "\n", -1);
    g_free(output);
    for (guint i = 0; lines[i]; i++) {
        g_auto(GStrv) fields = g_strsplit_set(lines[i], " \t", 0);
        const char* unit = fields[0];
        if (!unit || !*unit)
            continue;
        gboolean stale = FALSE;
        for (guint j = 0; stale_service_patterns[j]; j++)
            stale = stale || g_pattern_match_simple(stale_service_patterns[j], unit);
        if (!stale)
            continue;
        const char* stop[] = {"systemctl", "--user", "stop", unit, NULL};
        run_command(stop, TRUE);
    }
}

/* Desktop identity as the lingering user manager held it before this session.
 * GNOME leaves these set after logout, and its services read them while the
 * next session starts, before that session publishes its own. Put the old
 * values back at logout, so a GNOME session that follows finds what it would
 * have found had Gnoblin never run. Display variables are not restored. */
static const char* const identity_variables[] = {"XDG_CURRENT_DESKTOP", "XDG_SESSION_DESKTOP",
                                                 "DESKTOP_SESSION", NULL};

static char* saved_identity[G_N_ELEMENTS(identity_variables)];

static void save_identity_environment(void) {
    if (!session_systemd_available)
        return;
    const char* argv[] = {"systemctl", "--user", "show-environment", NULL};
    gchar* output = NULL;
    gint status = 0;
    if (!g_spawn_sync(NULL, (char**)argv, NULL, G_SPAWN_SEARCH_PATH, NULL, NULL, &output, NULL,
                      &status, NULL) ||
        !WIFEXITED(status) || WEXITSTATUS(status) != 0) {
        g_free(output);
        return;
    }
    g_auto(GStrv) lines = g_strsplit(output, "\n", -1);
    g_free(output);
    for (guint i = 0; lines[i]; i++) {
        for (guint j = 0; identity_variables[j]; j++) {
            const gsize length = strlen(identity_variables[j]);
            if (!g_str_has_prefix(lines[i], identity_variables[j]) || lines[i][length] != '=')
                continue;
            const char* value = lines[i] + length + 1;
            /* Keep simple values only, and never our own from a crashed session. */
            if (!*value || strpbrk(value, "'\"\\$` \t") ||
                g_ascii_strcasecmp(value, "gnoblin") == 0)
                continue;
            g_free(saved_identity[j]);
            saved_identity[j] = g_strdup(lines[i]);
        }
    }
}

static void restore_identity_environment(void) {
    const char* restore[G_N_ELEMENTS(identity_variables)] = {NULL};
    guint count = 0;
    for (guint j = 0; identity_variables[j]; j++)
        if (saved_identity[j])
            restore[count++] = saved_identity[j];
    if (count)
        run_activation_update(restore);
}

static GnoblinSessionClass active_session_class = GNOBLIN_SESSION_CLASS_USER;

static void session_cleanup(void) {
    const char* stop[] = {
        "systemctl", "--user", "stop", "gnoblin-session.target", "graphical-session.target", NULL};
    const char* clear[] = {"systemctl",
                           "--user",
                           "unset-environment",
                           "GNOME_SHELL_SESSION_MODE",
                           "XDG_CURRENT_DESKTOP",
                           "XDG_SESSION_DESKTOP",
                           "DESKTOP_SESSION",
                           "XDG_SESSION_CLASS",
                           "XDG_SESSION_TYPE",
                           "WAYLAND_DISPLAY",
                           "DISPLAY",
                           "XAUTHORITY",
                           "GNOME_SETUP_DISPLAY",
                           NULL};
    const char* unset[] = {"GNOME_SHELL_SESSION_MODE=",
                           "XDG_CURRENT_DESKTOP=",
                           "XDG_SESSION_DESKTOP=",
                           "DESKTOP_SESSION=",
                           "XDG_SESSION_CLASS=",
                           "XDG_SESSION_TYPE=",
                           "WAYLAND_DISPLAY=",
                           "DISPLAY=",
                           "XAUTHORITY=",
                           "GNOME_SETUP_DISPLAY=",
                           NULL};
    /* A greeter never owned graphical-session.target, so it must not stop it. */
    if (session_systemd_available && active_session_class == GNOBLIN_SESSION_CLASS_USER)
        run_command(stop, FALSE);
    stop_stale_dbus_services();
    run_activation_update(unset);
    if (session_systemd_available)
        run_command(clear, FALSE);
    restore_identity_environment();
    g_clear_pointer(&session_prefix, g_free);
}

char* gnoblin_installed_prefix(void) {
    g_autofree char* exe = g_file_read_link("/proc/self/exe", NULL);
    if (!exe)
        return NULL;
    g_autofree char* bin_dir = g_path_get_dirname(exe);
    return g_path_get_dirname(bin_dir);
}

gboolean prepare_session_environment(GnoblinSessionClass session_class) {
    active_session_class = session_class;
    session_prefix = gnoblin_installed_prefix();
    if (!session_prefix) {
        g_printerr("gnoblin: could not locate installed executable\n");
        return FALSE;
    }
    /* greetd starts the greeter without XDG_SESSION_TYPE; the greeter is a Wayland
     * compositor by definition, so supply the value the checks below require. */
    if (session_class == GNOBLIN_SESSION_CLASS_GREETER && !g_getenv("XDG_SESSION_TYPE"))
        g_setenv("XDG_SESSION_TYPE", "wayland", TRUE);
    if (!g_str_equal(g_getenv("XDG_SESSION_TYPE") ?: "", "wayland") ||
        !g_getenv("XDG_SESSION_ID") || !*g_getenv("XDG_SESSION_ID") ||
        (g_getenv("WAYLAND_DISPLAY") && *g_getenv("WAYLAND_DISPLAY")) ||
        (g_getenv("DISPLAY") && *g_getenv("DISPLAY"))) {
        g_printerr("gnoblin: start it from a fresh Wayland login session\n");
        return FALSE;
    }
    session_systemd_available = user_systemd_available();
    if (session_systemd_available && session_class == GNOBLIN_SESSION_CLASS_USER) {
        const char* active[] = {
            "systemctl", "--user", "is-active", "--quiet", "graphical-session.target", NULL};
        if (run_command(active, FALSE)) {
            g_printerr("gnoblin: another graphical session is already active\n");
            return FALSE;
        }
    }
    g_unsetenv("XAUTHORITY");
    g_unsetenv("GNOME_SETUP_DISPLAY");
    g_unsetenv("GSETTINGS_SCHEMA_DIR");
    g_unsetenv("GI_TYPELIB_PATH");
    g_setenv("GNOBLIN_PREFIX", session_prefix, TRUE);
    g_autofree char* libdir_file = g_build_filename(session_prefix, "libexec/gnoblin-libdir", NULL);
    g_autofree char* libdir = NULL;
    g_file_get_contents(libdir_file, &libdir, NULL, NULL);
    if (libdir)
        g_strchomp(libdir);
    if (!libdir || !*libdir)
        libdir = g_strdup("lib64");
    g_setenv("GNOBLIN_LIBDIR", libdir, TRUE);
    const char* old_ld = g_getenv("LD_LIBRARY_PATH");
    g_autofree char* private_libs = g_build_filename(session_prefix, libdir, NULL);
    const char* api = g_getenv("GNOBLIN_MUTTER_API");
    g_autofree char* mutter_dir = g_strdup_printf("mutter-%s", api && *api ? api : "51");
    g_autofree char* mutter_libs = g_build_filename(private_libs, mutter_dir, NULL);
    g_autofree char* cxx_file = g_build_filename(session_prefix, "libexec/gnoblin-cxx-lib", NULL);
    g_autofree char* cxx_lib = NULL;
    g_file_get_contents(cxx_file, &cxx_lib, NULL, NULL);
    if (cxx_lib)
        g_strchomp(cxx_lib);
    g_autofree char* ld = cxx_lib && *cxx_lib
                              ? g_strdup_printf("%s:%s:%s", private_libs, mutter_libs, cxx_lib)
                              : g_strdup_printf("%s:%s%s%s", private_libs, mutter_libs,
                                                old_ld && *old_ld ? ":" : "", old_ld ?: "");
    g_setenv("LD_LIBRARY_PATH", ld, TRUE);
    g_autofree char* path =
        g_strdup_printf("%s/bin:%s", session_prefix, g_getenv("PATH") ?: "/usr/bin:/bin");
    g_setenv("PATH", path, TRUE);
    g_autofree char* data_dirs = g_strdup_printf(
        "%s/share:%s", session_prefix, g_getenv("XDG_DATA_DIRS") ?: "/usr/local/share:/usr/share");
    g_setenv("XDG_DATA_DIRS", data_dirs, TRUE);
    g_setenv("XDG_CURRENT_DESKTOP", "Gnoblin", TRUE);
    g_setenv("XDG_SESSION_DESKTOP", "gnoblin", TRUE);
    const char* session_id = g_getenv("XDG_SESSION_ID");
    if (session_id) {
        const char* loginctl[] = {"loginctl",         "show-session", session_id,
                                  "--property=Class", "--value",      NULL};
        gchar* out = NULL;
        gint status = 0;
        if (g_spawn_sync(NULL, (char**)loginctl, NULL, G_SPAWN_SEARCH_PATH, NULL, NULL, &out, NULL,
                         &status, NULL) &&
            WIFEXITED(status) && WEXITSTATUS(status) == 0 && out && *g_strstrip(out))
            g_setenv("XDG_SESSION_CLASS", g_strstrip(out), TRUE);
        g_free(out);
    }
    return TRUE;
}

gboolean activate_session(GnoblinSessionClass session_class) {
    atexit(session_cleanup);
    const char* clear_display[] = {
        "WAYLAND_DISPLAY=", "DISPLAY=", "XAUTHORITY=", "GNOME_SETUP_DISPLAY=", NULL};
    save_identity_environment();
    run_activation_update(clear_display);
    /* One call clears the display variables and the previous session's mode. */
    const char* unset_stale[] = {
        "systemctl", "--user",     "unset-environment",   "WAYLAND_DISPLAY",
        "DISPLAY",   "XAUTHORITY", "GNOME_SETUP_DISPLAY", "GNOME_SHELL_SESSION_MODE",
        NULL};
    if (session_systemd_available)
        run_command(unset_stale, FALSE);
    const char* sync[] = {"GNOME_SHELL_SESSION_MODE",
                          "XDG_CONFIG_HOME",
                          "XDG_CURRENT_DESKTOP",
                          "XDG_SESSION_DESKTOP",
                          "DESKTOP_SESSION",
                          "XDG_SESSION_CLASS",
                          "XDG_SESSION_TYPE",
                          "WAYLAND_DISPLAY",
                          "DISPLAY",
                          "XAUTHORITY",
                          NULL};
    g_unsetenv("GNOME_SHELL_SESSION_MODE");
    run_activation_update(sync);
    if (session_systemd_available && session_class == GNOBLIN_SESSION_CLASS_USER) {
        /* A lingering user manager keeps the environment and some services of
         * the previous session, for example GNOME's. The portal frontend reads
         * XDG_CURRENT_DESKTOP once at start and would keep choosing GNOME's
         * backends here, so restart it now that the environment is correct.
         * D-Bus starts it again on demand if it was not running. */
        const char* restart_portal[] = {"systemctl", "--user", "try-restart",
                                        "xdg-desktop-portal.service", NULL};
        run_command(restart_portal, FALSE);
    }
    stop_stale_dbus_services();
    if (session_systemd_available && session_class == GNOBLIN_SESSION_CLASS_USER) {
        const char* stop_old[] = {"systemctl", "--user", "stop",
                                  "org.gnome.SettingsDaemon.ScreensaverProxy.service", NULL};
        run_command(stop_old, FALSE);
        const char* clear_private[] = {"systemctl",       "--user",          "unset-environment",
                                       "GNOBLIN_PREFIX",  "GNOBLIN_LIBDIR",  "GSETTINGS_SCHEMA_DIR",
                                       "LD_LIBRARY_PATH", "GI_TYPELIB_PATH", NULL};
        run_command(clear_private, FALSE);
        const char* reload[] = {"systemctl", "--user", "daemon-reload", NULL};
        /* Integrate with graphical-session.target when systemd is available;
         * the compositor runtime and Lua autostart do not depend on this path.
         * Installing the units already reloads the manager, so a reload on every
         * login (about 0.3 s of manager work here) only happens if the target
         * cannot be started without it. */
        const char* start[] = {"systemctl", "--user", "start", "gnoblin-session.target", NULL};
        if (!run_command(start, FALSE) &&
            (!run_command(reload, FALSE) || !run_command(start, FALSE)))
            g_warning("gnoblin: systemd user-session integration could not be activated");
    }
    return TRUE;
}
