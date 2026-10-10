#include "gnoblinctl-internal.h"

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
