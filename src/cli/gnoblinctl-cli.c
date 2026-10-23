#include "gnoblinctl-internal.h"

static gboolean is_flag(const char* name) {
    return g_str_equal(name, "focused") || g_str_equal(name, "activate") ||
           g_str_equal(name, "follow") || g_str_equal(name, "autoplay") ||
           g_str_equal(name, "json") || g_str_equal(name, "help") || g_str_equal(name, "version");
}

const CommandSpec commands[] = {
    {"status", NULL},
    {"ping", NULL},
    {"version", NULL},
    {"capabilities", NULL},
    {"focus", "history policy"},
    {"reload", NULL},
    {"logout", NULL},
    {"session", "activity lock logout suspend hibernate reboot poweroff"},
    {"privacy", "stop-sharing stop-recording"},
    {"window", "list match menu interactive-move interactive-resize above unabove stick unstick "
               "focus close kill raise lower minimize unminimize toggle-minimize "
               "restore-or-minimize restore maximize "
               "unmaximize fullscreen "
               "unfullscreen move resize monitor workspace thumbnail"},
    {"layer", "list"},
    {"completion", NULL},
    {"shortcut", "actions list capture"},
    {"config", "path default show reload restore-default"},
    {"init", NULL},
    {"workspace", "list create rename remove switch next previous move-active"},
    {"monitor", "list"},
    {"output", "list primary"},
    {"input", "list current select devices orientation-lock"},
    {"launch", "status begin end"},
    {"animation", "list get surfaces inspect preview seek step play pause stop"},
    {"lua", NULL},
};

const gsize commands_count = G_N_ELEMENTS(commands);

const CommandSpec* find_command(const char* name) {
    if (!name)
        return NULL;
    for (guint i = 0; i < G_N_ELEMENTS(commands); i++)
        if (g_str_equal(commands[i].name, name))
            return &commands[i];
    return NULL;
}

gboolean word_in(const char* words, const char* word) {
    if (!words || !word)
        return FALSE;
    g_auto(GStrv) parts = g_strsplit(words, " ", -1);
    for (guint i = 0; parts[i]; i++)
        if (g_str_equal(parts[i], word))
            return TRUE;
    return FALSE;
}

/* One line for every command and action. The help screens and the "did you mean" hints read this
 * table. */
