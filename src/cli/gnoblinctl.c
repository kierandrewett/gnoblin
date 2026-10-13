#include "gnoblinctl-internal.h"

/* Suspend, hibernate, restart and power off go to systemd-logind, which the session user may
 * already call. They do not involve the compositor, so they work from a terminal, a shell or a
 * keybinding. */
static gboolean run_power_action(const char* action, GError** error) {
    static const struct {
        const char* action;
        const char* method;
        const char* query;
        const char* label;
    } actions[] = {
        {"suspend", "Suspend", "CanSuspend", "suspend"},
        {"hibernate", "Hibernate", "CanHibernate", "hibernate"},
        {"reboot", "Reboot", "CanReboot", "restart"},
        {"poweroff", "PowerOff", "CanPowerOff", "power off"},
    };
    for (guint i = 0; i < G_N_ELEMENTS(actions); i++) {
        if (!g_str_equal(actions[i].action, action))
            continue;
        g_autoptr(GDBusConnection) bus = g_bus_get_sync(G_BUS_TYPE_SYSTEM, NULL, error);
        if (!bus)
            return FALSE;
        g_autoptr(GVariant) answer = g_dbus_connection_call_sync(
            bus, "org.freedesktop.login1", "/org/freedesktop/login1",
            "org.freedesktop.login1.Manager", actions[i].query, NULL, G_VARIANT_TYPE("(s)"),
            G_DBUS_CALL_FLAGS_NONE, 5000, NULL, error);
        if (!answer)
            return FALSE;
        const char* verdict = NULL;
        g_variant_get(answer, "(&s)", &verdict);
        if (!g_str_equal(verdict, "yes") && !g_str_equal(verdict, "challenge")) {
            g_set_error(error, G_IO_ERROR, G_IO_ERROR_NOT_SUPPORTED,
                        "this machine cannot %s here (logind answers \"%s\")", actions[i].label,
                        verdict);
            return FALSE;
        }
        g_autoptr(GVariant) result = g_dbus_connection_call_sync(
            bus, "org.freedesktop.login1", "/org/freedesktop/login1",
            "org.freedesktop.login1.Manager", actions[i].method, g_variant_new("(b)", TRUE), NULL,
            G_DBUS_CALL_FLAGS_NONE, 30000, NULL, error);
        if (!result && error && *error)
            g_dbus_error_strip_remote_error(*error);
        return result != NULL;
    }
    g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT, "unknown power action %s", action);
    return FALSE;
}

int main(int argc, char** argv) {
    signal(SIGPIPE, SIG_IGN);
    Cli cli = {0};
    g_autoptr(GError) error = NULL;
    g_autoptr(JsonNode) result = NULL;
    if (!parse_cli(&cli, argc, argv, &error))
        goto failure;
    if (cli.version) {
        print_version(cli.format);
        return 0;
    }
    if (cli.help || !cli.command ||
        (find_command(cli.command) && find_command(cli.command)->actions && !cli.action &&
         !g_str_equal(cli.command, "privacy"))) {
        print_help(cli.command, cli.action);
        return 0;
    }
    if (!validate_cli(&cli, &error))
        goto failure;
    if (g_str_equal(cli.command, "completion")) {
        if (arg_count(&cli) != 1 || !word_in("bash zsh fish", arg(&cli, 0))) {
            g_set_error_literal(&error, G_OPTION_ERROR, G_OPTION_ERROR_BAD_VALUE,
                                "completion needs bash, zsh, or fish");
            goto failure;
        }
        print_completion(arg(&cli, 0));
        return 0;
    }
    if (g_str_equal(cli.command, "init")) {
        if (arg_count(&cli) != 0) {
            g_printerr("gnoblinctl init accepts no arguments\n");
            return 1;
        }
        if (!initialize_config(&error)) {
            g_printerr("gnoblinctl: %s\n", error->message);
            return 1;
        }
        return 0;
    }
    if (g_str_equal(cli.command, "config") && g_str_equal(cli.action, "restore-default")) {
        if (arg_count(&cli) != 0) {
            g_printerr("gnoblinctl config restore-default accepts no arguments\n");
            return 1;
        }
        if (!restore_default_config(&error)) {
            g_printerr("gnoblinctl: %s\n", error->message);
            return 1;
        }
        g_clear_error(&error);
        g_autoptr(JsonNode) reload =
            call_compositor(&cli, "api", "runtime.reload_config", new_object(), &error);
        if (reload)
            g_print("Reloaded the restored configuration.\n");
        else if (error && g_error_matches(error, G_IO_ERROR, G_IO_ERROR_CONNECTION_REFUSED))
            g_print("No running Gnoblin session; the restored configuration will load at login.\n");
        else {
            g_printerr(
                "gnoblinctl: config files were restored, but live reload was not applied: %s\n",
                error ? error->message : "unknown compositor error");
            return 1;
        }
        return 0;
    }
    if (g_str_equal(cli.command, "lua")) {
        if (arg_count(&cli) > 1) {
            g_printerr("gnoblinctl lua accepts at most one Lua file\n");
            return 1;
        }
        return run_lua_console(&cli, arg(&cli, 0));
    }
    if (g_str_equal(cli.command, "session") && cli.action &&
        word_in("suspend hibernate reboot poweroff", cli.action)) {
        if (arg_count(&cli) != 0) {
            g_printerr("gnoblinctl session %s accepts no arguments\n", cli.action);
            return 1;
        }
        if (!run_power_action(cli.action, &error)) {
            g_printerr("gnoblinctl: %s\n", error->message);
            return 1;
        }
        return 0;
    }
    result = dispatch(&cli, &error);
    if (!result)
        goto failure;
    render(result, cli.format,
           g_str_equal(cli.command, "config") && g_strcmp0(cli.action, "default") == 0);
    return 0;

failure:
    g_printerr("gnoblinctl: %s\n", error ? error->message : "command failed");
    return 1;
}
