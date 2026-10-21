/* Exercise the production environment handoff without spawning any commands
 * or connecting to a session bus. Unused runtime functions are discarded at
 * link time, so this needs only GLib/GIO, not a built compositor. */
#include "../src/session/gnoblin-runtime-internal.h"

static gboolean fixture_spawn_sync(const gchar* working_directory, gchar** argv, gchar** envp,
                                   GSpawnFlags flags, GSpawnChildSetupFunc child_setup,
                                   gpointer user_data, gchar** standard_output,
                                   gchar** standard_error, gint* wait_status, GError** error) {
    const char* mode = g_getenv("GNOME_SHELL_SESSION_MODE");
    g_print("%s", mode ? mode : "<unset>");
    for (guint i = 0; argv[i]; i++)
        g_print("\t%s", argv[i]);
    g_print("\n");

    if (g_str_equal(argv[0], "fixture-spawn-failure")) {
        g_set_error_literal(error, G_SPAWN_ERROR, G_SPAWN_ERROR_NOENT, "fixture missing command");
        return FALSE;
    }
    *wait_status = g_str_equal(argv[0], "fixture-exit-failure")     ? 1 << 8
                   : g_str_equal(argv[0], "fixture-signal-failure") ? SIGTERM
                                                                    : 0;
    if (standard_output) {
        if (g_str_equal(argv[0], "systemctl") && g_str_equal(argv[2], "list-units"))
            *standard_output =
                g_strdup("dbus-1-org.gnome.OnlineAccounts@1.service loaded active running GOA\n"
                         "dbus-2-org.gnome.Identity@2.service loaded active running Identity\n"
                         "dbus-3-org.a11y.atspi.Registry@3.service loaded active running Registry\n"
                         "gnome-keyring-daemon.service loaded active running Keyring\n"
                         "dbus-4-org.freedesktop.secrets@4.service loaded active running Secrets\n"
                         "unrelated.service loaded active running Other\n");
        else
            *standard_output = g_strdup("");
    }
    return TRUE;
}

static GDBusConnection* fixture_bus_get_sync(GBusType bus_type, GCancellable* cancellable,
                                             GError** error) {
    g_printerr("the environment fixture must not connect to a bus\n");
    exit(EXIT_FAILURE);
}

#define g_spawn_sync fixture_spawn_sync
#define g_bus_get_sync fixture_bus_get_sync
#include "../src/session/gnoblin-runtime-environment.c"
#undef g_bus_get_sync
#undef g_spawn_sync

int main(int argc, char** argv) {
    if (argc != 2)
        return EXIT_FAILURE;
    if (g_str_has_prefix(argv[1], "fixture-")) {
        const char* command[] = {argv[1], NULL};
        /* Optional commands still report failure to callers that need a fallback. */
        if (run_command(command, FALSE) || run_command(command, TRUE)) {
            g_printerr("a failed command was reported as successful\n");
            return EXIT_FAILURE;
        }
        return 0;
    }

    session_systemd_available = g_str_equal(argv[1], "systemd");
    g_setenv("GNOME_SHELL_SESSION_MODE", "gnome", TRUE);
    g_setenv("XDG_CURRENT_DESKTOP", "Gnoblin", TRUE);
    g_setenv("XDG_SESSION_DESKTOP", "gnoblin", TRUE);
    g_setenv("XDG_SESSION_TYPE", "wayland", TRUE);
    g_unsetenv("WAYLAND_DISPLAY");
    g_unsetenv("DISPLAY");
    g_unsetenv("XAUTHORITY");
    g_unsetenv("GNOME_SETUP_DISPLAY");
    if (!activate_session(GNOBLIN_SESSION_CLASS_USER) || g_getenv("GNOME_SHELL_SESSION_MODE")) {
        g_printerr("session activation failed or retained the previous session mode\n");
        return EXIT_FAILURE;
    }
    /* Separate activation from the real atexit cleanup, which uses the same
     * intercepted command boundary. */
    g_print("ACTIVATED\n");
    return 0;
}