static const HelpEntry help_entries[] = {
    {"status", NULL, "Show the running session and lock availability"},
    {"ping", NULL, "Check whether the compositor control socket responds"},
    {"version", NULL, "Show the version reported by the running compositor"},
    {"capabilities", NULL, "List compositor and protocol capabilities"},
    {"focus", NULL, "Inspect focus policy and recent focus history"},
    {"focus", "policy", "Show the committed focus policy"},
    {"focus", "history", "List recently focused windows"},
    {"reload", NULL, "Reload supported runtime configuration"},
    {"logout", NULL, "End this Gnoblin session and return to the login manager"},
    {"session", NULL, "Read idle activity, lock, log out, suspend or power off"},
    {"session", "activity", "Read the latest idle-monitor sample"},
    {"session", "lock", "Ask a subscribed external shell client to lock the session"},
    {"session", "logout", "End this Gnoblin session and return to the login manager"},
    {"session", "suspend", "Suspend the machine"},
    {"session", "hibernate", "Hibernate the machine"},
    {"session", "reboot", "Restart the machine"},
    {"session", "poweroff", "Shut the machine down"},
    {"privacy", NULL, "Show privacy indicators, or stop sharing or recording"},
    {"privacy", "stop-sharing", "Stop screen sharing"},
    {"privacy", "stop-recording", "Stop recording"},
    {"window", NULL, "List and manage windows"},
    {"window", "list", "List open windows"},
    {"window", "match", "Show the fields used to match one window in config rules"},
    {"window", "menu", "Rejected: needs a trusted shell input context"},
    {"window", "interactive-move", "Rejected: needs a trusted shell input context"},
    {"window", "interactive-resize", "Rejected: needs a trusted shell input context"},
    {"window", "above", "Keep a window above others"},
    {"window", "unabove", "Stop keeping a window above others"},
    {"window", "stick", "Show a window on every workspace"},
    {"window", "unstick", "Show a window on one workspace only"},
    {"window", "focus", "Rejected: needs a one-use trusted context or an activation token"},
    {"window", "close", "Close a window"},
    {"window", "kill", "Force-quit a window by killing its client process"},
    {"window", "raise", "Raise a window above the others"},
    {"window", "lower", "Lower a window below the others"},
    {"window", "minimize", "Minimize a window"},
    {"window", "unminimize", "Bring back a minimized window"},
    {"window", "toggle-minimize", "Minimize a window, or bring it back if minimized"},
    {"window", "restore-or-minimize", "Restore a window, or minimize it, depending on its state"},
    {"window", "restore", "Restore a maximized, fullscreen or minimized window"},
    {"window", "maximize", "Maximize a window"},
    {"window", "unmaximize", "Stop maximizing a window"},
    {"window", "fullscreen", "Make a window fullscreen"},
    {"window", "unfullscreen", "Leave fullscreen"},
    {"window", "move", "Set position in logical screen coordinates"},
    {"window", "resize", "Set frame size in logical pixels"},
    {"window", "monitor", "Move to a connector ID, or use an index for active"},
    {"window", "workspace", "Move a window to an existing workspace"},
    {"window", "thumbnail", "Save a window thumbnail as a PNG"},
    {"layer", NULL, "Inspect layer-shell surfaces"},
    {"layer", "list", "List layer surfaces and their rule namespaces"},
    {"completion", NULL, "Print shell completion setup for bash, zsh or fish"},
    {"shortcut", NULL, "List configured shortcuts and capture key combinations"},
    {"shortcut", "actions", "List built-in shortcut actions, optionally by group"},
    {"shortcut", "list", "List shortcuts registered by the native compositor"},
    {"shortcut", "capture", "Capture one key combination and print its GTK accelerator"},
    {"config", NULL, "Show the default or manage the active configuration"},
    {"config", "path", "Show the active configuration path"},
    {"config", "default", "Print the bundled default Lua configuration"},
    {"config", "show", "Show the committed settings snapshot"},
    {"config", "reload", "Reload the active configuration"},
    {"config", "restore-default", "Back up and replace the config folder with embedded defaults"},
    {"init", NULL, "Create an editable default config tree from the binary"},
    {"workspace", NULL, "List and switch workspaces"},
    {"workspace", "list", "List workspaces"},
    {"workspace", "create", "Create a workspace"},
    {"workspace", "rename", "Rename a workspace"},
    {"workspace", "remove", "Remove a workspace"},
    {"workspace", "switch", "Switch to an existing workspace"},
    {"workspace", "next", "Switch to the next workspace"},
    {"workspace", "previous", "Switch to the previous workspace"},
    {"workspace", "move-active", "Move the active window to a workspace"},
    {"monitor", NULL, "List monitors"},
    {"monitor", "list", "List logical monitors and geometry"},
    {"output", NULL, "Inspect rendering outputs"},
    {"output", "list", "List output modes and renderer details"},
    {"output", "primary", "Show the primary output"},
    {"input", NULL, "Inspect and select input sources"},
    {"input", "list", "List configured keyboard sources"},
    {"input", "current", "Show the active keyboard source"},
    {"input", "select", "Select a configured keyboard source"},
    {"input", "devices", "List detected input devices and capabilities"},
    {"input", "orientation-lock", "Read or set the orientation lock"},
    {"launch", NULL, "Inspect launch feedback"},
    {"launch", "status", "Show pending app launches"},
    {"launch", "begin", "Start a global busy-cursor request"},
    {"launch", "end", "End a busy-cursor request"},
    {"animation", NULL, "Inspect and preview compositor animations"},
    {"animation", "list", "List configured animations"},
    {"animation", "get", "Show one configured animation"},
    {"animation", "surfaces", "List layer-shell animation targets"},
    {"animation", "inspect", "Inspect a configured animation"},
    {"animation", "preview", "Preview a configured animation"},
    {"animation", "seek", "Seek a preview to a percentage"},
    {"animation", "step", "Advance a preview by milliseconds"},
    {"animation", "play", "Play a preview session"},
    {"animation", "pause", "Pause a preview session"},
    {"animation", "stop", "Stop a preview session"},
    {"lua", NULL, "Open a Lua console, or run a Lua file, with the session API"},
};

const char* help_summary(const char* command, const char* action) {
    for (guint i = 0; i < G_N_ELEMENTS(help_entries); i++)
        if (g_str_equal(help_entries[i].command, command) &&
            g_strcmp0(help_entries[i].action, action) == 0)
            return help_entries[i].summary;
    return NULL;
}

/* Edit distance, for the "did you mean" hint. Names are short, so the plain table is fine. */
static guint edit_distance(const char* a, const char* b) {
    gsize length_a = strlen(a);
    gsize length_b = strlen(b);
    if (length_a > 40 || length_b > 40)
        return G_MAXUINT;
    guint row[41];
    for (gsize j = 0; j <= length_b; j++)
        row[j] = (guint)j;
    for (gsize i = 1; i <= length_a; i++) {
        guint diagonal = row[0];
        row[0] = (guint)i;
        for (gsize j = 1; j <= length_b; j++) {
            guint above = row[j];
            guint cost = a[i - 1] == b[j - 1] ? 0 : 1;
            row[j] = MIN(MIN(row[j] + 1, row[j - 1] + 1), diagonal + cost);
            diagonal = above;
        }
    }
    return row[length_b];
}

/* The closest name in a space separated list, or NULL when nothing is close enough to suggest. */
char* closest_word(const char* typed, const char* words) {
    if (!typed || !words)
        return NULL;
    g_auto(GStrv) parts = g_strsplit(words, " ", -1);
    const char* best = NULL;
    guint best_distance = G_MAXUINT;
    for (guint i = 0; parts[i]; i++) {
        guint distance = edit_distance(typed, parts[i]);
        if (g_str_has_prefix(parts[i], typed) && strlen(typed) >= 2)
            distance = 1;
        if (distance < best_distance) {
            best_distance = distance;
            best = parts[i];
        }
    }
    return best && best_distance <= MAX(2u, (guint)(strlen(typed) / 3)) ? g_strdup(best) : NULL;
}

const char* option(Cli* cli, const char* name) {
    return g_hash_table_lookup(cli->options, name);
}

gboolean has(Cli* cli, const char* name) {
    return g_hash_table_contains(cli->options, name);
}

gboolean parse_uint(const char* value, guint low, guint high, guint* result) {
    if (!value || !*value)
        return FALSE;
    char* end = NULL;
    guint64 number = g_ascii_strtoull(value, &end, 10);
    if (end == value || *end || number < low || number > high)
        return FALSE;
    *result = (guint)number;
    return TRUE;
}

gboolean parse_int(const char* value, gint low, gint high, gint* result) {
    if (!value || !*value)
        return FALSE;
    char* end = NULL;
    gint64 number = g_ascii_strtoll(value, &end, 10);
    if (end == value || *end || number < low || number > high)
        return FALSE;
    *result = (gint)number;
    return TRUE;
}

static char* executable_prefix(void) {
    g_autofree char* path = g_file_read_link("/proc/self/exe", NULL);
    if (!path)
        return NULL;
    g_autofree char* directory = g_path_get_dirname(path);
    return g_path_get_dirname(directory);
}

static char* installed_file(const char* relative) {
    g_autofree char* prefix = executable_prefix();
    return prefix ? g_build_filename(prefix, relative, NULL) : NULL;
}

static JsonNode* load_identity(void) {
    const char* override = g_getenv("GNOBLIN_VERSION_FILE");
    g_autofree char* installed = installed_file("share/gnoblin/version.json");
#ifdef GNOBLIN_IDENTITY_FALLBACK
    const char* fallback_path = GNOBLIN_IDENTITY_FALLBACK;
#else
    const char* fallback_path = NULL;
#endif
    const char* candidates[] = {override, installed, fallback_path};
    for (guint i = 0; i < G_N_ELEMENTS(candidates); i++) {
        if (!candidates[i])
            continue;
        g_autoptr(JsonParser) parser = json_parser_new();
        if (json_parser_load_from_file(parser, candidates[i], NULL) &&
            JSON_NODE_HOLDS_OBJECT(json_parser_get_root(parser)))
            return json_node_copy(json_parser_get_root(parser));
    }
    JsonObject* fallback = json_object_new();
    json_object_set_string_member(fallback, "version", "unknown");
    JsonNode* node = json_node_new(JSON_NODE_OBJECT);
    json_node_take_object(node, fallback);
    return node;
}

const char* member_string(JsonObject* object, const char* name, const char* fallback) {
    if (!object || !json_object_has_member(object, name))
        return fallback;
    JsonNode* node = json_object_get_member(object, name);
    return JSON_NODE_HOLDS_VALUE(node) && json_node_get_value_type(node) == G_TYPE_STRING
               ? json_node_get_string(node)
               : fallback;
}

JsonObject* member_object(JsonObject* object, const char* name) {
    if (!object || !json_object_has_member(object, name))
        return NULL;
    JsonNode* node = json_object_get_member(object, name);
    return JSON_NODE_HOLDS_OBJECT(node) ? json_node_get_object(node) : NULL;
}

JsonArray* member_array(JsonObject* object, const char* name) {
    JsonNode* node = object ? json_object_get_member(object, name) : NULL;
    return node && JSON_NODE_HOLDS_ARRAY(node) ? json_node_get_array(node) : NULL;
}

static void print_version_field(const char* label, const char* value) {
    g_print("%-11s %s\n", label, value);
}

void print_version(const char* format) {
    g_autoptr(JsonNode) identity = load_identity();
    if (g_str_equal(format, "json")) {
        g_autofree char* encoded = json_to_string(identity, TRUE);
        g_print("%s\n", encoded);
        return;
    }
    JsonObject* object = json_node_get_object(identity);
    g_print("Gnoblin %s\n", member_string(object, "version", "unknown"));

    /* The commit, the modified flag and the version already make up the build ID, so the text
     * output does not repeat it. The JSON record keeps buildId for scripts. */
    const char* build_time = member_string(object, "buildTime", NULL);
    if (build_time) {
        g_autoptr(GDateTime) built = g_date_time_new_from_iso8601(build_time, NULL);
        g_autofree char* formatted =
            built ? g_date_time_format(built, "%Y-%m-%d %H:%M:%S UTC") : NULL;
        print_version_field("Built", formatted ? formatted : build_time);
    }
    g_autofree char* commit =
        g_strdup_printf("%s%s", member_string(object, "gitSha", "unknown"),
                        json_object_has_member(object, "sourceModified") &&
                                json_object_get_boolean_member(object, "sourceModified")
                            ? " (modified)"
                            : "");
    print_version_field("Commit", commit);
    print_version_field("Remote", member_string(object, "gitRemote", "unknown"));
    print_version_field("Lua", member_string(object, "luaVersion", "unknown"));
    print_version_field("Native API", member_string(object, "apiVersion", "unknown"));
}

gboolean parse_cli(Cli* cli, int argc, char** argv, GError** error) {
    cli->positionals = g_ptr_array_new();
    cli->options = g_hash_table_new_full(g_str_hash, g_str_equal, g_free, g_free);
    gboolean values_only = FALSE;
    for (int i = 1; i < argc; i++) {
        const char* arg = argv[i];
        if (!values_only && g_str_equal(arg, "--")) {
            values_only = TRUE;
            continue;
        }
        if (!values_only && g_str_equal(arg, "-j")) {
            g_hash_table_replace(cli->options, g_strdup("json"), g_strdup("true"));
            continue;
        }
        if (!values_only && (g_str_equal(arg, "-h") || g_str_equal(arg, "--help"))) {
            cli->help = TRUE;
            continue;
        }
        if (!values_only && g_str_has_prefix(arg, "--")) {
            const char* name = arg + 2;
            const char* equals = strchr(name, '=');
            g_autofree char* key = equals ? g_strndup(name, equals - name) : g_strdup(name);
            const char* value = equals ? equals + 1 : NULL;
            if (!is_flag(key)) {
                if (!value && i + 1 < argc && argv[i + 1][0] != '-')
                    value = argv[++i];
                if (!value) {
                    g_set_error(error, G_OPTION_ERROR, G_OPTION_ERROR_BAD_VALUE,
                                "--%s needs a value", key);
                    return FALSE;
                }
            } else if (!value)
                value = "true";
            g_hash_table_replace(cli->options, g_strdup(key), g_strdup(value));
            continue;
        }
        if (!values_only && arg[0] == '-') {
            g_set_error(error, G_OPTION_ERROR, G_OPTION_ERROR_UNKNOWN_OPTION,
                        "unrecognized option: %s", arg);
            return FALSE;
        }
        g_ptr_array_add(cli->positionals, argv[i]);
    }

    if (has(cli, "version"))
        cli->version = TRUE;
    const char* format = option(cli, "format");
    cli->format = has(cli, "json") ? "json" : format ? format : "auto";
    if (!g_str_equal(cli->format, "auto") && !g_str_equal(cli->format, "json") &&
        !g_str_equal(cli->format, "table")) {
        g_set_error_literal(error, G_OPTION_ERROR, G_OPTION_ERROR_BAD_VALUE,
                            "--format must be auto, json, or table");
        return FALSE;
    }
    cli->timeout = 5;
    if (option(cli, "timeout") && !parse_uint(option(cli, "timeout"), 1, 60, &cli->timeout)) {
        g_set_error_literal(error, G_OPTION_ERROR, G_OPTION_ERROR_BAD_VALUE,
                            "--timeout must be between 1 and 60 seconds");
        return FALSE;
    }
    if (cli->positionals->len > 0)
        cli->command = g_ptr_array_index(cli->positionals, 0);
    if (cli->command && g_str_equal(cli->command, "help")) {
        cli->help = TRUE;
        cli->command = cli->positionals->len > 1 ? g_ptr_array_index(cli->positionals, 1) : NULL;
        cli->action = cli->positionals->len > 2 ? g_ptr_array_index(cli->positionals, 2) : NULL;
    } else if (cli->positionals->len > 1 && find_command(cli->command) &&
               find_command(cli->command)->actions)
        cli->action = g_ptr_array_index(cli->positionals, 1);

    const char* socket = option(cli, "socket");
    socket = socket ? socket : g_getenv("GNOBLIN_COMPOSITOR_SOCKET");
    if (socket)
        cli->socket_path = g_strdup(socket);
    else {
        const char* runtime = g_getenv("XDG_RUNTIME_DIR");
        g_autofree char* fallback =
            runtime ? NULL : g_strdup_printf("/run/user/%u", (guint)getuid());
        cli->socket_path =
            g_build_filename(runtime ? runtime : fallback, "gnoblin/compositor-v1.sock", NULL);
    }
    if (g_strcmp0(cli->command, "shortcut") == 0 && g_strcmp0(cli->action, "capture") == 0 &&
        !option(cli, "timeout"))
        cli->timeout = 30;
    return TRUE;
}

guint api_minor_for_method(const char* method) {
    static const struct {
        const char* name;
        guint minor;
    } methods[] = {
        {"session.logout", 32},
        {"session.suspend", 32},
        {"session.hibernate", 32},
        {"session.reboot", 32},
        {"session.poweroff", 32},
        {"config.restore_default", 32},
        {"window.restore_or_minimize", 48},
        {"privacy.stop_sharing", 31},
        {"privacy.stop_recording", 31},
        {"location.authorize_app", 65},
        {"auth.begin", 78},
        {"auth.respond", 78},
        {"auth.cancel", 78},
        {"prompt.respond", 79},
        {"prompt.cancel", 79},
        {"window.thumbnail", 23},
        {"shortcut.actions", 5},
        {"shortcut.capture", 8},
        {"shortcut.list", 9},
        {"session.status", 29},
        {"session.activity", 24},
        {"session.lock", 21},
        {"runtime.reload_config", 20},
        {"runtime.status", 67},
        {"version", 19},
        {"windows.list", 37},
        {"workspaces.list", 37},
        {"monitors.list", 37},
        {"outputs.list", 82},
        {"outputs.renderer", 82},
        {"outputs.list", 82},
        {"outputs.renderer", 82},
        {"monitors.privacy_screen", 74},
        {"monitors.set_privacy_screen", 74},
        {"layers.list", 37},
        {"launches.snapshot", 39},
        {"launch.begin", 50},
        {"launch.end", 50},
        {"shortcuts.list", 40},
        {"shortcuts.actions", 41},
        {"capabilities.list", 19},
        {"focus.history", 19},
        {"focus.policy", 19},
        {"settings", 19},
        {"layer.animation_policy", 31},
        {"privacy.state", 47},
        {"layer.list", 2},
        {"input.devices", 46},
        {"input.sources", 46},
        {"input.current_source", 46},
        {"input.orientation_lock", 66},
        {"appearance.color_scheme", 70},
        {"input.set_orientation_lock", 66},
        {"shortcut.bind", 22},
        {"shortcut.unbind", 11},
        {"shortcut.session.end", 64},
        {"input.select", 6},
        {"input.text_target", 28},
        {"input.insert_text", 28},
        {"launch.status", 8},
    };

    if (g_str_has_prefix(method, "animation."))
        return 18;
    for (guint i = 0; i < G_N_ELEMENTS(methods); i++)
        if (g_str_equal(method, methods[i].name))
            return methods[i].minor;
    /* Window records are a current API surface. Their operations are typed by
     * the supervisor, so negotiate the current minor rather than presenting a
     * legacy unversioned request. */
    if (g_str_has_prefix(method, "window."))
        return 64;
    if (g_str_has_prefix(method, "workspace."))
        return 64;
    return 8;
}
