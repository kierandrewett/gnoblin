/* Gnoblin control client. Uses the libraries already required by the session. */
#include <gio/gio.h>
#include <gio/gunixsocketaddress.h>
#include <json-glib/json-glib.h>
#include <glib/gstdio.h>
#include <lauxlib.h>
#include <lua.h>
#include <lualib.h>
#include <math.h>
#include <signal.h>
#include <stdio.h>
#include <string.h>
#include <sys/ioctl.h>
#include <unistd.h>

typedef struct {
    const char* command;
    const char* action;
    GPtrArray* positionals;
    GHashTable* options;
    const char* format;
    const char* socket_path;
    guint timeout;
    gboolean help;
    gboolean version;
} Cli;

static gboolean is_flag(const char* name) {
    return g_str_equal(name, "focused") || g_str_equal(name, "activate") ||
           g_str_equal(name, "follow") || g_str_equal(name, "autoplay") ||
           g_str_equal(name, "json") || g_str_equal(name, "help") || g_str_equal(name, "version");
}

typedef struct {
    const char* name;
    const char* actions;
} CommandSpec;

static const CommandSpec commands[] = {
    {"status", NULL},
    {"ping", NULL},
    {"version", NULL},
    {"capabilities", NULL},
    {"focus", "history policy"},
    {"reload", NULL},
    {"logout", NULL},
    {"session", "activity lock"},
    {"privacy", "stop-sharing stop-recording"},
    {"permissions", "list policy check"},
    {"window",
     "list match menu interactive-move interactive-resize above unabove stick unstick "
     "focus close minimize unminimize toggle-minimize restore-or-minimize restore maximize "
     "unmaximize fullscreen "
     "unfullscreen move resize monitor workspace thumbnail"},
    {"layer", "list"},
    {"completion", NULL},
    {"shortcut", "actions list capture"},
    {"config", "path default show reload"},
    {"workspace", "list create rename remove switch next previous move-active"},
    {"monitor", "list"},
    {"input", "list current select devices"},
    {"grant", "list revoke"},
    {"launch", "status begin end"},
    {"animation", "list get surfaces inspect preview seek step play pause stop"},
    {"lua", NULL},
};

static const CommandSpec* find_command(const char* name) {
    if (!name)
        return NULL;
    for (guint i = 0; i < G_N_ELEMENTS(commands); i++)
        if (g_str_equal(commands[i].name, name))
            return &commands[i];
    return NULL;
}

static gboolean word_in(const char* words, const char* word) {
    if (!words || !word)
        return FALSE;
    g_auto(GStrv) parts = g_strsplit(words, " ", -1);
    for (guint i = 0; parts[i]; i++)
        if (g_str_equal(parts[i], word))
            return TRUE;
    return FALSE;
}

static const char* option(Cli* cli, const char* name) {
    return g_hash_table_lookup(cli->options, name);
}

static gboolean has(Cli* cli, const char* name) {
    return g_hash_table_contains(cli->options, name);
}

static gboolean parse_uint(const char* value, guint low, guint high, guint* result) {
    if (!value || !*value)
        return FALSE;
    char* end = NULL;
    guint64 number = g_ascii_strtoull(value, &end, 10);
    if (end == value || *end || number < low || number > high)
        return FALSE;
    *result = (guint)number;
    return TRUE;
}

static gboolean parse_int(const char* value, gint low, gint high, gint* result) {
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

static const char* member_string(JsonObject* object, const char* name, const char* fallback) {
    if (!object || !json_object_has_member(object, name))
        return fallback;
    JsonNode* node = json_object_get_member(object, name);
    return JSON_NODE_HOLDS_VALUE(node) && json_node_get_value_type(node) == G_TYPE_STRING
               ? json_node_get_string(node)
               : fallback;
}

static JsonObject* member_object(JsonObject* object, const char* name) {
    if (!object || !json_object_has_member(object, name))
        return NULL;
    JsonNode* node = json_object_get_member(object, name);
    return JSON_NODE_HOLDS_OBJECT(node) ? json_node_get_object(node) : NULL;
}

static void print_version(const char* format) {
    g_autoptr(JsonNode) identity = load_identity();
    if (g_str_equal(format, "json")) {
        g_autofree char* encoded = json_to_string(identity, TRUE);
        g_print("%s\n", encoded);
        return;
    }
    JsonObject* object = json_node_get_object(identity);
    g_print("Gnoblin %s (GNOME %s)\n", member_string(object, "version", "unknown"),
            member_string(object, "gnomeVersion", "unknown"));
    g_print("Mutter: %s\n",
            member_string(member_object(object, "components"), "mutter", "unknown"));
    g_print("Lua: %s\n", member_string(object, "luaVersion", "unknown"));
    g_print("Native API: %s\n", member_string(object, "apiVersion", "unknown"));
    g_print("Build ID: %s\n", member_string(object, "buildId", "unknown"));
    const char* mutter_api = member_string(object, "mutterApi", NULL);
    if (mutter_api)
        g_print("Mutter API: %s\n", mutter_api);

    JsonObject* versions = member_object(object, "components");
    JsonObject* commits = member_object(object, "componentCommits");
    if (versions) {
        const struct {
            const char* name;
            const char* label;
        } components[] = {
            {"mutter", "Mutter"},
            {"xdg-desktop-portal-gnome", "Gnoblin portal backend"},
        };
        for (guint i = 0; i < G_N_ELEMENTS(components); i++) {
            const char* version = member_string(versions, components[i].name, NULL);
            if (!version)
                continue;
            const char* commit = member_string(commits, components[i].name, NULL);
            g_print("%s: %s", components[i].label, version);
            if (commit)
                g_print(" (upstream %.12s)", commit);
            g_print("\n");
        }
    }
    g_print("Git remote: %s\n", member_string(object, "gitRemote", "unknown"));
    g_print("Git commit: %s", member_string(object, "gitSha", "unknown"));
    if (json_object_has_member(object, "sourceModified") &&
        json_object_get_boolean_member(object, "sourceModified"))
        g_print(" (modified source tree)");
    g_print("\n");
}

static gboolean parse_cli(Cli* cli, int argc, char** argv, GError** error) {
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

static guint api_minor_for_method(const char* method) {
    static const struct {
        const char* name;
        guint minor;
    } methods[] = {
        {"session.logout", 32},
        {"window.restore_or_minimize", 48},
        {"privacy.stop_sharing", 31},
        {"privacy.stop_recording", 31},
        {"window.thumbnail", 23},
        {"shortcut.actions", 5},
        {"shortcut.capture", 8},
        {"shortcut.list", 9},
        {"session.status", 29},
        {"session.activity", 24},
        {"session.lock", 21},
        {"runtime.reload_config", 20},
        {"version", 19},
        {"windows.list", 37},
        {"workspaces.list", 37},
        {"monitors.list", 37},
        {"layers.list", 37},
        {"launches.snapshot", 39},
        {"shortcuts.list", 40},
        {"shortcuts.actions", 41},
        {"permissions.list", 42},
        {"permissions.check", 43},
        {"capabilities.list", 19},
        {"focus.history", 19},
        {"focus.policy", 19},
        {"settings", 19},
        {"layer.animation_policy", 31},
        {"privacy.state", 47},
        {"permissions.policy", 44},
        {"grant.list", 14},
        {"grant.revoke", 14},
        {"portals.grants", 45},
        {"layer.list", 2},
        {"input.devices", 46},
        {"input.sources", 46},
        {"input.current_source", 46},
        {"input.select", 6},
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

static JsonNode* call_compositor(Cli* cli, const char* op, const char* method,
                                 JsonObject* arguments, GError** error) {
    const char* method_name = method ? method : "";
    const char* operation_method = method_name;
    g_autofree char* id = g_uuid_string_random();
    g_autoptr(JsonBuilder) builder = json_builder_new();
    json_builder_begin_object(builder);
    json_builder_set_member_name(builder, "op");
    json_builder_add_string_value(builder, op);
    json_builder_set_member_name(builder, "id");
    json_builder_add_string_value(builder, id);
    if (method) {
        json_builder_set_member_name(builder, "method");
        json_builder_add_string_value(builder, method_name);
    }
    if (g_str_equal(method_name, "window.thumbnail") || g_str_equal(method_name, "layer.list") ||
        g_str_equal(method_name, "window.restore_or_minimize") ||
        g_str_equal(method_name, "input.devices") || g_str_equal(method_name, "input.sources") ||
        g_str_equal(method_name, "input.current_source") ||
        g_str_equal(method_name, "input.select") || g_str_equal(method_name, "shortcut.actions") ||
        g_str_equal(method_name, "shortcuts.actions") ||
        g_str_equal(method_name, "shortcut.capture") || g_str_equal(method_name, "shortcut.list") ||
        g_str_equal(method_name, "grant.list") || g_str_equal(method_name, "grant.revoke") ||
        g_str_equal(method_name, "portals.grants") ||
        g_str_equal(method_name, "permissions.policy") ||
        g_str_equal(method_name, "permissions.list") || g_str_equal(method_name, "privacy.state") ||
        g_str_equal(method_name, "permissions.check") || g_str_equal(method_name, "version") ||
        g_str_equal(method_name, "session.status") || g_str_equal(method_name, "session.logout") ||
        g_str_equal(method_name, "session.activity") || g_str_equal(method_name, "session.lock") ||
        g_str_equal(method_name, "privacy.stop_sharing") ||
        g_str_equal(method_name, "privacy.stop_recording") ||
        g_str_equal(method_name, "launch.status") ||
        g_str_equal(method_name, "launches.snapshot") ||
        g_str_equal(method_name, "shortcuts.actions") ||
        g_str_equal(method_name, "shortcuts.list") || g_str_equal(method_name, "portals.grants") ||
        g_str_equal(method_name, "capabilities.list") || g_str_equal(method_name, "windows.list") ||
        g_str_equal(method_name, "workspaces.list") || g_str_equal(method_name, "monitors.list") ||
        g_str_equal(method_name, "layers.list") || g_str_equal(method_name, "focus.history") ||
        g_str_equal(method_name, "focus.policy") || g_str_equal(method_name, "settings") ||
        g_str_equal(method_name, "layer.animation_policy") ||
        g_str_equal(method_name, "runtime.reload_config") ||
        g_str_has_prefix(method_name, "window.") || g_str_has_prefix(method_name, "workspace.") ||
        g_str_has_prefix(method_name, "animation.")) {
        json_builder_set_member_name(builder, "api_version");
        json_builder_begin_object(builder);
        json_builder_set_member_name(builder, "major");
        json_builder_add_int_value(builder, 1);
        json_builder_set_member_name(builder, "minor");
        json_builder_add_int_value(builder, api_minor_for_method(method_name));
        json_builder_end_object(builder);
    }
    if (method) {
        json_builder_set_member_name(builder, "arguments");
        json_builder_add_value(builder, json_node_init_object(json_node_alloc(), arguments));
    }
    json_builder_end_object(builder);
    g_autoptr(JsonNode) request = json_builder_get_root(builder);
    g_autofree char* encoded = json_to_string(request, FALSE);
    g_autofree char* payload = NULL;
    if (method) {
        g_autofree char* subscription_id = g_uuid_string_random();
        g_autoptr(JsonBuilder) subscription_builder = json_builder_new();
        json_builder_begin_object(subscription_builder);
        json_builder_set_member_name(subscription_builder, "op");
        json_builder_add_string_value(subscription_builder, "events");
        json_builder_set_member_name(subscription_builder, "id");
        json_builder_add_string_value(subscription_builder, subscription_id);
        json_builder_set_member_name(subscription_builder, "api_version");
        json_builder_begin_object(subscription_builder);
        json_builder_set_member_name(subscription_builder, "major");
        json_builder_add_int_value(subscription_builder, 1);
        json_builder_set_member_name(subscription_builder, "minor");
        json_builder_add_int_value(subscription_builder, 11);
        json_builder_end_object(subscription_builder);
        json_builder_set_member_name(subscription_builder, "events");
        json_builder_begin_array(subscription_builder);
        json_builder_add_string_value(subscription_builder, "gnoblin.operation.completed");
        json_builder_add_string_value(subscription_builder, "gnoblin.api.operation-completed");
        json_builder_end_array(subscription_builder);
        json_builder_end_object(subscription_builder);
        g_autoptr(JsonNode) subscription_request = json_builder_get_root(subscription_builder);
        g_autofree char* subscription_encoded = json_to_string(subscription_request, FALSE);
        payload = g_strdup_printf("%s\n%s\n", subscription_encoded, encoded);
    } else {
        payload = g_strconcat(encoded, "\n", NULL);
    }

    gboolean waits_for_operation =
        g_str_equal(method_name, "window.thumbnail") || g_str_equal(method_name, "input.select") ||
        g_str_equal(method_name, "shortcut.capture") || g_str_equal(method_name, "grant.list") ||
        g_str_equal(method_name, "grant.revoke") || g_str_equal(method_name, "animation.preview");
    guint wait_timeout = cli->timeout + (g_str_equal(method_name, "shortcut.capture") ? 2
                                         : g_str_has_prefix(method_name, "grant.")    ? 6
                                                                                      : 0);
    g_autoptr(GSocketClient) client = g_socket_client_new();
    g_socket_client_set_timeout(client, wait_timeout);
    g_autoptr(GSocketAddress) address = g_unix_socket_address_new(cli->socket_path);
    g_autoptr(GError) connect_error = NULL;
    g_autoptr(GSocketConnection) connection =
        g_socket_client_connect(client, G_SOCKET_CONNECTABLE(address), NULL, &connect_error);
    if (!connection) {
        g_set_error(error, G_IO_ERROR, G_IO_ERROR_CONNECTION_REFUSED,
                    "Cannot connect to compositor socket %s: %s. Check that a Gnoblin session is "
                    "running and that the socket path is correct.",
                    cli->socket_path, connect_error->message);
        return NULL;
    }
    g_socket_set_timeout(g_socket_connection_get_socket(connection), wait_timeout);
    if (!g_output_stream_write_all(g_io_stream_get_output_stream(G_IO_STREAM(connection)), payload,
                                   strlen(payload), NULL, NULL, error))
        return NULL;

    gint64 deadline = g_get_monotonic_time() + (gint64)wait_timeout * G_USEC_PER_SEC;
    gint64 operation_request_id = 0;
    g_autoptr(GString) pending = g_string_new(NULL);
    GInputStream* input = g_io_stream_get_input_stream(G_IO_STREAM(connection));
    while (g_get_monotonic_time() < deadline) {
        char chunk[4096];
        gssize count = g_input_stream_read(input, chunk, sizeof chunk, NULL, error);
        if (count < 0)
            return NULL;
        if (count == 0) {
            g_set_error_literal(
                error, G_IO_ERROR, G_IO_ERROR_CLOSED,
                "Compositor disconnected before replying; the request was not retried");
            return NULL;
        }
        g_string_append_len(pending, chunk, count);
        if (pending->len > 4 * 1024 * 1024) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NO_SPACE,
                                "Compositor response is too large");
            return NULL;
        }
        char* line_end;
        while ((line_end = memchr(pending->str, '\n', pending->len))) {
            g_autofree char* line = g_strndup(pending->str, line_end - pending->str);
            g_string_erase(pending, 0, line_end - pending->str + 1);
            g_autoptr(JsonParser) parser = json_parser_new();
            if (!json_parser_load_from_data(parser, line, -1, NULL) ||
                !JSON_NODE_HOLDS_OBJECT(json_parser_get_root(parser))) {
                g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                                    "Invalid compositor response");
                return NULL;
            }
            JsonObject* response = json_node_get_object(json_parser_get_root(parser));
            const char* event = member_string(response, "event", "");
            gboolean legacy_completion = g_str_equal(event, "gnoblin.api.operation-completed");
            gboolean canonical_completion = g_str_equal(event, "gnoblin.operation.completed");
            gint64 completion_id = json_object_get_int_member_with_default(
                response, legacy_completion ? "request_id" : "operation_id", 0);
            if (operation_request_id > 0 && (legacy_completion || canonical_completion) &&
                g_str_equal(member_string(response, "method", ""), operation_method) &&
                completion_id == operation_request_id) {
                if (!json_object_get_boolean_member_with_default(response, "ok", FALSE)) {
                    const char* message = member_string(response, "error", NULL);
                    JsonObject* details = member_object(response, "error");
                    if (!message && details)
                        message = member_string(details, "message", NULL);
                    g_set_error(error, G_IO_ERROR, G_IO_ERROR_FAILED, "%s",
                                message ? message : "Gnoblin operation failed");
                    return NULL;
                }
                const char* result_key = legacy_completion ? "result" : "value";
                JsonObject* result = member_object(response, result_key);
                if (!result) {
                    g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                                        "Invalid operation completion");
                    return NULL;
                }
                JsonNode* result_node = json_object_get_member(response, result_key);
                return json_node_copy(result_node);
            }
            if (!g_str_equal(member_string(response, "id", ""), id))
                continue;
            if (g_str_equal(event, "error")) {
                g_set_error_literal(
                    error, G_IO_ERROR, G_IO_ERROR_FAILED,
                    member_string(response, "message", "Compositor rejected request"));
                return NULL;
            }
            if (g_str_equal(event, "reply")) {
                JsonNode* result_node = json_object_get_member(response, "result");
                JsonObject* result = member_object(response, "result");
                gboolean read_method = word_in("version capabilities.list focus.history settings "
                                               "focus.policy launches.snapshot",
                                               method_name);
                gboolean operation_descriptor = result &&
                                                json_object_has_member(result, "request_id") &&
                                                json_object_has_member(result, "method");
                gboolean array_result = result_node && JSON_NODE_HOLDS_ARRAY(result_node) &&
                                        word_in("capabilities.list focus.history windows.list "
                                                "workspaces.list monitors.list layers.list "
                                                "shortcuts.actions shortcut.actions shortcut.list "
                                                "portals.grants "
                                                "shortcuts.list",
                                                method_name);
                gboolean null_result = result_node && JSON_NODE_HOLDS_NULL(result_node) &&
                                       (read_method || g_str_equal(method_name, "animation.get"));
                if (!result && !array_result && !null_result) {
                    g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                                        "Invalid compositor response");
                    return NULL;
                }
                if (waits_for_operation || operation_descriptor) {
                    JsonNode* request_id = json_object_get_member(result, "request_id");
                    if (!request_id || !JSON_NODE_HOLDS_VALUE(request_id) ||
                        (json_node_get_value_type(request_id) != G_TYPE_INT &&
                         json_node_get_value_type(request_id) != G_TYPE_INT64) ||
                        json_node_get_int(request_id) <= 0 ||
                        !g_str_equal(member_string(result, "method", ""), operation_method)) {
                        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                                            "Invalid operation descriptor");
                        return NULL;
                    }
                    operation_request_id = json_node_get_int(request_id);
                    continue;
                }
                return json_node_copy(result_node);
            }
        }
    }
    g_set_error_literal(
        error, G_IO_ERROR, G_IO_ERROR_TIMED_OUT,
        "Request timed out; it was not retried. Check current state before repeating an action.");
    return NULL;
}

static char* focused_window_id(Cli* cli, GError** error) {
    JsonObject* arguments = json_object_new();
    json_object_set_boolean_member(arguments, "focused", TRUE);
    g_autoptr(JsonNode) snapshot = call_compositor(cli, "api", "windows.list", arguments, error);
    json_object_unref(arguments);
    if (!snapshot)
        return NULL;
    if (!JSON_NODE_HOLDS_ARRAY(snapshot)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                            "Invalid focused-window snapshot");
        return NULL;
    }

    JsonArray* windows = json_node_get_array(snapshot);
    for (guint i = 0; i < json_array_get_length(windows); i++) {
        JsonObject* window = json_array_get_object_element(windows, i);
        if (!window)
            continue;
        JsonNode* focused = json_object_get_member(window, "focused");
        const char* id = member_string(window, "id", NULL);
        if (focused && JSON_NODE_HOLDS_VALUE(focused) &&
            json_node_get_value_type(focused) == G_TYPE_BOOLEAN && json_node_get_boolean(focused) &&
            id && *id)
            return g_strdup(id);
    }

    g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_FOUND,
                        "No focused window is available for this command");
    return NULL;
}

static JsonNode* window_match_from_snapshot(JsonArray* windows, const char* selector,
                                            GError** error) {
    JsonObject* selected = NULL;
    for (guint i = 0; i < json_array_get_length(windows); i++) {
        JsonObject* window = json_array_get_object_element(windows, i);
        if (!window)
            continue;
        if (g_str_equal(selector, "active")) {
            JsonNode* focused = json_object_get_member(window, "focused");
            if (focused && JSON_NODE_HOLDS_VALUE(focused) &&
                json_node_get_value_type(focused) == G_TYPE_BOOLEAN &&
                json_node_get_boolean(focused)) {
                selected = window;
                break;
            }
        } else if (g_str_equal(member_string(window, "id", ""), selector)) {
            selected = window;
            break;
        }
    }
    if (!selected) {
        g_set_error(error, G_IO_ERROR, G_IO_ERROR_NOT_FOUND,
                    "Window '%s' is not available in the current snapshot", selector);
        return NULL;
    }

    JsonObject* result = json_object_new();
    const char* identity_fields[] = {"id", "app_id", "gtk_app_id", "wm_class", "rule_app_id"};
    for (guint i = 0; i < G_N_ELEMENTS(identity_fields); i++) {
        JsonNode* value = json_object_get_member(selected, identity_fields[i]);
        if (value)
            json_object_set_member(result, identity_fields[i], json_node_copy(value));
    }

    JsonObject* match = json_object_new();
    json_object_set_string_member(match, "type", "window");
    JsonNode* rule_app_id = json_object_get_member(selected, "rule_app_id");
    JsonNode* title = json_object_get_member(selected, "title");
    JsonNode* focused = json_object_get_member(selected, "focused");
    if (rule_app_id)
        json_object_set_member(match, "app_id", json_node_copy(rule_app_id));
    if (title)
        json_object_set_member(match, "title", json_node_copy(title));
    if (focused)
        json_object_set_member(match, "focused", json_node_copy(focused));
    json_object_set_object_member(result, "match", match);

    JsonNode* node = json_node_new(JSON_NODE_OBJECT);
    json_node_take_object(node, result);
    return node;
}

static char* monitor_id_for_index(Cli* cli, guint monitor_index, GError** error) {
    g_autoptr(JsonObject) arguments = json_object_new();
    g_autoptr(JsonNode) snapshot = call_compositor(cli, "api", "monitors.list", arguments, error);
    if (!snapshot)
        return NULL;
    if (!JSON_NODE_HOLDS_ARRAY(snapshot)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA, "Invalid monitor snapshot");
        return NULL;
    }

    JsonArray* monitors = json_node_get_array(snapshot);
    for (guint i = 0; i < json_array_get_length(monitors); i++) {
        JsonObject* monitor = json_array_get_object_element(monitors, i);
        if (!monitor)
            continue;
        JsonNode* index = json_object_get_member(monitor, "index");
        const char* id = member_string(monitor, "id", NULL);
        if (index && JSON_NODE_HOLDS_VALUE(index) &&
            (json_node_get_value_type(index) == G_TYPE_INT64 ||
             json_node_get_value_type(index) == G_TYPE_INT) &&
            json_node_get_int(index) == monitor_index && id && *id)
            return g_strdup(id);
    }

    g_set_error(error, G_IO_ERROR, G_IO_ERROR_NOT_FOUND,
                "monitor index %u is not present in the current monitor list", monitor_index);
    return NULL;
}

static gboolean is(const char* left, const char* right) {
    return g_strcmp0(left, right) == 0;
}

static const char* arg(Cli* cli, guint index) {
    const CommandSpec* spec = find_command(cli->command);
    guint offset = (spec && spec->actions ? 2 : 1) + index;
    return offset < cli->positionals->len ? g_ptr_array_index(cli->positionals, offset) : NULL;
}

static guint arg_count(Cli* cli) {
    const CommandSpec* spec = find_command(cli->command);
    guint offset = spec && spec->actions ? 2 : 1;
    return cli->positionals->len > offset ? cli->positionals->len - offset : 0;
}

static gboolean validate_cli(Cli* cli, GError** error) {
    const CommandSpec* spec = find_command(cli->command);
    if (!spec) {
        g_set_error(error, G_OPTION_ERROR, G_OPTION_ERROR_UNKNOWN_OPTION, "unknown command: %s",
                    cli->command);
        return FALSE;
    }
    if (spec->actions && cli->action && !word_in(spec->actions, cli->action)) {
        g_set_error(error, G_OPTION_ERROR, G_OPTION_ERROR_UNKNOWN_OPTION, "unknown %s action: %s",
                    cli->command, cli->action);
        return FALSE;
    }
    const char* extra = NULL;
    if (g_strcmp0(cli->command, "window") == 0)
        extra = g_strcmp0(cli->action, "list") == 0        ? "app-id title focused"
                : g_strcmp0(cli->action, "workspace") == 0 ? "id number"
                : g_strcmp0(cli->action, "thumbnail") == 0 ? "output width height"
                                                           : NULL;
    else if (g_strcmp0(cli->command, "workspace") == 0)
        extra = g_strcmp0(cli->action, "create") == 0   ? "name id activate"
                : g_strcmp0(cli->action, "rename") == 0 ? "name id number"
                : g_strcmp0(cli->action, "remove") == 0 || g_strcmp0(cli->action, "switch") == 0
                    ? "id number"
                : g_strcmp0(cli->action, "move-active") == 0 ? "id number follow"
                                                             : NULL;
    else if (g_strcmp0(cli->command, "animation") == 0)
        extra = g_strcmp0(cli->action, "inspect") == 0   ? "event window layer namespace"
                : g_strcmp0(cli->action, "preview") == 0 ? "event window layer namespace autoplay"
                                                         : NULL;
    else if (g_strcmp0(cli->command, "focus") == 0 && g_strcmp0(cli->action, "history") == 0)
        extra = "workspace-id monitor-id limit";

    GHashTableIter iterator;
    gpointer key;
    g_hash_table_iter_init(&iterator, cli->options);
    while (g_hash_table_iter_next(&iterator, &key, NULL))
        if (!word_in("json format timeout socket version", key) && !word_in(extra, key)) {
            g_set_error(error, G_OPTION_ERROR, G_OPTION_ERROR_UNKNOWN_OPTION,
                        "unrecognized option for %s%s%s: --%s", cli->command,
                        cli->action ? " " : "", cli->action ? cli->action : "", (const char*)key);
            return FALSE;
        }

    if (!spec->actions && g_strcmp0(cli->command, "completion") != 0 &&
        g_strcmp0(cli->command, "lua") != 0 && arg_count(cli) != 0) {
        g_set_error(error, G_OPTION_ERROR, G_OPTION_ERROR_BAD_VALUE, "%s takes no arguments",
                    cli->command);
        return FALSE;
    }
    if (spec->actions && cli->action &&
        word_in("list current next previous surfaces path default show reload capture status "
                "activity lock stop-sharing stop-recording policy history",
                cli->action) &&
        arg_count(cli) != 0) {
        g_set_error(error, G_OPTION_ERROR, G_OPTION_ERROR_BAD_VALUE, "%s %s takes no arguments",
                    cli->command, cli->action);
        return FALSE;
    }
    if (g_strcmp0(cli->command, "animation") == 0 &&
        (g_strcmp0(cli->action, "inspect") == 0 || g_strcmp0(cli->action, "preview") == 0)) {
        guint targets = has(cli, "window") + has(cli, "layer") + has(cli, "namespace");
        if (targets > 1) {
            g_set_error_literal(error, G_OPTION_ERROR, G_OPTION_ERROR_BAD_VALUE,
                                "choose one of --window, --layer, or --namespace");
            return FALSE;
        }
    }
    return TRUE;
}

static gboolean require_count(Cli* cli, guint minimum, guint maximum, GError** error) {
    guint count = arg_count(cli);
    if (count >= minimum && count <= maximum)
        return TRUE;
    g_set_error(error, G_OPTION_ERROR, G_OPTION_ERROR_BAD_VALUE, "%s %s expects %u%s argument%s",
                cli->command, cli->action ? cli->action : "", minimum,
                minimum == maximum ? "" : " or more", minimum == 1 ? "" : "s");
    return FALSE;
}

static gboolean number_arg(Cli* cli, guint index, const char* name, gint low, gint high,
                           gint* result, GError** error) {
    if (parse_int(arg(cli, index), low, high, result))
        return TRUE;
    g_set_error(error, G_OPTION_ERROR, G_OPTION_ERROR_BAD_VALUE, "%s must be between %d and %d",
                name, low, high);
    return FALSE;
}

static JsonNode* string_node(const char* value) {
    JsonNode* node = json_node_new(JSON_NODE_VALUE);
    json_node_set_string(node, value);
    return node;
}

static JsonNode* object_node(JsonObject* object) {
    JsonNode* node = json_node_new(JSON_NODE_OBJECT);
    json_node_take_object(node, object);
    return node;
}

static JsonObject* new_object(void) {
    return json_object_new();
}

static void set_string(JsonObject* object, const char* name, const char* value) {
    json_object_set_string_member(object, name, value);
}

static void set_boolean(JsonObject* object, const char* name, gboolean value) {
    json_object_set_boolean_member(object, name, value);
}

static void set_number(JsonObject* object, const char* name, gint64 value) {
    json_object_set_int_member(object, name, value);
}

static void set_if(JsonObject* object, const char* name, const char* value) {
    if (value)
        set_string(object, name, value);
}

static gboolean workspace_selector(Cli* cli, guint positional, JsonObject* object, GError** error) {
    const char* stable = option(cli, "id");
    const char* number = option(cli, "number");
    const char* shorthand = arg(cli, positional);
    if ((stable != NULL) + (number != NULL) + (shorthand != NULL) != 1) {
        g_set_error_literal(error, G_OPTION_ERROR, G_OPTION_ERROR_BAD_VALUE,
                            "choose exactly one of --id, --number, or a workspace number");
        return FALSE;
    }
    if (stable)
        set_string(object, "id", stable);
    else {
        gint parsed;
        if (!parse_int(number ? number : shorthand, 1, 1024, &parsed)) {
            g_set_error_literal(error, G_OPTION_ERROR, G_OPTION_ERROR_BAD_VALUE,
                                "workspace number must be between 1 and 1024");
            return FALSE;
        }
        set_number(object, "number", parsed);
    }
    return TRUE;
}

static char* config_path(void) {
    const char* override = g_getenv("GNOBLIN_CONFIG");
    if (override && *override) {
        if (override[0] != '~')
            return g_strdup(override);
        if (override[1] == 0)
            return g_strdup(g_get_home_dir());
        if (override[1] == '/')
            return g_build_filename(g_get_home_dir(), override + 2, NULL);
        return g_strdup(override);
    }
    const char* config_home = g_getenv("XDG_CONFIG_HOME");
    g_autofree char* directory =
        config_home && *config_home
            ? g_build_filename(config_home, "gnoblin", NULL)
            : g_build_filename(g_get_home_dir(), ".config", "gnoblin", NULL);
    /* Match the runtime: legacy files remain visible until users convert them. */
    const char* names[] = {"init.lua", "gnoblin.toml", "gnoblin.conf"};
    for (guint i = 0; i < G_N_ELEMENTS(names); i++) {
        char* candidate = g_build_filename(directory, names[i], NULL);
        if (g_file_test(candidate, G_FILE_TEST_EXISTS))
            return candidate;
        g_free(candidate);
    }
    return g_build_filename(directory, "init.lua", NULL);
}

static char* default_config(GError** error) {
    g_autofree char* installed = installed_file("share/gnoblin/init.lua.example");
    const char* directories = g_getenv("XDG_DATA_DIRS");
    g_auto(GStrv) parts = g_strsplit(
        directories && *directories ? directories : "/usr/local/share:/usr/share", ":", -1);
    g_autoptr(GPtrArray) paths = g_ptr_array_new_with_free_func(g_free);
    if (installed)
        g_ptr_array_add(paths, g_strdup(installed));
    g_ptr_array_add(paths, g_strdup("src/data/init.lua.example"));
    for (guint i = 0; parts[i]; i++)
        if (*parts[i])
            g_ptr_array_add(paths, g_build_filename(parts[i], "gnoblin/init.lua.example", NULL));
    for (guint i = 0; i < paths->len; i++) {
        char* contents = NULL;
        if (g_file_get_contents(g_ptr_array_index(paths, i), &contents, NULL, NULL))
            return contents;
    }
    g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_FOUND,
                        "Default Lua config not found; install the Gnoblin session config package");
    return NULL;
}

static JsonNode* dispatch(Cli* cli, GError** error) {
    const char* command = cli->command;
    const char* action = cli->action;
    const char* op = "api";
    JsonObject* arguments = new_object();
    const char* method = NULL;
    g_autofree char* owned_method = NULL;
    g_autoptr(JsonNode) reply = NULL;

    if (is(command, "config")) {
        if (is(action, "path")) {
            json_object_unref(arguments);
            g_autofree char* path = config_path();
            return string_node(path);
        }
        if (is(action, "default")) {
            json_object_unref(arguments);
            g_autofree char* config = default_config(error);
            return config ? string_node(config) : NULL;
        }
        if (is(action, "reload"))
            method = "runtime.reload_config";
        else if (is(action, "show"))
            method = "settings";
    } else if (is(command, "window")) {
        if (is(action, "list")) {
            set_if(arguments, "app_id", option(cli, "app-id"));
            set_if(arguments, "title", option(cli, "title"));
            if (has(cli, "focused"))
                set_boolean(arguments, "focused", TRUE);
            method = "windows.list";
        } else if (is(action, "match")) {
            if (!require_count(cli, 0, 1, error))
                goto invalid;
            if (!arg(cli, 0) || is(arg(cli, 0), "active"))
                set_boolean(arguments, "focused", TRUE);
            method = "windows.list";
        } else if (is(action, "workspace")) {
            if (!require_count(cli, 1, 2, error))
                goto invalid;
            JsonObject* selector = new_object();
            if (!workspace_selector(cli, 1, selector, error)) {
                json_object_unref(selector);
                goto invalid;
            }
            if (is(arg(cli, 0), "active")) {
                set_string(arguments, "window", arg(cli, 0));
                json_object_set_object_member(arguments, "workspace", selector);
                method = "workspace.move_window";
            } else {
                set_string(arguments, "id", arg(cli, 0));
                json_object_set_object_member(arguments, "workspace", selector);
                method = "window.move_to_workspace";
            }
        } else if (is(action, "thumbnail")) {
            if (!require_count(cli, 1, 1, error))
                goto invalid;
            if (!option(cli, "output") || !*option(cli, "output")) {
                g_set_error_literal(error, G_OPTION_ERROR, G_OPTION_ERROR_BAD_VALUE,
                                    "window thumbnail requires --output PATH");
                goto invalid;
            }
            guint width = 320;
            guint height = 200;
            if ((option(cli, "width") && !parse_uint(option(cli, "width"), 1, 480, &width)) ||
                (option(cli, "height") && !parse_uint(option(cli, "height"), 1, 320, &height))) {
                g_set_error_literal(error, G_OPTION_ERROR, G_OPTION_ERROR_BAD_VALUE,
                                    "thumbnail bounds must be width 1..480 and height 1..320");
                goto invalid;
            }
            set_string(arguments, "id", arg(cli, 0));
            set_number(arguments, "width", width);
            set_number(arguments, "height", height);
            method = "window.thumbnail";
        } else if (action) {
            guint count = is(action, "move") || is(action, "resize") ? 3
                          : is(action, "monitor")                    ? 2
                                                                     : 1;
            guint minimum = count == 1 ? 0 : count;
            if (!require_count(cli, minimum, count, error))
                goto invalid;
            const char* window = arg(cli, 0) ? arg(cli, 0) : "active";
            if (is(action, "focus")) {
                g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                                    "window focus requires a one-use trusted context or an XDG "
                                    "activation token; gnoblinctl cannot create either");
                goto invalid;
            }
            if (word_in("menu interactive-move interactive-resize", action)) {
                g_set_error_literal(
                    error, G_IO_ERROR, G_IO_ERROR_NOT_SUPPORTED,
                    "window menu and interactive actions require a trusted shell input context; "
                    "gnoblinctl cannot create one");
                goto invalid;
            }
            if (is(action, "toggle-minimize") && is(window, "active")) {
                g_set_error_literal(error, G_OPTION_ERROR, G_OPTION_ERROR_BAD_VALUE,
                                    "window toggle-minimize requires a stable window ID");
                goto invalid;
            }
            gboolean active_window = is(window, "active");
            g_autofree char* resolved_window = NULL;
            if (is(window, "active")) {
                resolved_window = focused_window_id(cli, error);
                if (!resolved_window)
                    goto invalid;
                window = resolved_window;
            }
            set_string(arguments, "id", window);
            if (is(action, "close"))
                method = "window.close";
            else if (is(action, "minimize"))
                method = "window.minimize";
            else if (is(action, "unminimize"))
                method = "window.unminimize";
            else if (is(action, "restore"))
                method = "window.restore";
            else if (is(action, "restore-or-minimize"))
                method = "window.restore_or_minimize";
            else if (is(action, "toggle-minimize"))
                method = "window.toggle_minimize";
            else if (word_in("maximize unmaximize", action)) {
                method = "window.set_maximized";
                set_boolean(arguments, "enabled", is(action, "maximize"));
            } else if (word_in("fullscreen unfullscreen", action)) {
                method = "window.set_fullscreen";
                set_boolean(arguments, "enabled", is(action, "fullscreen"));
            } else if (word_in("above unabove", action)) {
                method = "window.set_above";
                set_boolean(arguments, "enabled", is(action, "above"));
            } else if (word_in("stick unstick", action)) {
                method = "window.set_sticky";
                set_boolean(arguments, "enabled", is(action, "stick"));
            } else if (is(action, "move"))
                method = "window.move";
            else if (is(action, "resize"))
                method = "window.resize";
            else if (is(action, "monitor"))
                method = "window.move_to_monitor";
            if (is(action, "move") || is(action, "resize") || is(action, "monitor")) {
                if (is(action, "monitor")) {
                    g_autofree char* indexed_monitor_id = NULL;
                    const char* monitor_id = arg(cli, 1);
                    if (active_window) {
                        guint monitor_index;
                        if (!parse_uint(monitor_id, 0, 1023, &monitor_index)) {
                            g_set_error_literal(
                                error, G_OPTION_ERROR, G_OPTION_ERROR_BAD_VALUE,
                                "active window monitor target must be an index from "
                                "gnoblinctl monitor list");
                            goto invalid;
                        }
                        indexed_monitor_id = monitor_id_for_index(cli, monitor_index, error);
                        if (!indexed_monitor_id)
                            goto invalid;
                        monitor_id = indexed_monitor_id;
                    }
                    if (!monitor_id || !*monitor_id || strlen(monitor_id) > 128 ||
                        !g_utf8_validate(monitor_id, -1, NULL)) {
                        g_set_error_literal(error, G_OPTION_ERROR, G_OPTION_ERROR_BAD_VALUE,
                                            "monitor must be a connector ID from monitor list");
                        goto invalid;
                    }
                    set_string(arguments, "monitor", monitor_id);
                } else {
                    const char* first = is(action, "move") ? "x" : "width";
                    const char* second = is(action, "move") ? "y" : "height";
                    gint value;
                    gint low = is(action, "move") ? -100000 : 1;
                    gint high = is(action, "move") ? 100000 : 32768;
                    if (!number_arg(cli, 1, first, low, high, &value, error))
                        goto invalid;
                    set_number(arguments, first, value);
                    if (count == 3) {
                        if (!number_arg(cli, 2, second, low, high, &value, error))
                            goto invalid;
                        set_number(arguments, second, value);
                    }
                }
            }
        }
    } else if (is(command, "workspace")) {
        if (is(action, "list"))
            method = "workspaces.list";
        else if (is(action, "next") || is(action, "previous"))
            method = owned_method = g_strdup_printf("workspace.%s", action);
        else if (is(action, "create")) {
            if (!require_count(cli, 0, 0, error))
                goto invalid;
            if (!option(cli, "name")) {
                g_set_error_literal(error, G_OPTION_ERROR, G_OPTION_ERROR_BAD_VALUE,
                                    "workspace create requires --name");
                goto invalid;
            }
            set_string(arguments, "name", option(cli, "name"));
            set_boolean(arguments, "activate", has(cli, "activate"));
            set_if(arguments, "id", option(cli, "id"));
            method = "workspace.create";
        } else if (is(action, "rename") || is(action, "remove") || is(action, "switch") ||
                   is(action, "move-active")) {
            if (!require_count(cli, 0, is(action, "switch") ? 1 : 0, error))
                goto invalid;
            if (!workspace_selector(cli, 0, arguments, error))
                goto invalid;
            if (is(action, "rename")) {
                if (!option(cli, "name")) {
                    g_set_error_literal(error, G_OPTION_ERROR, G_OPTION_ERROR_BAD_VALUE,
                                        "workspace rename requires --name");
                    goto invalid;
                }
                set_string(arguments, "name", option(cli, "name"));
            }
            if (is(action, "move-active")) {
                JsonObject* selector = json_object_ref(arguments);
                arguments = new_object();
                json_object_set_object_member(arguments, "workspace", selector);
                set_boolean(arguments, "follow", has(cli, "follow"));
            }
            method = is(action, "move-active")
                         ? "workspace.move_active"
                         : (owned_method = g_strdup_printf("workspace.%s", action));
        }
    } else if (is(command, "animation") && action) {
        if (is(action, "get")) {
            if (!require_count(cli, 1, 1, error))
                goto invalid;
            set_string(arguments, "name", arg(cli, 0));
        } else if (is(action, "inspect") || is(action, "preview")) {
            if (!require_count(cli, 1, 1, error))
                goto invalid;
            set_string(arguments, "name", arg(cli, 0));
            set_if(arguments, "event", option(cli, "event"));
            const char* target = option(cli, "window");
            const char* type = "window";
            if (option(cli, "layer")) {
                target = option(cli, "layer");
                type = "layer";
            }
            if (option(cli, "namespace")) {
                target = option(cli, "namespace");
                type = "namespace";
            }
            set_string(arguments, "target_type", type);
            set_string(arguments, "target", target ? target : "active");
            if (is(action, "preview"))
                set_boolean(arguments, "autoplay", has(cli, "autoplay"));
        } else if (is(action, "seek") || is(action, "step")) {
            if (!require_count(cli, 2, 2, error))
                goto invalid;
            set_string(arguments, "session", arg(cli, 0));
            gint value;
            if (!number_arg(cli, 1, "value", is(action, "seek") ? 0 : 1,
                            is(action, "seek") ? 100 : 60000, &value, error))
                goto invalid;
            if (is(action, "seek"))
                json_object_set_double_member(arguments, "progress", value / 100.0);
            else
                set_number(arguments, "milliseconds", value);
        } else if (is(action, "play") || is(action, "pause") || is(action, "stop")) {
            if (!require_count(cli, 1, 1, error))
                goto invalid;
            set_string(arguments, "session", arg(cli, 0));
        }
        method = owned_method = g_strdup_printf("animation.%s", action);
    } else if (is(command, "input")) {
        if (is(action, "devices")) {
            if (!require_count(cli, 0, 0, error))
                goto invalid;
            method = "input.devices";
        } else if (is(action, "list")) {
            method = "input.sources";
        } else if (is(action, "current")) {
            method = "input.current_source";
        } else if (is(action, "select")) {
            if (!require_count(cli, 2, 2, error))
                goto invalid;
            set_string(arguments, "type", arg(cli, 0));
            set_string(arguments, "id", arg(cli, 1));
            method = "input.select";
        } else if (action) {
            method = owned_method = g_strdup_printf("input.%s", action);
        }
    } else if (is(command, "permissions")) {
        if (is(action, "check")) {
            if (!require_count(cli, 2, 2, error))
                goto invalid;
            set_string(arguments, "capability", arg(cli, 0));
            set_string(arguments, "identity", arg(cli, 1));
            method = "permissions.check";
        } else if (is(action, "policy")) {
            if (!require_count(cli, 0, 0, error))
                goto invalid;
            method = "permissions.policy";
        } else {
            method = "permissions.list";
        }
    } else if (is(command, "grant")) {
        if (is(action, "revoke")) {
            if (!require_count(cli, 2, 2, error))
                goto invalid;
            if (!is(arg(cli, 0), "screen-cast") && !is(arg(cli, 0), "remote-desktop")) {
                g_set_error_literal(error, G_OPTION_ERROR, G_OPTION_ERROR_BAD_VALUE,
                                    "grant kind must be screen-cast or remote-desktop");
                goto invalid;
            }
            set_string(arguments, "kind", arg(cli, 0));
            set_string(arguments, "id", arg(cli, 1));
        }
        if (action)
            method = owned_method = g_strdup_printf("grant.%s", action);
    } else if (is(command, "launch")) {
        if (is(action, "status")) {
            if (!require_count(cli, 0, 0, error))
                goto invalid;
            method = "launches.snapshot";
        } else if (is(action, "begin")) {
            if (!require_count(cli, 2, 3, error))
                goto invalid;
            set_string(arguments, "token", arg(cli, 0));
            set_string(arguments, "application", arg(cli, 1));
            gint milliseconds = 3000;
            if (arg(cli, 2) && !number_arg(cli, 2, "milliseconds", 1, 60000, &milliseconds, error))
                goto invalid;
            set_number(arguments, "milliseconds", milliseconds);
        } else if (is(action, "end")) {
            if (!require_count(cli, 1, 1, error))
                goto invalid;
            set_string(arguments, "token", arg(cli, 0));
        }
        if (action && !is(action, "status"))
            method = owned_method = g_strdup_printf("launch.%s", action);
    } else if (is(command, "layer") && is(action, "list"))
        method = "layers.list";
    else if (is(command, "monitor") && is(action, "list"))
        method = "monitors.list";
    else if (is(command, "focus") && is(action, "history")) {
        set_if(arguments, "workspace_id", option(cli, "workspace-id"));
        set_if(arguments, "monitor_id", option(cli, "monitor-id"));
        if (option(cli, "limit")) {
            guint limit;
            if (!parse_uint(option(cli, "limit"), 1, 256, &limit)) {
                g_set_error_literal(error, G_OPTION_ERROR, G_OPTION_ERROR_BAD_VALUE,
                                    "--limit must be between 1 and 256");
                goto invalid;
            }
            set_number(arguments, "limit", limit);
        }
        method = "focus.history";
    } else if (is(command, "focus") && is(action, "policy"))
        method = "focus.policy";
    else if (is(command, "capabilities"))
        method = "capabilities.list";
    else if (is(command, "privacy") && is(action, "stop-sharing"))
        method = "privacy.stop_sharing";
    else if (is(command, "privacy") && is(action, "stop-recording"))
        method = "privacy.stop_recording";
    else if (is(command, "privacy"))
        method = "privacy.state";
    else if (is(command, "reload"))
        method = "runtime.reload_config";
    else if (is(command, "session") && is(action, "activity"))
        method = "session.activity";
    else if (is(command, "session") && is(action, "lock"))
        method = "session.lock";
    else if (is(command, "status"))
        method = "session.status";
    else if (is(command, "logout"))
        method = "session.logout";
    else if (is(command, "ping"))
        op = "ping";
    else if (is(command, "version"))
        method = "version";
    else if (is(command, "shortcut") && is(action, "list"))
        method = "shortcuts.list";
    else if (is(command, "shortcut") && is(action, "actions")) {
        if (arg_count(cli) > 1) {
            g_set_error_literal(error, G_OPTION_ERROR, G_OPTION_ERROR_BAD_VALUE,
                                "shortcut actions accepts at most one group");
            goto invalid;
        }
        if (arg_count(cli) == 1) {
            if (!word_in("wm mutter wayland", arg(cli, 0))) {
                g_set_error_literal(error, G_OPTION_ERROR, G_OPTION_ERROR_BAD_VALUE,
                                    "group must be wm, mutter, or wayland");
                goto invalid;
            }
            set_string(arguments, "group", arg(cli, 0));
        }
        method = "shortcuts.actions";
    } else if (is(command, "shortcut") && is(action, "capture")) {
        g_printerr("Press a shortcut now; Escape cancels.\n");
        set_number(arguments, "timeout", cli->timeout);
        method = "shortcut.capture";
    }

    if (!method && !g_str_equal(op, "ping")) {
        g_set_error_literal(error, G_OPTION_ERROR, G_OPTION_ERROR_UNKNOWN_OPTION,
                            "unknown command or action");
        goto invalid;
    }

    reply = call_compositor(cli, op, method, arguments, error);
    if (!reply)
        return NULL;
    if (is(command, "window") && is(action, "list") && JSON_NODE_HOLDS_ARRAY(reply)) {
        JsonObject* result = json_object_new();
        json_object_set_member(result, "windows", json_node_copy(reply));
        JsonNode* node = json_node_new(JSON_NODE_OBJECT);
        json_node_take_object(node, result);
        return node;
    }
    if (is(command, "window") && is(action, "match") && JSON_NODE_HOLDS_ARRAY(reply)) {
        g_autoptr(JsonNode) result = window_match_from_snapshot(
            json_node_get_array(reply), arg(cli, 0) ? arg(cli, 0) : "active", error);
        return result ? json_node_copy(result) : NULL;
    }
    if (is(command, "monitor") && is(action, "list") && JSON_NODE_HOLDS_ARRAY(reply)) {
        JsonObject* result = json_object_new();
        json_object_set_member(result, "monitors", json_node_copy(reply));
        JsonNode* node = json_node_new(JSON_NODE_OBJECT);
        json_node_take_object(node, result);
        return node;
    }
    if (is(command, "workspace") && is(action, "list") && JSON_NODE_HOLDS_ARRAY(reply)) {
        JsonNode* workspaces_node = json_node_copy(reply);
        JsonArray* workspaces = json_node_get_array(workspaces_node);
        for (guint i = 0; i < json_array_get_length(workspaces); i++) {
            JsonObject* workspace = json_array_get_object_element(workspaces, i);
            JsonNode* window_count =
                workspace ? json_object_get_member(workspace, "window_count") : NULL;
            if (window_count) {
                json_object_set_member(workspace, "windows", json_node_copy(window_count));
                json_object_remove_member(workspace, "window_count");
            }
        }
        JsonObject* result = json_object_new();
        json_object_set_member(result, "workspaces", workspaces_node);
        JsonNode* node = json_node_new(JSON_NODE_OBJECT);
        json_node_take_object(node, result);
        return node;
    }
    if (is(command, "layer") && is(action, "list") && JSON_NODE_HOLDS_ARRAY(reply)) {
        JsonObject* result = json_object_new();
        json_object_set_member(result, "layers", json_node_copy(reply));
        JsonNode* node = json_node_new(JSON_NODE_OBJECT);
        json_node_take_object(node, result);
        return node;
    }
    if (!JSON_NODE_HOLDS_OBJECT(reply)) {
        if (is(command, "animation") && is(action, "get") && JSON_NODE_HOLDS_NULL(reply))
            return json_node_copy(reply);
        if ((is(command, "capabilities") || (is(command, "focus") && is(action, "history")) ||
             (is(command, "shortcut") && (is(action, "actions") || is(action, "list")))) &&
            JSON_NODE_HOLDS_ARRAY(reply))
            return json_node_copy(reply);
        if ((is(command, "version") || is(command, "capabilities") || is(command, "focus") ||
             (is(command, "config") && is(action, "show"))) &&
            JSON_NODE_HOLDS_NULL(reply))
            return json_node_copy(reply);
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                            "Invalid compositor response");
        return NULL;
    }
    JsonObject* response = json_node_get_object(reply);
    if (is(command, "window") && is(action, "thumbnail")) {
        const char* encoded = member_string(response, "data", NULL);
        const char* window_id = member_string(response, "window_id", NULL);
        gint64 width = json_object_get_int_member_with_default(response, "width", 0);
        gint64 height = json_object_get_int_member_with_default(response, "height", 0);
        guint requested_width = 320;
        guint requested_height = 200;
        if ((option(cli, "width") && !parse_uint(option(cli, "width"), 1, 480, &requested_width)) ||
            (option(cli, "height") &&
             !parse_uint(option(cli, "height"), 1, 320, &requested_height))) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                                "Invalid thumbnail dimensions");
            return NULL;
        }
        gsize image_length = 0;
        g_autofree guchar* image = encoded ? g_base64_decode(encoded, &image_length) : NULL;
        static const guchar png_signature[] = {0x89, 'P', 'N', 'G', '\r', '\n', 0x1a, '\n'};

        if (!window_id || !g_str_equal(window_id, arg(cli, 0)) || width < 1 || width > 480 ||
            height < 1 || height > 320 || width > requested_width || height > requested_height ||
            !image || image_length < 24 || image_length > 512 * 1024 ||
            memcmp(image, png_signature, sizeof png_signature) != 0) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                                "Compositor returned an invalid window thumbnail");
            return NULL;
        }
        guint32 png_width = ((guint32)image[16] << 24) | ((guint32)image[17] << 16) |
                            ((guint32)image[18] << 8) | image[19];
        guint32 png_height = ((guint32)image[20] << 24) | ((guint32)image[21] << 16) |
                             ((guint32)image[22] << 8) | image[23];
        if (png_width != (guint32)width || png_height != (guint32)height) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                                "Thumbnail dimensions do not match the PNG image");
            return NULL;
        }

        g_autoptr(GFile) output = g_file_new_for_commandline_arg(option(cli, "output"));
        if (!g_file_replace_contents(output, (const char*)image, image_length, NULL, FALSE,
                                     G_FILE_CREATE_PRIVATE, NULL, NULL, error))
            return NULL;

        JsonObject* result = json_object_new();
        json_object_set_string_member(result, "path", option(cli, "output"));
        json_object_set_int_member(result, "width", width);
        json_object_set_int_member(result, "height", height);
        JsonNode* node = json_node_new(JSON_NODE_OBJECT);
        json_node_take_object(node, result);
        return node;
    }
    if (is(command, "version"))
        return json_node_copy(reply);
    if (is(command, "ping"))
        return string_node(member_string(response, "pong", ""));
    if (is(command, "shortcut") && is(action, "capture"))
        return string_node(member_string(response, "accelerator", ""));
    return json_node_copy(reply);

invalid:
    json_object_unref(arguments);
    return NULL;
}

#define GNOBLINCTL_READONLY_TABLE_METATABLE "gnoblinctl.ReadonlyTable"

static JsonNode* lua_to_json(lua_State* state, int index, guint depth, GError** error) {
    if (depth > 64) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "Lua values may be nested at most 64 levels");
        return NULL;
    }
    index = lua_absindex(state, index);
    switch (lua_type(state, index)) {
    case LUA_TNIL: {
        return json_node_new(JSON_NODE_NULL);
    }
    case LUA_TBOOLEAN: {
        JsonNode* node = json_node_new(JSON_NODE_VALUE);
        json_node_set_boolean(node, lua_toboolean(state, index));
        return node;
    }
    case LUA_TNUMBER: {
        if (lua_isinteger(state, index)) {
            JsonNode* node = json_node_new(JSON_NODE_VALUE);
            json_node_set_int(node, lua_tointeger(state, index));
            return node;
        }
        lua_Number number = lua_tonumber(state, index);
        if (!isfinite(number)) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                                "Lua numbers must be finite");
            return NULL;
        }
        JsonNode* node = json_node_new(JSON_NODE_VALUE);
        json_node_set_double(node, number);
        return node;
    }
    case LUA_TSTRING: {
        size_t length = 0;
        const char* value = lua_tolstring(state, index, &length);
        if (!g_utf8_validate(value, length, NULL) || memchr(value, '\0', length)) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                                "Lua strings must be valid UTF-8 without NUL bytes");
            return NULL;
        }
        JsonNode* node = json_node_new(JSON_NODE_VALUE);
        json_node_set_string(node, value);
        return node;
    }
    case LUA_TTABLE: {
        guint array_length = lua_rawlen(state, index);
        guint entries = 0;
        gboolean array = array_length > 0;
        lua_pushnil(state);
        while (lua_next(state, index)) {
            entries++;
            if (!lua_isinteger(state, -2) || lua_tointeger(state, -2) < 1 ||
                lua_tointeger(state, -2) > array_length)
                array = FALSE;
            lua_pop(state, 1);
        }
        if (array && entries != array_length)
            array = FALSE;
        if (array) {
            JsonArray* values = json_array_new();
            for (guint i = 1; i <= array_length; i++) {
                lua_rawgeti(state, index, i);
                JsonNode* value = lua_to_json(state, -1, depth + 1, error);
                lua_pop(state, 1);
                if (!value) {
                    json_array_unref(values);
                    return NULL;
                }
                json_array_add_element(values, value);
            }
            JsonNode* node = json_node_new(JSON_NODE_ARRAY);
            json_node_take_array(node, values);
            return node;
        }

        JsonObject* object = json_object_new();
        lua_pushnil(state);
        while (lua_next(state, index)) {
            if (lua_type(state, -2) != LUA_TSTRING) {
                lua_pop(state, 2);
                json_object_unref(object);
                g_set_error_literal(
                    error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                    "Lua object keys must be strings; array keys must be dense integers");
                return NULL;
            }
            const char* key = lua_tostring(state, -2);
            JsonNode* value = lua_to_json(state, -1, depth + 1, error);
            if (!value) {
                lua_pop(state, 2);
                json_object_unref(object);
                return NULL;
            }
            json_object_set_member(object, key, value);
            lua_pop(state, 1);
        }
        JsonNode* node = json_node_new(JSON_NODE_OBJECT);
        json_node_take_object(node, object);
        return node;
    }
    case LUA_TUSERDATA: {
        if (!luaL_testudata(state, index, GNOBLINCTL_READONLY_TABLE_METATABLE))
            break;
        lua_getiuservalue(state, index, 1);
        JsonNode* node = lua_to_json(state, -1, depth + 1, error);
        lua_pop(state, 1);
        return node;
    }
    default:
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "Lua API arguments must contain only JSON values");
        return NULL;
    }
}

static void json_to_lua(lua_State* state, JsonNode* node) {
    if (!node || JSON_NODE_HOLDS_NULL(node)) {
        lua_pushnil(state);
    } else if (JSON_NODE_HOLDS_OBJECT(node)) {
        JsonObject* object = json_node_get_object(node);
        lua_createtable(state, 0, json_object_get_size(object));
        GList* members = json_object_get_members(object);
        for (GList* item = members; item; item = item->next) {
            const char* key = item->data;
            json_to_lua(state, json_object_get_member(object, key));
            lua_setfield(state, -2, key);
        }
        g_list_free(members);
    } else if (JSON_NODE_HOLDS_ARRAY(node)) {
        JsonArray* array = json_node_get_array(node);
        guint length = json_array_get_length(array);
        lua_createtable(state, length, 0);
        for (guint i = 0; i < length; i++) {
            json_to_lua(state, json_array_get_element(array, i));
            lua_rawseti(state, -2, i + 1);
        }
    } else {
        GType type = json_node_get_value_type(node);
        if (type == G_TYPE_BOOLEAN)
            lua_pushboolean(state, json_node_get_boolean(node));
        else if (type == G_TYPE_INT || type == G_TYPE_INT64)
            lua_pushinteger(state, json_node_get_int(node));
        else if (type == G_TYPE_DOUBLE)
            lua_pushnumber(state, json_node_get_double(node));
        else if (type == G_TYPE_STRING)
            lua_pushstring(state, json_node_get_string(node));
        else
            lua_pushnil(state);
    }
}

#define GNOBLINCTL_WINDOW_RECORD_METATABLE "gnoblinctl.Window"
#define GNOBLINCTL_WORKSPACE_RECORD_METATABLE "gnoblinctl.Workspace"
#define GNOBLINCTL_MONITOR_RECORD_METATABLE "gnoblinctl.Monitor"
#define GNOBLINCTL_LAYER_SURFACE_RECORD_METATABLE "gnoblinctl.LayerSurface"
#define GNOBLINCTL_ANIMATION_PREVIEW_RECORD_METATABLE "gnoblinctl.AnimationPreview"
#define GNOBLINCTL_PORTAL_GRANT_RECORD_METATABLE "gnoblinctl.PortalGrant"
#define GNOBLINCTL_INPUT_DEVICE_RECORD_METATABLE "gnoblinctl.InputDevice"
#define GNOBLINCTL_INPUT_SOURCE_RECORD_METATABLE "gnoblinctl.InputSource"
#define GNOBLINCTL_SHORTCUT_STATE_RECORD_METATABLE "gnoblinctl.ShortcutState"
#define GNOBLINCTL_SHORTCUT_ACTION_RECORD_METATABLE "gnoblinctl.ShortcutAction"
#define GNOBLINCTL_FOCUS_POLICY_RECORD_METATABLE "gnoblinctl.FocusPolicy"
#define GNOBLINCTL_SETTINGS_RECORD_METATABLE "gnoblinctl.Settings"
#define GNOBLINCTL_LAYER_ANIMATION_POLICY_RECORD_METATABLE "gnoblinctl.LayerAnimationPolicy"
#define GNOBLINCTL_PRIVACY_STATE_RECORD_METATABLE "gnoblinctl.PrivacyState"
#define GNOBLINCTL_CAPABILITY_RECORD_METATABLE "gnoblinctl.Capability"
#define GNOBLINCTL_PERMISSION_POLICY_RECORD_METATABLE "gnoblinctl.PermissionPolicy"
#define GNOBLINCTL_SESSION_STATUS_RECORD_METATABLE "gnoblinctl.SessionStatus"
#define GNOBLINCTL_SESSION_ACTIVITY_RECORD_METATABLE "gnoblinctl.SessionActivity"
#define GNOBLINCTL_PERMISSION_DECISION_RECORD_METATABLE "gnoblinctl.PermissionDecision"

static int lua_cli_animation_preview_method(lua_State* state);
static int lua_cli_animations_list(lua_State* state);
static int lua_cli_animations_get(lua_State* state);
static int lua_cli_animations_surfaces(lua_State* state);
static int lua_cli_launches_read(lua_State* state);
static int lua_cli_portal_grant_revoke(lua_State* state);
static int lua_cli_input_snapshot(lua_State* state);
static int lua_cli_input_select_source(lua_State* state);
static int lua_cli_shortcuts_list(lua_State* state);
static int lua_cli_shortcuts_actions(lua_State* state);
static int lua_cli_shortcuts_capture(lua_State* state);
static int lua_cli_focus_policy_property(lua_State* state);
static int lua_cli_settings_property(lua_State* state);
static int lua_cli_layer_animation_policy(lua_State* state);
static int lua_cli_privacy_state(lua_State* state);
static int lua_cli_capabilities_list(lua_State* state);
static int lua_cli_permissions_policy(lua_State* state);
static int lua_cli_permissions_list(lua_State* state);
static int lua_cli_session_status(lua_State* state);
static int lua_cli_session_activity(lua_State* state);
static int lua_cli_permissions_check(lua_State* state);
static int lua_cli_version(lua_State* state);

/* Nested JSON values are userdata-backed proxies instead of ordinary Lua
 * tables. An empty proxy table would still allow rawset() to shadow fields,
 * while returning the backing table from __pairs would expose mutable state. */
static void lua_cli_push_readonly_value(lua_State* state, int index) {
    index = lua_absindex(state, index);
    if (!lua_istable(state, index)) {
        lua_pushvalue(state, index);
        return;
    }

    lua_newuserdatauv(state, 1, 1);
    int proxy = lua_absindex(state, -1);
    lua_pushvalue(state, index);
    lua_setiuservalue(state, proxy, 1);
    luaL_getmetatable(state, GNOBLINCTL_READONLY_TABLE_METATABLE);
    lua_setmetatable(state, proxy);
}

static int lua_cli_readonly_index(lua_State* state) {
    lua_getiuservalue(state, 1, 1);
    lua_pushvalue(state, 2);
    lua_gettable(state, -2);
    lua_cli_push_readonly_value(state, -1);
    return 1;
}

static int lua_cli_readonly_newindex(lua_State* state) {
    return luaL_error(state, "Snapshot values are read-only");
}

static int lua_cli_readonly_len(lua_State* state) {
    lua_getiuservalue(state, 1, 1);
    lua_pushinteger(state, (lua_Integer)lua_rawlen(state, -1));
    return 1;
}

static int lua_cli_readonly_next(lua_State* state) {
    lua_pushvalue(state, lua_upvalueindex(1));
    int backing = lua_absindex(state, -1);
    lua_pushvalue(state, 2);
    if (!lua_next(state, backing))
        return 0;
    lua_cli_push_readonly_value(state, -1);
    lua_remove(state, -2);
    lua_remove(state, backing);
    return 2;
}

static int lua_cli_readonly_pairs(lua_State* state) {
    lua_getiuservalue(state, 1, 1);
    lua_pushcclosure(state, lua_cli_readonly_next, 1);
    lua_pushnil(state);
    lua_pushnil(state);
    return 3;
}

static void register_lua_cli_readonly_table(lua_State* state) {
    if (!luaL_newmetatable(state, GNOBLINCTL_READONLY_TABLE_METATABLE)) {
        lua_pop(state, 1);
        return;
    }
    lua_pushcfunction(state, lua_cli_readonly_index);
    lua_setfield(state, -2, "__index");
    lua_pushcfunction(state, lua_cli_readonly_newindex);
    lua_setfield(state, -2, "__newindex");
    lua_pushcfunction(state, lua_cli_readonly_len);
    lua_setfield(state, -2, "__len");
    lua_pushcfunction(state, lua_cli_readonly_pairs);
    lua_setfield(state, -2, "__pairs");
    lua_pushliteral(state, "read-only snapshot");
    lua_setfield(state, -2, "__metatable");
    lua_pop(state, 1);
}

/* The console must not turn a compositor snapshot into a mutable policy object.
 * Keep the JSON fields in a private backing table and expose the same read-only
 * property/method split as the supervised runtime's Window records. */
static int lua_cli_window_index(lua_State* state) {
    lua_getiuservalue(state, 1, 1);
    lua_pushvalue(state, 2);
    lua_gettable(state, -2);
    if (!lua_isnil(state, -1)) {
        lua_cli_push_readonly_value(state, -1);
        return 1;
    }
    lua_pop(state, 2);
    lua_getiuservalue(state, 1, 2);
    lua_pushvalue(state, 2);
    lua_gettable(state, -2);
    return 1;
}

static int lua_cli_window_newindex(lua_State* state) {
    return luaL_error(state, "Window records are read-only");
}

static int lua_cli_window_len(lua_State* state) {
    lua_getiuservalue(state, 1, 1);
    lua_pushinteger(state, (lua_Integer)lua_rawlen(state, -1));
    return 1;
}

static int lua_cli_window_pairs(lua_State* state) {
    lua_getiuservalue(state, 1, 1);
    lua_pushcclosure(state, lua_cli_readonly_next, 1);
    lua_pushnil(state);
    lua_pushnil(state);
    return 3;
}

static int lua_cli_window_tostring(lua_State* state) {
    lua_getiuservalue(state, 1, 1);
    lua_getfield(state, -1, "id");
    const char* id = lua_tostring(state, -1);
    lua_pushfstring(state, "Window<%s>", id ? id : "unknown");
    return 1;
}

static const char* lua_cli_window_id(lua_State* state) {
    luaL_checkudata(state, 1, GNOBLINCTL_WINDOW_RECORD_METATABLE);
    lua_getiuservalue(state, 1, 1);
    lua_getfield(state, -1, "id");
    const char* id = lua_tostring(state, -1);
    if (!id || !*id)
        luaL_error(state, "Window record has no stable id");
    return id;
}

static const char* lua_cli_workspace_id(lua_State* state) {
    luaL_checkudata(state, 1, GNOBLINCTL_WORKSPACE_RECORD_METATABLE);
    lua_getiuservalue(state, 1, 1);
    lua_getfield(state, -1, "id");
    const char* id = lua_tostring(state, -1);
    if (!id || !*id)
        luaL_error(state, "Workspace record has no stable id");
    return id;
}

static const char* lua_cli_animation_preview_session(lua_State* state) {
    luaL_checkudata(state, 1, GNOBLINCTL_ANIMATION_PREVIEW_RECORD_METATABLE);
    lua_getiuservalue(state, 1, 3);
    const char* session = lua_tostring(state, -1);
    if (!session || !*session)
        luaL_error(state, "AnimationPreview record has no session ID");
    return session;
}

static gboolean lua_cli_animation_preview_valid(JsonObject* object) {
    static const char* const string_fields[] = {"id", "name", "event", "target", "target_type"};
    for (guint i = 0; i < G_N_ELEMENTS(string_fields); i++) {
        JsonNode* field = json_object_get_member(object, string_fields[i]);
        if (!field || !JSON_NODE_HOLDS_VALUE(field) ||
            json_node_get_value_type(field) != G_TYPE_STRING)
            return FALSE;
    }
    JsonNode* progress = json_object_get_member(object, "progress");
    JsonNode* playing = json_object_get_member(object, "playing");
    JsonNode* revision = json_object_get_member(object, "revision");
    return progress && JSON_NODE_HOLDS_VALUE(progress) &&
           (json_node_get_value_type(progress) == G_TYPE_DOUBLE ||
            json_node_get_value_type(progress) == G_TYPE_INT ||
            json_node_get_value_type(progress) == G_TYPE_INT64) &&
           playing && JSON_NODE_HOLDS_VALUE(playing) &&
           json_node_get_value_type(playing) == G_TYPE_BOOLEAN && revision &&
           JSON_NODE_HOLDS_VALUE(revision) &&
           (json_node_get_value_type(revision) == G_TYPE_INT ||
            json_node_get_value_type(revision) == G_TYPE_INT64);
}

static void lua_cli_push_animation_preview_record(lua_State* state, Cli* cli, JsonObject* object,
                                                  const char* session) {
    if (!lua_cli_animation_preview_valid(object) || !session || !*session)
        luaL_error(state, "animation preview completion returned an invalid AnimationPreview");

    /* The compositor's session token is an implementation detail used to
     * address subsequent controls. Keep it out of the public read-only record. */
    JsonObject* public_fields = json_object_new();
    GList* members = json_object_get_members(object);
    for (GList* item = members; item; item = item->next) {
        const char* key = item->data;
        if (!g_str_equal(key, "session"))
            json_object_set_member(public_fields, key,
                                   json_node_copy(json_object_get_member(object, key)));
    }
    g_list_free(members);
    g_autoptr(JsonNode) node = json_node_new(JSON_NODE_OBJECT);
    json_node_take_object(node, public_fields);
    json_to_lua(state, node);
    int backing = lua_absindex(state, -1);
    lua_newuserdatauv(state, 1, 3);
    int record = lua_absindex(state, -1);
    lua_pushvalue(state, backing);
    lua_setiuservalue(state, record, 1);
    lua_newtable(state);
    static const char* const methods[][2] = {
        {"seek", "animation.seek"},   {"step", "animation.step"}, {"play", "animation.play"},
        {"pause", "animation.pause"}, {"stop", "animation.stop"}, {NULL, NULL},
    };
    for (guint i = 0; methods[i][0]; i++) {
        lua_pushlightuserdata(state, cli);
        lua_pushstring(state, methods[i][1]);
        lua_pushcclosure(state, lua_cli_animation_preview_method, 2);
        lua_setfield(state, -2, methods[i][0]);
    }
    lua_setiuservalue(state, record, 2);
    lua_pushstring(state, session);
    lua_setiuservalue(state, record, 3);
    luaL_getmetatable(state, GNOBLINCTL_ANIMATION_PREVIEW_RECORD_METATABLE);
    lua_setmetatable(state, record);
    lua_remove(state, backing);
}

static JsonObject* lua_cli_table_object(lua_State* state, int index, const char* description) {
    g_autoptr(GError) conversion_error = NULL;
    g_autoptr(JsonNode) node = lua_to_json(state, index, 0, &conversion_error);
    if (!node)
        luaL_error(state, "%s: %s", description, conversion_error->message);
    if (!JSON_NODE_HOLDS_OBJECT(node))
        luaL_error(state, "%s must be a named table", description);
    return json_object_ref(json_node_get_object(node));
}

static int lua_cli_animation_preview_method(lua_State* state) {
    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    const char* method = lua_tostring(state, lua_upvalueindex(2));
    int supplied = lua_gettop(state) - 1;
    g_autofree char* session = g_strdup(lua_cli_animation_preview_session(state));
    lua_settop(state, supplied + 1);

    JsonObject* arguments = json_object_new();
    json_object_set_string_member(arguments, "session", session);
    if (g_str_equal(method, "animation.seek")) {
        if (supplied != 1 || lua_type(state, 2) != LUA_TNUMBER) {
            json_object_unref(arguments);
            return luaL_error(state, "preview:seek requires a number from 0 to 1");
        }
        double progress = lua_tonumber(state, 2);
        if (!isfinite(progress) || progress < 0.0 || progress > 1.0) {
            json_object_unref(arguments);
            return luaL_error(state, "preview:seek requires a number from 0 to 1");
        }
        json_object_set_double_member(arguments, "progress", progress);
    } else if (g_str_equal(method, "animation.step")) {
        if (supplied != 1 || !lua_isinteger(state, 2) || lua_tointeger(state, 2) < 1 ||
            lua_tointeger(state, 2) > 60000) {
            json_object_unref(arguments);
            return luaL_error(state, "preview:step requires an integer from 1 to 60000");
        }
        json_object_set_int_member(arguments, "milliseconds", lua_tointeger(state, 2));
    } else if (g_str_equal(method, "animation.play") || g_str_equal(method, "animation.pause") ||
               g_str_equal(method, "animation.stop")) {
        if (supplied != 0) {
            json_object_unref(arguments);
            return luaL_error(state, "%s takes no arguments", method + strlen("animation."));
        }
    } else {
        json_object_unref(arguments);
        return luaL_error(state, "unsupported AnimationPreview method %s", method);
    }

    g_autoptr(GError) call_error = NULL;
    g_autoptr(JsonNode) result = call_compositor(cli, "api", method, arguments, &call_error);
    json_object_unref(arguments);
    if (!result)
        return luaL_error(state, "%s failed: %s", method, call_error->message);
    if (g_str_equal(method, "animation.stop")) {
        json_to_lua(state, result);
        return 1;
    }
    if (!JSON_NODE_HOLDS_OBJECT(result))
        return luaL_error(state, "%s returned an invalid AnimationPreview", method);
    JsonObject* preview = json_node_get_object(result);
    const char* next_session = member_string(preview, "session", NULL);
    if (!next_session)
        next_session = member_string(preview, "id", NULL);
    lua_cli_push_animation_preview_record(state, cli, preview, next_session);
    return 1;
}

static int lua_cli_animations_preview(lua_State* state) {
    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    if (lua_gettop(state) != 1 || !lua_istable(state, 1))
        return luaL_error(state, "gnoblin.animations.preview requires one spec table");
    g_autoptr(JsonObject) arguments = lua_cli_table_object(state, 1, "animation preview spec");
    g_autoptr(GError) call_error = NULL;
    g_autoptr(JsonNode) result =
        call_compositor(cli, "api", "animation.preview", arguments, &call_error);
    if (!result)
        return luaL_error(state, "gnoblin.animations.preview failed: %s", call_error->message);
    if (!JSON_NODE_HOLDS_OBJECT(result))
        return luaL_error(state, "gnoblin.animations.preview returned an invalid preview");
    JsonObject* preview = json_node_get_object(result);
    const char* session = member_string(preview, "session", NULL);
    if (!session)
        session = member_string(preview, "id", NULL);
    lua_cli_push_animation_preview_record(state, cli, preview, session);
    return 1;
}

static int lua_cli_animations_list(lua_State* state) {
    if (lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.animations.list takes no arguments");
    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    JsonObject* arguments = json_object_new();
    g_autoptr(GError) call_error = NULL;
    g_autoptr(JsonNode) result =
        call_compositor(cli, "api", "animation.list", arguments, &call_error);
    json_object_unref(arguments);
    if (!result)
        return luaL_error(state, "gnoblin.animations.list failed: %s", call_error->message);
    JsonObject* response = JSON_NODE_HOLDS_OBJECT(result) ? json_node_get_object(result) : NULL;
    JsonArray* animations = response ? json_object_get_array_member(response, "animations") : NULL;
    if (!animations)
        return luaL_error(state, "gnoblin.animations.list returned an invalid snapshot");

    lua_createtable(state, json_array_get_length(animations), 0);
    for (guint i = 0; i < json_array_get_length(animations); i++) {
        JsonNode* animation = json_array_get_element(animations, i);
        JsonObject* record =
            JSON_NODE_HOLDS_OBJECT(animation) ? json_node_get_object(animation) : NULL;
        const char* name = record ? member_string(record, "name", NULL) : NULL;
        if (!name || !*name)
            return luaL_error(state, "gnoblin.animations.list returned an invalid AnimationInfo");
        json_to_lua(state, animation);
        lua_cli_push_readonly_value(state, -1);
        lua_remove(state, -2);
        lua_rawseti(state, -2, i + 1);
    }
    lua_cli_push_readonly_value(state, -1);
    lua_remove(state, -2);
    return 1;
}

static int lua_cli_animations_get(lua_State* state) {
    if (lua_gettop(state) != 1 || lua_type(state, 1) != LUA_TSTRING || !*lua_tostring(state, 1))
        return luaL_error(state, "gnoblin.animations.get requires one animation name");
    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    JsonObject* arguments = json_object_new();
    json_object_set_string_member(arguments, "name", lua_tostring(state, 1));
    g_autoptr(GError) call_error = NULL;
    g_autoptr(JsonNode) result =
        call_compositor(cli, "api", "animation.get", arguments, &call_error);
    json_object_unref(arguments);
    if (!result)
        return luaL_error(state, "gnoblin.animations.get failed: %s", call_error->message);
    if (JSON_NODE_HOLDS_NULL(result)) {
        lua_pushnil(state);
        return 1;
    }
    JsonObject* record = JSON_NODE_HOLDS_OBJECT(result) ? json_node_get_object(result) : NULL;
    const char* name = record ? member_string(record, "name", NULL) : NULL;
    if (!name || !g_str_equal(name, lua_tostring(state, 1)))
        return luaL_error(state, "gnoblin.animations.get returned an invalid AnimationInfo");
    json_to_lua(state, result);
    lua_cli_push_readonly_value(state, -1);
    lua_remove(state, -2);
    return 1;
}

static int lua_cli_animations_surfaces(lua_State* state) {
    if (lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.animations.surfaces takes no arguments");
    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    JsonObject* arguments = json_object_new();
    g_autoptr(GError) call_error = NULL;
    g_autoptr(JsonNode) result =
        call_compositor(cli, "api", "animation.surfaces", arguments, &call_error);
    json_object_unref(arguments);
    if (!result)
        return luaL_error(state, "gnoblin.animations.surfaces failed: %s", call_error->message);
    JsonObject* response = JSON_NODE_HOLDS_OBJECT(result) ? json_node_get_object(result) : NULL;
    JsonArray* surfaces = response ? json_object_get_array_member(response, "surfaces") : NULL;
    if (!surfaces)
        return luaL_error(state, "gnoblin.animations.surfaces returned an invalid snapshot");
    for (guint i = 0; i < json_array_get_length(surfaces); i++) {
        JsonObject* surface = json_array_get_object_element(surfaces, i);
        if (!member_string(surface, "id", NULL) || !*member_string(surface, "id", "") ||
            !member_string(surface, "namespace", NULL) ||
            !*member_string(surface, "namespace", "") || !member_string(surface, "title", NULL))
            return luaL_error(state,
                              "gnoblin.animations.surfaces returned an invalid Surface record");
    }

    json_to_lua(state, result);
    lua_cli_push_readonly_value(state, -1);
    lua_remove(state, -2);
    return 1;
}

static gboolean lua_cli_launch_record_valid(JsonObject* launch) {
    const char* token = member_string(launch, "token", NULL);
    const char* application = member_string(launch, "application", NULL);
    const char* state_name = member_string(launch, "state", NULL);
    static const char* const integer_fields[] = {"started_at", "timeout_ms", "revision", NULL};
    if (!token || !*token || !application || !*application || !state_name || !*state_name)
        return FALSE;
    for (guint i = 0; integer_fields[i]; i++) {
        JsonNode* field = json_object_get_member(launch, integer_fields[i]);
        if (!field || !JSON_NODE_HOLDS_VALUE(field) ||
            (json_node_get_value_type(field) != G_TYPE_INT64 &&
             json_node_get_value_type(field) != G_TYPE_INT) ||
            json_node_get_int(field) < 0 ||
            (g_str_equal(integer_fields[i], "timeout_ms") && json_node_get_int(field) == 0))
            return FALSE;
    }
    return TRUE;
}

static int lua_cli_launches_read(lua_State* state) {
    if (lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.launches reads take no arguments");
    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    gboolean return_snapshot = lua_toboolean(state, lua_upvalueindex(2));
    JsonObject* arguments = json_object_new();
    g_autoptr(GError) call_error = NULL;
    g_autoptr(JsonNode) result =
        call_compositor(cli, "api", "launches.snapshot", arguments, &call_error);
    json_object_unref(arguments);
    if (!result)
        return luaL_error(state, "gnoblin.launches read failed: %s", call_error->message);
    JsonObject* snapshot = JSON_NODE_HOLDS_OBJECT(result) ? json_node_get_object(result) : NULL;
    JsonArray* launches = snapshot ? json_object_get_array_member(snapshot, "launches") : NULL;
    JsonNode* revision = snapshot ? json_object_get_member(snapshot, "revision") : NULL;
    if (!launches || !revision || !JSON_NODE_HOLDS_VALUE(revision) ||
        (json_node_get_value_type(revision) != G_TYPE_INT64 &&
         json_node_get_value_type(revision) != G_TYPE_INT) ||
        json_node_get_int(revision) < 0)
        return luaL_error(state, "gnoblin.launches returned an invalid snapshot");
    for (guint i = 0; i < json_array_get_length(launches); i++) {
        JsonNode* value = json_array_get_element(launches, i);
        if (!JSON_NODE_HOLDS_OBJECT(value) ||
            !lua_cli_launch_record_valid(json_node_get_object(value)))
            return luaL_error(state, "gnoblin.launches returned an invalid Launch record");
    }
    if (return_snapshot) {
        json_to_lua(state, result);
        lua_cli_push_readonly_value(state, -1);
        lua_remove(state, -2);
        return 1;
    }
    lua_createtable(state, json_array_get_length(launches), 0);
    for (guint i = 0; i < json_array_get_length(launches); i++) {
        JsonNode* value = json_array_get_element(launches, i);
        json_to_lua(state, value);
        lua_cli_push_readonly_value(state, -1);
        lua_remove(state, -2);
        lua_rawseti(state, -2, i + 1);
    }
    lua_cli_push_readonly_value(state, -1);
    lua_remove(state, -2);
    return 1;
}

static void lua_cli_set_window_id(JsonObject* arguments, const char* id) {
    json_object_set_string_member(arguments, "id", id);
}

static int lua_cli_window_method(lua_State* state) {
    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    const char* method = lua_tostring(state, lua_upvalueindex(2));
    int supplied = lua_gettop(state) - 1;
    g_autofree char* id = g_strdup(lua_cli_window_id(state));
    lua_settop(state, supplied + 1);

    /* Focus contexts are minted for a particular client connection and this
     * console opens a fresh connection for each request. Do not accept a
     * caller-supplied string or fabricate a capability that cannot work. */
    if (g_str_equal(method, "window.focus") || g_str_equal(method, "window.begin_move") ||
        g_str_equal(method, "window.begin_resize"))
        return luaL_error(state,
                          "%s requires a live FocusContext from a supervised runtime callback; "
                          "gnoblinctl cannot mint or reuse one",
                          method);

    JsonObject* arguments = json_object_new();
    lua_cli_set_window_id(arguments, id);
    if (g_str_equal(method, "window.close") || g_str_equal(method, "window.minimize") ||
        g_str_equal(method, "window.toggle_minimize") || g_str_equal(method, "window.unminimize") ||
        g_str_equal(method, "window.restore") ||
        g_str_equal(method, "window.restore_or_minimize")) {
        if (supplied != 0) {
            json_object_unref(arguments);
            return luaL_error(state, "%s takes no arguments", method);
        }
    } else if (g_str_equal(method, "window.set_maximized") ||
               g_str_equal(method, "window.set_fullscreen") ||
               g_str_equal(method, "window.set_above") ||
               g_str_equal(method, "window.set_sticky")) {
        if (supplied != 1 || !lua_isboolean(state, 2)) {
            json_object_unref(arguments);
            return luaL_error(state, "%s requires one boolean", method);
        }
        json_object_set_boolean_member(arguments, "enabled", lua_toboolean(state, 2));
    } else if (g_str_equal(method, "window.move") || g_str_equal(method, "window.resize") ||
               g_str_equal(method, "window.thumbnail")) {
        if (supplied != 1 || !lua_istable(state, 2)) {
            json_object_unref(arguments);
            return luaL_error(state, "%s requires one table", method);
        }
        g_autoptr(JsonObject) values = lua_cli_table_object(state, 2, method);
        const char* first = g_str_equal(method, "window.move") ? "x" : "width";
        const char* second = g_str_equal(method, "window.move") ? "y" : "height";
        JsonNode* first_value = json_object_get_member(values, first);
        JsonNode* second_value = json_object_get_member(values, second);
        if (json_object_get_size(values) != 2 || !first_value || !second_value ||
            !JSON_NODE_HOLDS_VALUE(first_value) || !JSON_NODE_HOLDS_VALUE(second_value) ||
            (json_node_get_value_type(first_value) != G_TYPE_INT &&
             json_node_get_value_type(first_value) != G_TYPE_INT64) ||
            (json_node_get_value_type(second_value) != G_TYPE_INT &&
             json_node_get_value_type(second_value) != G_TYPE_INT64)) {
            json_object_unref(arguments);
            return luaL_error(state, "%s requires integer %s and %s", method, first, second);
        }
        json_object_set_member(arguments, first, json_node_copy(first_value));
        json_object_set_member(arguments, second, json_node_copy(second_value));
    } else if (g_str_equal(method, "window.move_to_workspace")) {
        if (supplied < 1 || supplied > 2 || !lua_istable(state, 2) ||
            (supplied == 2 && !lua_isnil(state, 3) && !lua_istable(state, 3))) {
            json_object_unref(arguments);
            return luaL_error(state,
                              "window.move_to_workspace requires a selector and optional options");
        }
        g_autoptr(JsonObject) selector = lua_cli_table_object(state, 2, "workspace selector");
        if (json_object_get_size(selector) != 1 || (!json_object_has_member(selector, "id") &&
                                                    !json_object_has_member(selector, "number"))) {
            json_object_unref(arguments);
            return luaL_error(state, "workspace selector accepts exactly id or number");
        }
        JsonNode* workspace = json_node_new(JSON_NODE_OBJECT);
        json_node_set_object(workspace, selector);
        json_object_set_member(arguments, "workspace", workspace);
        if (supplied == 2 && !lua_isnil(state, 3)) {
            g_autoptr(JsonObject) options = lua_cli_table_object(state, 3, "workspace options");
            JsonNode* follow = json_object_get_member(options, "follow");
            if (json_object_get_size(options) != 1 || !follow || !JSON_NODE_HOLDS_VALUE(follow) ||
                json_node_get_value_type(follow) != G_TYPE_BOOLEAN) {
                json_object_unref(arguments);
                return luaL_error(state, "workspace options accept only boolean follow");
            }
            json_object_set_boolean_member(arguments, "follow", json_node_get_boolean(follow));
        }
    } else if (g_str_equal(method, "window.move_to_monitor")) {
        if (supplied != 1) {
            json_object_unref(arguments);
            return luaL_error(state, "window.move_to_monitor requires a monitor selector");
        }
        g_autofree char* monitor = lua_isstring(state, 2) ? g_strdup(lua_tostring(state, 2)) : NULL;
        if (!monitor && lua_istable(state, 2)) {
            lua_getfield(state, 2, "id");
            if (lua_isstring(state, -1))
                monitor = g_strdup(lua_tostring(state, -1));
            lua_pop(state, 1);
        }
        if (!monitor || !*monitor) {
            json_object_unref(arguments);
            return luaL_error(state, "window.move_to_monitor requires a monitor ID");
        }
        json_object_set_string_member(arguments, "monitor", monitor);
    } else {
        json_object_unref(arguments);
        return luaL_error(state, "unsupported Window method %s", method);
    }

    g_autoptr(GError) call_error = NULL;
    g_autoptr(JsonNode) result = call_compositor(cli, "api", method, arguments, &call_error);
    json_object_unref(arguments);
    if (!result)
        return luaL_error(state, "%s failed: %s", method, call_error->message);
    json_to_lua(state, result);
    return 1;
}

static void lua_cli_push_window_record(lua_State* state, Cli* cli, JsonObject* object) {
    g_autoptr(JsonNode) node = json_node_new(JSON_NODE_OBJECT);
    json_node_set_object(node, object);
    json_to_lua(state, node);
    int backing = lua_absindex(state, -1);
    lua_newuserdatauv(state, 1, 2);
    int record = lua_absindex(state, -1);
    lua_pushvalue(state, backing);
    lua_setiuservalue(state, record, 1);
    lua_newtable(state);
    static const char* const methods[][2] = {
        {"close", "window.close"},
        {"minimize", "window.minimize"},
        {"toggle_minimize", "window.toggle_minimize"},
        {"unminimize", "window.unminimize"},
        {"restore", "window.restore"},
        {"restore_or_minimize", "window.restore_or_minimize"},
        {"set_maximized", "window.set_maximized"},
        {"set_fullscreen", "window.set_fullscreen"},
        {"set_above", "window.set_above"},
        {"set_sticky", "window.set_sticky"},
        {"move", "window.move"},
        {"resize", "window.resize"},
        {"move_to_workspace", "window.move_to_workspace"},
        {"move_to_monitor", "window.move_to_monitor"},
        {"focus", "window.focus"},
        {"begin_move", "window.begin_move"},
        {"begin_resize", "window.begin_resize"},
        {"thumbnail", "window.thumbnail"},
        {NULL, NULL},
    };
    for (guint i = 0; methods[i][0]; i++) {
        lua_pushlightuserdata(state, cli);
        lua_pushstring(state, methods[i][1]);
        lua_pushcclosure(state, lua_cli_window_method, 2);
        lua_setfield(state, -2, methods[i][0]);
    }
    lua_setiuservalue(state, record, 2);
    luaL_getmetatable(state, GNOBLINCTL_WINDOW_RECORD_METATABLE);
    lua_setmetatable(state, record);
    lua_remove(state, backing);
}

static int lua_cli_workspace_tostring(lua_State* state) {
    lua_getiuservalue(state, 1, 1);
    lua_getfield(state, -1, "id");
    const char* id = lua_tostring(state, -1);
    lua_pushfstring(state, "Workspace<%s>", id ? id : "unknown");
    return 1;
}

static int lua_cli_workspace_method(lua_State* state) {
    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    const char* method = lua_tostring(state, lua_upvalueindex(2));
    int supplied = lua_gettop(state) - 1;
    g_autofree char* id = g_strdup(lua_cli_workspace_id(state));
    lua_settop(state, supplied + 1);

    JsonObject* arguments = json_object_new();
    if (g_str_equal(method, "workspace.switch") || g_str_equal(method, "workspace.remove")) {
        if (supplied != 0) {
            json_object_unref(arguments);
            return luaL_error(state, "%s takes no arguments", method);
        }
        json_object_set_string_member(arguments, "id", id);
    } else if (g_str_equal(method, "workspace.rename")) {
        if (supplied != 1 || lua_type(state, 2) != LUA_TSTRING || !*lua_tostring(state, 2) ||
            strlen(lua_tostring(state, 2)) > 80) {
            json_object_unref(arguments);
            return luaL_error(state,
                              "workspace.rename requires a nonempty name up to 80 characters");
        }
        json_object_set_string_member(arguments, "id", id);
        json_object_set_string_member(arguments, "name", lua_tostring(state, 2));
    } else if (g_str_equal(method, "workspace.move_window")) {
        if (supplied < 1 || supplied > 2 ||
            (supplied == 2 && !lua_isnil(state, 3) && !lua_istable(state, 3))) {
            json_object_unref(arguments);
            return luaL_error(state,
                              "workspace.move_window requires a window and optional options");
        }
        g_autofree char* window = NULL;
        if (lua_type(state, 2) == LUA_TSTRING)
            window = g_strdup(lua_tostring(state, 2));
        else if (luaL_testudata(state, 2, GNOBLINCTL_WINDOW_RECORD_METATABLE)) {
            lua_getiuservalue(state, 2, 1);
            lua_getfield(state, -1, "id");
            if (lua_isstring(state, -1))
                window = g_strdup(lua_tostring(state, -1));
            lua_pop(state, 2);
        }
        if (!window || !*window) {
            json_object_unref(arguments);
            return luaL_error(state, "workspace.move_window requires a Window record or window ID");
        }
        json_object_set_string_member(arguments, "window", window);
        JsonObject* workspace = json_object_new();
        json_object_set_string_member(workspace, "id", id);
        JsonNode* workspace_node = json_node_new(JSON_NODE_OBJECT);
        json_node_take_object(workspace_node, workspace);
        json_object_set_member(arguments, "workspace", workspace_node);
        if (supplied == 2 && !lua_isnil(state, 3)) {
            g_autoptr(JsonObject) options = lua_cli_table_object(state, 3, "workspace options");
            JsonNode* follow = json_object_get_member(options, "follow");
            if (json_object_get_size(options) != 1 || !follow || !JSON_NODE_HOLDS_VALUE(follow) ||
                json_node_get_value_type(follow) != G_TYPE_BOOLEAN) {
                json_object_unref(arguments);
                return luaL_error(state, "workspace options accept only boolean follow");
            }
            json_object_set_boolean_member(arguments, "follow", json_node_get_boolean(follow));
        }
    } else {
        json_object_unref(arguments);
        return luaL_error(state, "unsupported Workspace method %s", method);
    }

    g_autoptr(GError) call_error = NULL;
    g_autoptr(JsonNode) result = call_compositor(cli, "api", method, arguments, &call_error);
    json_object_unref(arguments);
    if (!result)
        return luaL_error(state, "%s failed: %s", method, call_error->message);
    json_to_lua(state, result);
    return 1;
}

static void lua_cli_push_workspace_record(lua_State* state, Cli* cli, JsonObject* object) {
    g_autoptr(JsonNode) node = json_node_new(JSON_NODE_OBJECT);
    json_node_set_object(node, object);
    json_to_lua(state, node);
    int backing = lua_absindex(state, -1);
    lua_newuserdatauv(state, 1, 2);
    int record = lua_absindex(state, -1);
    lua_pushvalue(state, backing);
    lua_setiuservalue(state, record, 1);
    lua_newtable(state);
    static const char* const methods[][2] = {
        {"activate", "workspace.switch"},
        {"rename", "workspace.rename"},
        {"remove", "workspace.remove"},
        {"move_here", "workspace.move_window"},
        {NULL, NULL},
    };
    for (guint i = 0; methods[i][0]; i++) {
        lua_pushlightuserdata(state, cli);
        lua_pushstring(state, methods[i][1]);
        lua_pushcclosure(state, lua_cli_workspace_method, 2);
        lua_setfield(state, -2, methods[i][0]);
    }
    lua_setiuservalue(state, record, 2);
    luaL_getmetatable(state, GNOBLINCTL_WORKSPACE_RECORD_METATABLE);
    lua_setmetatable(state, record);
    lua_remove(state, backing);
}

static int lua_cli_monitor_tostring(lua_State* state) {
    lua_getiuservalue(state, 1, 1);
    lua_getfield(state, -1, "id");
    const char* id = lua_tostring(state, -1);
    lua_pushfstring(state, "Monitor<%s>", id ? id : "unknown");
    return 1;
}

static void lua_cli_push_monitor_record(lua_State* state, JsonObject* object) {
    const char* id = member_string(object, "id", NULL);
    if (!id || !*id)
        luaL_error(state, "gnoblin.monitors returned a monitor without a stable id");

    g_autoptr(JsonNode) node = json_node_new(JSON_NODE_OBJECT);
    json_node_set_object(node, object);
    json_to_lua(state, node);
    int backing = lua_absindex(state, -1);
    lua_newuserdatauv(state, 1, 2);
    int record = lua_absindex(state, -1);
    lua_pushvalue(state, backing);
    lua_setiuservalue(state, record, 1);
    lua_newtable(state);
    lua_setiuservalue(state, record, 2);
    luaL_getmetatable(state, GNOBLINCTL_MONITOR_RECORD_METATABLE);
    lua_setmetatable(state, record);
    lua_remove(state, backing);
}

static int lua_cli_layer_surface_tostring(lua_State* state) {
    lua_getiuservalue(state, 1, 1);
    lua_getfield(state, -1, "id");
    const char* id = lua_tostring(state, -1);
    lua_pushfstring(state, "LayerSurface<%s>", id ? id : "unknown");
    return 1;
}

static void lua_cli_push_layer_surface_record(lua_State* state, JsonObject* object) {
    const char* id = member_string(object, "id", NULL);
    if (!id || !*id)
        luaL_error(state, "gnoblin.layers.list returned a layer surface without a stable id");

    g_autoptr(JsonNode) node = json_node_new(JSON_NODE_OBJECT);
    json_node_set_object(node, object);
    json_to_lua(state, node);
    int backing = lua_absindex(state, -1);
    lua_newuserdatauv(state, 1, 2);
    int record = lua_absindex(state, -1);
    lua_pushvalue(state, backing);
    lua_setiuservalue(state, record, 1);
    lua_newtable(state);
    lua_setiuservalue(state, record, 2);
    luaL_getmetatable(state, GNOBLINCTL_LAYER_SURFACE_RECORD_METATABLE);
    lua_setmetatable(state, record);
    lua_remove(state, backing);
}

static gboolean lua_cli_input_source_valid(JsonObject* object) {
    static const char* const string_fields[] = {"id", "type", "short_name", "name"};
    for (guint i = 0; i < G_N_ELEMENTS(string_fields); i++) {
        JsonNode* field = json_object_get_member(object, string_fields[i]);
        if (!field || !JSON_NODE_HOLDS_VALUE(field) ||
            json_node_get_value_type(field) != G_TYPE_STRING)
            return FALSE;
    }
    JsonNode* current = json_object_get_member(object, "current");
    return current && JSON_NODE_HOLDS_VALUE(current) &&
           json_node_get_value_type(current) == G_TYPE_BOOLEAN &&
           *member_string(object, "id", "") &&
           (g_str_equal(member_string(object, "type", ""), "xkb") ||
            g_str_equal(member_string(object, "type", ""), "ibus"));
}

static gboolean lua_cli_input_device_valid(JsonObject* object) {
    static const char* const string_fields[] = {"id", "name", "device_type"};
    for (guint i = 0; i < G_N_ELEMENTS(string_fields); i++) {
        JsonNode* field = json_object_get_member(object, string_fields[i]);
        if (!field || !JSON_NODE_HOLDS_VALUE(field) ||
            json_node_get_value_type(field) != G_TYPE_STRING)
            return FALSE;
    }
    JsonNode* capabilities = json_object_get_member(object, "capabilities");
    if (!capabilities || !JSON_NODE_HOLDS_ARRAY(capabilities) || !*member_string(object, "id", ""))
        return FALSE;
    JsonArray* capability_array = json_node_get_array(capabilities);
    for (guint i = 0; i < json_array_get_length(capability_array); i++) {
        JsonNode* capability = json_array_get_element(capability_array, i);
        if (!capability || !JSON_NODE_HOLDS_VALUE(capability) ||
            json_node_get_value_type(capability) != G_TYPE_STRING)
            return FALSE;
    }
    return TRUE;
}

static void lua_cli_push_input_record(lua_State* state, JsonObject* object, gint64 revision,
                                      gboolean is_device) {
    gboolean valid =
        is_device ? lua_cli_input_device_valid(object) : lua_cli_input_source_valid(object);
    if (!valid)
        luaL_error(state, "input snapshot returned an invalid %s",
                   is_device ? "InputDevice" : "InputSource");

    JsonObject* public_fields = json_object_new();
    GList* members = json_object_get_members(object);
    for (GList* item = members; item; item = item->next) {
        const char* key = item->data;
        if (!g_str_equal(key, "revision"))
            json_object_set_member(public_fields, key,
                                   json_node_copy(json_object_get_member(object, key)));
    }
    g_list_free(members);
    json_object_set_int_member(public_fields, "revision", revision);
    g_autoptr(JsonNode) node = json_node_new(JSON_NODE_OBJECT);
    json_node_take_object(node, public_fields);
    json_to_lua(state, node);
    int backing = lua_absindex(state, -1);
    lua_newuserdatauv(state, 1, 2);
    int record = lua_absindex(state, -1);
    lua_pushvalue(state, backing);
    lua_setiuservalue(state, record, 1);
    lua_newtable(state);
    lua_setiuservalue(state, record, 2);
    luaL_getmetatable(state, is_device ? GNOBLINCTL_INPUT_DEVICE_RECORD_METATABLE
                                       : GNOBLINCTL_INPUT_SOURCE_RECORD_METATABLE);
    lua_setmetatable(state, record);
    lua_remove(state, backing);
}

static void lua_cli_push_shortcut_record(lua_State* state, JsonObject* object, gboolean is_action) {
    const char* identity = member_string(object, is_action ? "id" : "name", NULL);
    if (!identity || !*identity)
        luaL_error(state, "gnoblin.shortcuts.%s returned a record without a stable %s",
                   is_action ? "actions" : "list", is_action ? "id" : "name");

    g_autoptr(JsonNode) node = json_node_new(JSON_NODE_OBJECT);
    json_node_set_object(node, object);
    json_to_lua(state, node);
    int backing = lua_absindex(state, -1);
    lua_newuserdatauv(state, 1, 1);
    int record = lua_absindex(state, -1);
    lua_pushvalue(state, backing);
    lua_setiuservalue(state, record, 1);
    luaL_getmetatable(state, is_action ? GNOBLINCTL_SHORTCUT_ACTION_RECORD_METATABLE
                                       : GNOBLINCTL_SHORTCUT_STATE_RECORD_METATABLE);
    lua_setmetatable(state, record);
    lua_remove(state, backing);
}

static int lua_cli_shortcut_record_tostring(lua_State* state) {
    gboolean is_action =
        luaL_testudata(state, 1, GNOBLINCTL_SHORTCUT_ACTION_RECORD_METATABLE) != NULL;
    lua_getiuservalue(state, 1, 1);
    lua_getfield(state, -1, is_action ? "id" : "name");
    const char* identity = lua_tostring(state, -1);
    lua_pushfstring(state, "%s<%s>", is_action ? "ShortcutAction" : "ShortcutState",
                    identity ? identity : "unknown");
    return 1;
}

static int lua_cli_shortcut_snapshot(lua_State* state, gboolean is_action) {
    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    JsonObject* arguments = json_object_new();
    if (is_action && lua_gettop(state) == 1)
        json_object_set_string_member(arguments, "group", lua_tostring(state, 1));
    const char* method = is_action ? "shortcuts.actions" : "shortcuts.list";
    g_autoptr(GError) call_error = NULL;
    g_autoptr(JsonNode) result = call_compositor(cli, "api", method, arguments, &call_error);
    json_object_unref(arguments);
    if (!result)
        return luaL_error(state, "gnoblin.%s failed: %s", method, call_error->message);
    if (!JSON_NODE_HOLDS_ARRAY(result))
        return luaL_error(state, "gnoblin.%s returned an invalid snapshot", method);

    JsonArray* entries = json_node_get_array(result);
    lua_createtable(state, json_array_get_length(entries), 0);
    for (guint i = 0; i < json_array_get_length(entries); i++) {
        JsonObject* entry = json_array_get_object_element(entries, i);
        if (!entry)
            return luaL_error(state, "gnoblin.%s returned an invalid record", method);
        lua_cli_push_shortcut_record(state, entry, is_action);
        lua_rawseti(state, -2, i + 1);
    }
    return 1;
}

static int lua_cli_shortcuts_list(lua_State* state) {
    if (lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.shortcuts.list takes no arguments");
    return lua_cli_shortcut_snapshot(state, FALSE);
}

static int lua_cli_shortcuts_actions(lua_State* state) {
    if (lua_gettop(state) > 1 || (lua_gettop(state) == 1 && lua_type(state, 1) != LUA_TSTRING))
        return luaL_error(state, "gnoblin.shortcuts.actions accepts an optional group string");
    return lua_cli_shortcut_snapshot(state, TRUE);
}

static int lua_cli_shortcuts_capture(lua_State* state) {
    if (lua_gettop(state) > 1 || (lua_gettop(state) == 1 && !lua_istable(state, 1)))
        return luaL_error(state, "gnoblin.shortcuts.capture accepts one optional options table");

    gint timeout_seconds = 30;
    JsonObject* arguments = json_object_new();
    if (lua_gettop(state) == 1) {
        json_object_unref(arguments);
        arguments = lua_cli_table_object(state, 1, "shortcut capture options");
        if (json_object_get_size(arguments) > 1 ||
            (json_object_get_size(arguments) == 1 &&
             !json_object_has_member(arguments, "timeout"))) {
            json_object_unref(arguments);
            return luaL_error(state, "gnoblin.shortcuts.capture accepts only timeout");
        }
        JsonNode* timeout = json_object_get_member(arguments, "timeout");
        if (timeout) {
            if (!JSON_NODE_HOLDS_VALUE(timeout) ||
                (json_node_get_value_type(timeout) != G_TYPE_INT &&
                 json_node_get_value_type(timeout) != G_TYPE_INT64)) {
                json_object_unref(arguments);
                return luaL_error(state,
                                  "shortcut capture timeout must be an integer from 1 to 60");
            }
            timeout_seconds = (gint)json_node_get_int(timeout);
            if (timeout_seconds < 1 || timeout_seconds > 60) {
                json_object_unref(arguments);
                return luaL_error(state,
                                  "shortcut capture timeout must be an integer from 1 to 60");
            }
        }
    }

    Cli capture_cli = *(Cli*)lua_touserdata(state, lua_upvalueindex(1));
    capture_cli.timeout = (guint)timeout_seconds;
    g_autoptr(GError) call_error = NULL;
    g_autoptr(JsonNode) result =
        call_compositor(&capture_cli, "api", "shortcut.capture", arguments, &call_error);
    json_object_unref(arguments);
    if (!result)
        return luaL_error(state, "gnoblin.shortcuts.capture failed: %s", call_error->message);
    JsonObject* object = JSON_NODE_HOLDS_OBJECT(result) ? json_node_get_object(result) : NULL;
    const char* accelerator = member_string(object, "accelerator", NULL);
    if (!accelerator || !*accelerator)
        return luaL_error(state, "gnoblin.shortcuts.capture returned an invalid result");

    json_to_lua(state, result);
    return 1;
}

static void register_lua_cli_shortcut_record(lua_State* state, const char* metatable) {
    if (!luaL_newmetatable(state, metatable)) {
        lua_pop(state, 1);
        return;
    }
    lua_pushcfunction(state, lua_cli_window_index);
    lua_setfield(state, -2, "__index");
    lua_pushcfunction(state, lua_cli_readonly_newindex);
    lua_setfield(state, -2, "__newindex");
    lua_pushcfunction(state, lua_cli_window_len);
    lua_setfield(state, -2, "__len");
    lua_pushcfunction(state, lua_cli_window_pairs);
    lua_setfield(state, -2, "__pairs");
    lua_pushcfunction(state, lua_cli_shortcut_record_tostring);
    lua_setfield(state, -2, "__tostring");
    lua_pop(state, 1);
}

static gboolean lua_cli_focus_policy_valid(JsonObject* object) {
    const char* focus_mode = member_string(object, "focus_mode", NULL);
    const char* focus_new_windows = member_string(object, "focus_new_windows", NULL);
    if ((!focus_mode || (!g_str_equal(focus_mode, "click") && !g_str_equal(focus_mode, "sloppy") &&
                         !g_str_equal(focus_mode, "mouse"))) ||
        (!focus_new_windows ||
         (!g_str_equal(focus_new_windows, "strict") && !g_str_equal(focus_new_windows, "smart"))))
        return FALSE;

    static const char* const boolean_fields[] = {"raise_on_click", "auto_raise",
                                                 "focus_change_on_pointer_rest"};
    for (guint i = 0; i < G_N_ELEMENTS(boolean_fields); i++) {
        JsonNode* field = json_object_get_member(object, boolean_fields[i]);
        if (!field || !JSON_NODE_HOLDS_VALUE(field) ||
            json_node_get_value_type(field) != G_TYPE_BOOLEAN)
            return FALSE;
    }
    const char* const integer_fields[] = {"auto_raise_delay", "revision"};
    for (guint i = 0; i < G_N_ELEMENTS(integer_fields); i++) {
        JsonNode* field = json_object_get_member(object, integer_fields[i]);
        if (!field || !JSON_NODE_HOLDS_VALUE(field) ||
            (json_node_get_value_type(field) != G_TYPE_INT64 &&
             json_node_get_value_type(field) != G_TYPE_INT))
            return FALSE;
    }
    gint64 delay = json_object_get_int_member(object, "auto_raise_delay");
    gint64 revision = json_object_get_int_member(object, "revision");
    return delay >= 0 && delay <= 10000 && revision >= 0;
}

static int lua_cli_focus_policy_tostring(lua_State* state) {
    lua_pushliteral(state, "FocusPolicy");
    return 1;
}

static void register_lua_cli_focus_policy_record(lua_State* state) {
    if (!luaL_newmetatable(state, GNOBLINCTL_FOCUS_POLICY_RECORD_METATABLE)) {
        lua_pop(state, 1);
        return;
    }
    lua_pushcfunction(state, lua_cli_readonly_index);
    lua_setfield(state, -2, "__index");
    lua_pushcfunction(state, lua_cli_readonly_newindex);
    lua_setfield(state, -2, "__newindex");
    lua_pushcfunction(state, lua_cli_window_len);
    lua_setfield(state, -2, "__len");
    lua_pushcfunction(state, lua_cli_window_pairs);
    lua_setfield(state, -2, "__pairs");
    lua_pushcfunction(state, lua_cli_focus_policy_tostring);
    lua_setfield(state, -2, "__tostring");
    lua_pop(state, 1);
}

static int lua_cli_settings_tostring(lua_State* state) {
    lua_pushliteral(state, "Settings");
    return 1;
}

static void register_lua_cli_settings_record(lua_State* state) {
    if (!luaL_newmetatable(state, GNOBLINCTL_SETTINGS_RECORD_METATABLE)) {
        lua_pop(state, 1);
        return;
    }
    lua_pushcfunction(state, lua_cli_readonly_index);
    lua_setfield(state, -2, "__index");
    lua_pushcfunction(state, lua_cli_readonly_newindex);
    lua_setfield(state, -2, "__newindex");
    lua_pushcfunction(state, lua_cli_window_len);
    lua_setfield(state, -2, "__len");
    lua_pushcfunction(state, lua_cli_window_pairs);
    lua_setfield(state, -2, "__pairs");
    lua_pushcfunction(state, lua_cli_settings_tostring);
    lua_setfield(state, -2, "__tostring");
    lua_pop(state, 1);
}

static int lua_cli_settings_property(lua_State* state) {
    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    JsonObject* arguments = json_object_new();
    g_autoptr(GError) call_error = NULL;
    g_autoptr(JsonNode) result = call_compositor(cli, "api", "settings", arguments, &call_error);
    json_object_unref(arguments);
    if (!result)
        return luaL_error(state, "gnoblin.settings failed: %s", call_error->message);
    if (!JSON_NODE_HOLDS_OBJECT(result))
        return luaL_error(state, "gnoblin.settings returned an invalid snapshot");

    JsonObject* object = json_node_get_object(result);
    JsonNode* revision = json_object_get_member(object, "revision");
    if (!revision || !JSON_NODE_HOLDS_VALUE(revision) ||
        (json_node_get_value_type(revision) != G_TYPE_INT64 &&
         json_node_get_value_type(revision) != G_TYPE_INT) ||
        json_node_get_int(revision) < 0)
        return luaL_error(state, "gnoblin.settings returned an invalid Settings revision");

    json_to_lua(state, result);
    int backing = lua_absindex(state, -1);
    lua_newuserdatauv(state, 1, 1);
    int record = lua_absindex(state, -1);
    lua_pushvalue(state, backing);
    lua_setiuservalue(state, record, 1);
    luaL_getmetatable(state, GNOBLINCTL_SETTINGS_RECORD_METATABLE);
    lua_setmetatable(state, record);
    lua_remove(state, backing);
    return 1;
}

static gboolean lua_cli_layer_animation_policy_valid(JsonObject* object,
                                                     const char* requested_namespace) {
    if (!g_str_equal(member_string(object, "namespace", ""), requested_namespace))
        return FALSE;
    JsonNode* window_shadow = json_object_get_member(object, "window_shadow");
    JsonNode* revision = json_object_get_member(object, "revision");
    if (!window_shadow || !revision || !JSON_NODE_HOLDS_VALUE(revision) ||
        (json_node_get_value_type(revision) != G_TYPE_INT64 &&
         json_node_get_value_type(revision) != G_TYPE_INT) ||
        json_node_get_int(revision) < 0)
        return FALSE;

    static const char* const phase_names[] = {"enter", "exit"};
    for (guint i = 0; i < G_N_ELEMENTS(phase_names); i++) {
        JsonObject* phase = member_object(object, phase_names[i]);
        if (!phase || !*member_string(phase, "animation", ""))
            return FALSE;
        JsonNode* duration = json_object_get_member(phase, "duration");
        if (duration && (!JSON_NODE_HOLDS_VALUE(duration) ||
                         (json_node_get_value_type(duration) != G_TYPE_INT64 &&
                          json_node_get_value_type(duration) != G_TYPE_INT)))
            return FALSE;
        JsonNode* easing = json_object_get_member(phase, "easing");
        if (easing && !JSON_NODE_HOLDS_VALUE(easing) && !JSON_NODE_HOLDS_OBJECT(easing))
            return FALSE;
    }
    return TRUE;
}

static int lua_cli_layer_animation_policy_tostring(lua_State* state) {
    lua_getiuservalue(state, 1, 1);
    lua_getfield(state, -1, "namespace");
    const char* namespace = lua_tostring(state, -1);
    lua_pushfstring(state, "LayerAnimationPolicy<%s>", namespace ? namespace : "unknown");
    return 1;
}

static void register_lua_cli_layer_animation_policy_record(lua_State* state) {
    if (!luaL_newmetatable(state, GNOBLINCTL_LAYER_ANIMATION_POLICY_RECORD_METATABLE)) {
        lua_pop(state, 1);
        return;
    }
    lua_pushcfunction(state, lua_cli_readonly_index);
    lua_setfield(state, -2, "__index");
    lua_pushcfunction(state, lua_cli_readonly_newindex);
    lua_setfield(state, -2, "__newindex");
    lua_pushcfunction(state, lua_cli_window_len);
    lua_setfield(state, -2, "__len");
    lua_pushcfunction(state, lua_cli_window_pairs);
    lua_setfield(state, -2, "__pairs");
    lua_pushcfunction(state, lua_cli_layer_animation_policy_tostring);
    lua_setfield(state, -2, "__tostring");
    lua_pop(state, 1);
}

static int lua_cli_layer_animation_policy(lua_State* state) {
    if (lua_gettop(state) != 1 || lua_type(state, 1) != LUA_TSTRING)
        return luaL_error(state, "gnoblin.layers.animation_policy requires one namespace string");
    const char* namespace = lua_tostring(state, 1);
    gsize length = strlen(namespace);
    if (length == 0 || length > 128 || !g_utf8_validate(namespace, length, NULL))
        return luaL_error(state, "layer namespace must be 1 to 128 bytes of UTF-8");

    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    JsonObject* arguments = json_object_new();
    json_object_set_string_member(arguments, "namespace", namespace);
    g_autoptr(GError) call_error = NULL;
    g_autoptr(JsonNode) result =
        call_compositor(cli, "api", "layer.animation_policy", arguments, &call_error);
    json_object_unref(arguments);
    if (!result)
        return luaL_error(state, "gnoblin.layers.animation_policy failed: %s", call_error->message);
    if (!JSON_NODE_HOLDS_OBJECT(result) ||
        !lua_cli_layer_animation_policy_valid(json_node_get_object(result), namespace))
        return luaL_error(state, "gnoblin.layers.animation_policy returned an invalid snapshot");

    json_to_lua(state, result);
    int backing = lua_absindex(state, -1);
    lua_newuserdatauv(state, 1, 1);
    int record = lua_absindex(state, -1);
    lua_pushvalue(state, backing);
    lua_setiuservalue(state, record, 1);
    luaL_getmetatable(state, GNOBLINCTL_LAYER_ANIMATION_POLICY_RECORD_METATABLE);
    lua_setmetatable(state, record);
    lua_remove(state, backing);
    return 1;
}

static gboolean lua_cli_privacy_state_valid(JsonObject* object) {
    JsonObject* available = member_object(object, "available");
    JsonNode* revision = json_object_get_member(object, "revision");
    if (!available || !revision || !JSON_NODE_HOLDS_VALUE(revision) ||
        (json_node_get_value_type(revision) != G_TYPE_INT64 &&
         json_node_get_value_type(revision) != G_TYPE_INT) ||
        json_node_get_int(revision) < 0)
        return FALSE;

    GList* available_fields = json_object_get_members(available);
    for (GList* item = available_fields; item; item = item->next) {
        const char* key = item->data;
        JsonNode* value = json_object_get_member(available, key);
        if (!value || !JSON_NODE_HOLDS_VALUE(value) ||
            json_node_get_value_type(value) != G_TYPE_BOOLEAN) {
            g_list_free(available_fields);
            return FALSE;
        }
    }
    g_list_free(available_fields);

    GList* fields = json_object_get_members(object);
    for (GList* item = fields; item; item = item->next) {
        const char* key = item->data;
        if (g_str_equal(key, "available") || g_str_equal(key, "revision"))
            continue;
        JsonNode* value = json_object_get_member(object, key);
        JsonNode* source_available = json_object_get_member(available, key);
        if (!value || !JSON_NODE_HOLDS_VALUE(value) ||
            json_node_get_value_type(value) != G_TYPE_BOOLEAN || !source_available ||
            !json_node_get_boolean(source_available)) {
            g_list_free(fields);
            return FALSE;
        }
    }
    g_list_free(fields);

    available_fields = json_object_get_members(available);
    for (GList* item = available_fields; item; item = item->next) {
        const char* key = item->data;
        JsonNode* source_available = json_object_get_member(available, key);
        if (json_node_get_boolean(source_available) && !json_object_has_member(object, key)) {
            g_list_free(available_fields);
            return FALSE;
        }
    }
    g_list_free(available_fields);
    return TRUE;
}

static int lua_cli_privacy_state_tostring(lua_State* state) {
    lua_pushliteral(state, "PrivacyState");
    return 1;
}

static void register_lua_cli_privacy_state_record(lua_State* state) {
    if (!luaL_newmetatable(state, GNOBLINCTL_PRIVACY_STATE_RECORD_METATABLE)) {
        lua_pop(state, 1);
        return;
    }
    lua_pushcfunction(state, lua_cli_readonly_index);
    lua_setfield(state, -2, "__index");
    lua_pushcfunction(state, lua_cli_readonly_newindex);
    lua_setfield(state, -2, "__newindex");
    lua_pushcfunction(state, lua_cli_window_len);
    lua_setfield(state, -2, "__len");
    lua_pushcfunction(state, lua_cli_window_pairs);
    lua_setfield(state, -2, "__pairs");
    lua_pushcfunction(state, lua_cli_privacy_state_tostring);
    lua_setfield(state, -2, "__tostring");
    lua_pop(state, 1);
}

static int lua_cli_privacy_state(lua_State* state) {
    if (lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.privacy.state takes no arguments");
    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    JsonObject* arguments = json_object_new();
    g_autoptr(GError) call_error = NULL;
    g_autoptr(JsonNode) result =
        call_compositor(cli, "api", "privacy.state", arguments, &call_error);
    json_object_unref(arguments);
    if (!result)
        return luaL_error(state, "gnoblin.privacy.state failed: %s", call_error->message);
    if (!JSON_NODE_HOLDS_OBJECT(result) ||
        !lua_cli_privacy_state_valid(json_node_get_object(result)))
        return luaL_error(state, "gnoblin.privacy.state returned an invalid PrivacyState");

    json_to_lua(state, result);
    int backing = lua_absindex(state, -1);
    lua_newuserdatauv(state, 1, 1);
    int record = lua_absindex(state, -1);
    lua_pushvalue(state, backing);
    lua_setiuservalue(state, record, 1);
    luaL_getmetatable(state, GNOBLINCTL_PRIVACY_STATE_RECORD_METATABLE);
    lua_setmetatable(state, record);
    lua_remove(state, backing);
    return 1;
}

static gboolean lua_cli_capability_valid(JsonObject* object) {
    const char* id = member_string(object, "id", NULL);
    const char* description = member_string(object, "description", NULL);
    JsonNode* available = json_object_get_member(object, "available");
    JsonNode* revision = json_object_get_member(object, "revision");
    JsonNode* reason = json_object_get_member(object, "reason");
    return id && *id && description && *description && available &&
           JSON_NODE_HOLDS_VALUE(available) &&
           json_node_get_value_type(available) == G_TYPE_BOOLEAN && revision &&
           JSON_NODE_HOLDS_VALUE(revision) &&
           (json_node_get_value_type(revision) == G_TYPE_INT64 ||
            json_node_get_value_type(revision) == G_TYPE_INT) &&
           json_node_get_int(revision) >= 0 &&
           (!reason ||
            (JSON_NODE_HOLDS_VALUE(reason) && json_node_get_value_type(reason) == G_TYPE_STRING &&
             *json_node_get_string(reason)));
}

static int lua_cli_capability_tostring(lua_State* state) {
    lua_getiuservalue(state, 1, 1);
    lua_getfield(state, -1, "id");
    const char* id = lua_tostring(state, -1);
    lua_pushfstring(state, "Capability<%s>", id ? id : "unknown");
    return 1;
}

static void register_lua_cli_capability_record(lua_State* state) {
    if (!luaL_newmetatable(state, GNOBLINCTL_CAPABILITY_RECORD_METATABLE)) {
        lua_pop(state, 1);
        return;
    }
    lua_pushcfunction(state, lua_cli_readonly_index);
    lua_setfield(state, -2, "__index");
    lua_pushcfunction(state, lua_cli_readonly_newindex);
    lua_setfield(state, -2, "__newindex");
    lua_pushcfunction(state, lua_cli_window_pairs);
    lua_setfield(state, -2, "__pairs");
    lua_pushcfunction(state, lua_cli_capability_tostring);
    lua_setfield(state, -2, "__tostring");
    lua_pop(state, 1);
}

static void lua_cli_push_capability_record(lua_State* state, JsonNode* node) {
    json_to_lua(state, node);
    int backing = lua_absindex(state, -1);
    lua_newuserdatauv(state, 1, 1);
    int record = lua_absindex(state, -1);
    lua_pushvalue(state, backing);
    lua_setiuservalue(state, record, 1);
    luaL_getmetatable(state, GNOBLINCTL_CAPABILITY_RECORD_METATABLE);
    lua_setmetatable(state, record);
    lua_remove(state, backing);
}

static int lua_cli_capabilities_list(lua_State* state) {
    if (lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.capabilities.list takes no arguments");
    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    JsonObject* arguments = json_object_new();
    g_autoptr(GError) call_error = NULL;
    g_autoptr(JsonNode) result =
        call_compositor(cli, "api", "capabilities.list", arguments, &call_error);
    json_object_unref(arguments);
    if (!result)
        return luaL_error(state, "gnoblin.capabilities.list failed: %s", call_error->message);
    if (!JSON_NODE_HOLDS_ARRAY(result))
        return luaL_error(state, "gnoblin.capabilities.list returned an invalid snapshot");

    JsonArray* capabilities = json_node_get_array(result);
    guint length = json_array_get_length(capabilities);
    lua_createtable(state, length, 0);
    for (guint i = 0; i < length; i++) {
        JsonNode* node = json_array_get_element(capabilities, i);
        if (!JSON_NODE_HOLDS_OBJECT(node) || !lua_cli_capability_valid(json_node_get_object(node)))
            return luaL_error(state,
                              "gnoblin.capabilities.list returned an invalid Capability record");
        lua_cli_push_capability_record(state, node);
        lua_rawseti(state, -2, i + 1);
    }
    return 1;
}

static gboolean lua_cli_permission_policy_valid(JsonObject* object) {
    const char* default_level = member_string(object, "default", NULL);
    JsonArray* rules = json_object_get_array_member(object, "rules");
    JsonNode* revision = json_object_get_member(object, "revision");
    if (!default_level ||
        (!g_str_equal(default_level, "default") && !g_str_equal(default_level, "ask") &&
         !g_str_equal(default_level, "deny")) ||
        !rules || !revision || !JSON_NODE_HOLDS_VALUE(revision) ||
        (json_node_get_value_type(revision) != G_TYPE_INT64 &&
         json_node_get_value_type(revision) != G_TYPE_INT) ||
        json_node_get_int(revision) < 0)
        return FALSE;
    for (guint i = 0; i < json_array_get_length(rules); i++)
        if (!JSON_NODE_HOLDS_OBJECT(json_array_get_element(rules, i)))
            return FALSE;
    return TRUE;
}

static int lua_cli_permission_policy_tostring(lua_State* state) {
    lua_pushliteral(state, "PermissionPolicy");
    return 1;
}

static void register_lua_cli_permission_policy_record(lua_State* state) {
    if (!luaL_newmetatable(state, GNOBLINCTL_PERMISSION_POLICY_RECORD_METATABLE)) {
        lua_pop(state, 1);
        return;
    }
    lua_pushcfunction(state, lua_cli_readonly_index);
    lua_setfield(state, -2, "__index");
    lua_pushcfunction(state, lua_cli_readonly_newindex);
    lua_setfield(state, -2, "__newindex");
    lua_pushcfunction(state, lua_cli_window_pairs);
    lua_setfield(state, -2, "__pairs");
    lua_pushcfunction(state, lua_cli_permission_policy_tostring);
    lua_setfield(state, -2, "__tostring");
    lua_pop(state, 1);
}

static gboolean lua_cli_permission_string_array_valid(JsonArray* array) {
    if (!array)
        return FALSE;
    for (guint i = 0; i < json_array_get_length(array); i++) {
        JsonNode* value = json_array_get_element(array, i);
        if (!value || !JSON_NODE_HOLDS_VALUE(value) ||
            json_node_get_value_type(value) != G_TYPE_STRING || !json_node_get_string(value)[0])
            return FALSE;
    }
    return TRUE;
}

static int lua_cli_permissions_list(lua_State* state) {
    if (lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.permissions.list takes no arguments");
    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    JsonObject* arguments = json_object_new();
    g_autoptr(GError) call_error = NULL;
    g_autoptr(JsonNode) result =
        call_compositor(cli, "api", "permissions.list", arguments, &call_error);
    json_object_unref(arguments);
    if (!result)
        return luaL_error(state, "gnoblin.permissions.list failed: %s", call_error->message);
    JsonObject* object = JSON_NODE_HOLDS_OBJECT(result) ? json_node_get_object(result) : NULL;
    JsonObject* policy = member_object(object, "policy");
    const char* default_level = member_string(policy, "default", NULL);
    JsonNode* path = object ? json_object_get_member(object, "path") : NULL;
    if (!policy || !default_level ||
        (!g_str_equal(default_level, "default") && !g_str_equal(default_level, "ask") &&
         !g_str_equal(default_level, "deny")) ||
        !json_object_get_array_member(policy, "rules") ||
        !lua_cli_permission_string_array_valid(
            object ? json_object_get_array_member(object, "capabilities") : NULL) ||
        !lua_cli_permission_string_array_valid(
            object ? json_object_get_array_member(object, "levels") : NULL) ||
        !path || !JSON_NODE_HOLDS_VALUE(path) || json_node_get_value_type(path) != G_TYPE_STRING)
        return luaL_error(state, "gnoblin.permissions.list returned an invalid snapshot");

    json_to_lua(state, result);
    lua_cli_push_readonly_value(state, -1);
    lua_remove(state, -2);
    return 1;
}

static int lua_cli_version(lua_State* state) {
    if (lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.version takes no arguments");
    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    JsonObject* arguments = json_object_new();
    g_autoptr(GError) call_error = NULL;
    g_autoptr(JsonNode) result = call_compositor(cli, "api", "version", arguments, &call_error);
    json_object_unref(arguments);
    if (!result)
        return luaL_error(state, "gnoblin.version failed: %s", call_error->message);
    JsonObject* object = JSON_NODE_HOLDS_OBJECT(result) ? json_node_get_object(result) : NULL;
    static const char* const fields[] = {"gnoblin",    "gnome",   "mutter",   "lua", "api",
                                         "git_remote", "git_sha", "build_id", NULL};
    for (guint i = 0; fields[i]; i++)
        if (!member_string(object, fields[i], NULL))
            return luaL_error(state, "gnoblin.version returned an invalid Version record");
    json_to_lua(state, result);
    lua_cli_push_readonly_value(state, -1);
    lua_remove(state, -2);
    return 1;
}

static int lua_cli_permissions_policy(lua_State* state) {
    if (lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.permissions.policy takes no arguments");
    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    JsonObject* arguments = json_object_new();
    g_autoptr(GError) call_error = NULL;
    g_autoptr(JsonNode) result =
        call_compositor(cli, "api", "permissions.policy", arguments, &call_error);
    json_object_unref(arguments);
    if (!result)
        return luaL_error(state, "gnoblin.permissions.policy failed: %s", call_error->message);
    if (!JSON_NODE_HOLDS_OBJECT(result) ||
        !lua_cli_permission_policy_valid(json_node_get_object(result)))
        return luaL_error(state, "gnoblin.permissions.policy returned an invalid PermissionPolicy");

    json_to_lua(state, result);
    int backing = lua_absindex(state, -1);
    lua_newuserdatauv(state, 1, 1);
    int record = lua_absindex(state, -1);
    lua_pushvalue(state, backing);
    lua_setiuservalue(state, record, 1);
    luaL_getmetatable(state, GNOBLINCTL_PERMISSION_POLICY_RECORD_METATABLE);
    lua_setmetatable(state, record);
    lua_remove(state, backing);
    return 1;
}

static gboolean lua_cli_permission_decision_valid(JsonObject* object) {
    const char* level = member_string(object, "level", NULL);
    JsonNode* rule = json_object_get_member(object, "rule");
    JsonArray* monitors = json_object_get_array_member(object, "monitors");
    JsonArray* devices = json_object_get_array_member(object, "devices");
    JsonNode* clipboard = json_object_get_member(object, "clipboard");
    JsonNode* revision = json_object_get_member(object, "revision");
    if (!level ||
        (!g_str_equal(level, "default") && !g_str_equal(level, "ask") &&
         !g_str_equal(level, "allow") && !g_str_equal(level, "deny")) ||
        !rule || !JSON_NODE_HOLDS_VALUE(rule) || json_node_get_value_type(rule) != G_TYPE_STRING ||
        !monitors || !devices || !clipboard || !JSON_NODE_HOLDS_VALUE(clipboard) ||
        json_node_get_value_type(clipboard) != G_TYPE_BOOLEAN || !revision ||
        !JSON_NODE_HOLDS_VALUE(revision) ||
        (json_node_get_value_type(revision) != G_TYPE_INT64 &&
         json_node_get_value_type(revision) != G_TYPE_INT) ||
        json_node_get_int(revision) < 0)
        return FALSE;
    for (guint i = 0; i < json_array_get_length(monitors); i++) {
        JsonNode* monitor = json_array_get_element(monitors, i);
        if (!monitor || !JSON_NODE_HOLDS_VALUE(monitor) ||
            json_node_get_value_type(monitor) != G_TYPE_STRING)
            return FALSE;
    }
    for (guint i = 0; i < json_array_get_length(devices); i++) {
        JsonNode* device = json_array_get_element(devices, i);
        const char* name = device && JSON_NODE_HOLDS_VALUE(device) &&
                                   json_node_get_value_type(device) == G_TYPE_STRING
                               ? json_node_get_string(device)
                               : NULL;
        if (!name || (!g_str_equal(name, "keyboard") && !g_str_equal(name, "pointer") &&
                      !g_str_equal(name, "touchscreen")))
            return FALSE;
    }
    return TRUE;
}

static int lua_cli_permission_decision_tostring(lua_State* state) {
    lua_pushliteral(state, "PermissionDecision");
    return 1;
}

static void register_lua_cli_permission_decision_record(lua_State* state) {
    if (!luaL_newmetatable(state, GNOBLINCTL_PERMISSION_DECISION_RECORD_METATABLE)) {
        lua_pop(state, 1);
        return;
    }
    lua_pushcfunction(state, lua_cli_readonly_index);
    lua_setfield(state, -2, "__index");
    lua_pushcfunction(state, lua_cli_readonly_newindex);
    lua_setfield(state, -2, "__newindex");
    lua_pushcfunction(state, lua_cli_window_pairs);
    lua_setfield(state, -2, "__pairs");
    lua_pushcfunction(state, lua_cli_permission_decision_tostring);
    lua_setfield(state, -2, "__tostring");
    lua_pop(state, 1);
}

static int lua_cli_permissions_check(lua_State* state) {
    JsonObject* arguments = json_object_new();
    if (lua_gettop(state) == 2 && lua_type(state, 1) == LUA_TSTRING &&
        lua_type(state, 2) == LUA_TSTRING) {
        json_object_set_string_member(arguments, "capability", lua_tostring(state, 1));
        json_object_set_string_member(arguments, "identity", lua_tostring(state, 2));
    } else if (lua_gettop(state) == 1 && lua_istable(state, 1)) {
        lua_pushnil(state);
        while (lua_next(state, 1)) {
            const char* key = lua_type(state, -2) == LUA_TSTRING ? lua_tostring(state, -2) : NULL;
            if (!key || (!g_str_equal(key, "capability") && !g_str_equal(key, "identity")) ||
                lua_type(state, -1) != LUA_TSTRING) {
                json_object_unref(arguments);
                return luaL_error(
                    state, "gnoblin.permissions.check accepts string capability and identity");
            }
            lua_pop(state, 1);
        }
        lua_getfield(state, 1, "capability");
        const char* capability =
            lua_type(state, -1) == LUA_TSTRING ? lua_tostring(state, -1) : NULL;
        lua_getfield(state, 1, "identity");
        const char* identity = lua_type(state, -1) == LUA_TSTRING ? lua_tostring(state, -1) : NULL;
        if (!capability || !identity) {
            lua_pop(state, 2);
            json_object_unref(arguments);
            return luaL_error(state,
                              "gnoblin.permissions.check requires string capability and identity");
        }
        json_object_set_string_member(arguments, "capability", capability);
        json_object_set_string_member(arguments, "identity", identity);
        lua_pop(state, 2);
    } else {
        json_object_unref(arguments);
        return luaL_error(state,
                          "gnoblin.permissions.check accepts (capability, identity) or one table");
    }

    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    g_autoptr(GError) call_error = NULL;
    g_autoptr(JsonNode) result =
        call_compositor(cli, "api", "permissions.check", arguments, &call_error);
    json_object_unref(arguments);
    if (!result)
        return luaL_error(state, "gnoblin.permissions.check failed: %s", call_error->message);
    if (!JSON_NODE_HOLDS_OBJECT(result) ||
        !lua_cli_permission_decision_valid(json_node_get_object(result)))
        return luaL_error(state,
                          "gnoblin.permissions.check returned an invalid PermissionDecision");

    json_to_lua(state, result);
    int backing = lua_absindex(state, -1);
    lua_newuserdatauv(state, 1, 1);
    int record = lua_absindex(state, -1);
    lua_pushvalue(state, backing);
    lua_setiuservalue(state, record, 1);
    luaL_getmetatable(state, GNOBLINCTL_PERMISSION_DECISION_RECORD_METATABLE);
    lua_setmetatable(state, record);
    lua_remove(state, backing);
    return 1;
}

static gboolean lua_cli_session_status_valid(JsonObject* object) {
    const char* session_state = member_string(object, "state", NULL);
    JsonNode* lock_available = json_object_get_member(object, "lock_available");
    if (!session_state || !g_str_equal(session_state, "running") || !lock_available ||
        !JSON_NODE_HOLDS_VALUE(lock_available) ||
        json_node_get_value_type(lock_available) != G_TYPE_BOOLEAN)
        return FALSE;
    JsonNode* lock_state_node = json_object_get_member(object, "lock_state");
    if (!json_node_get_boolean(lock_available))
        return lock_state_node == NULL;
    if (!lock_state_node || !JSON_NODE_HOLDS_VALUE(lock_state_node) ||
        json_node_get_value_type(lock_state_node) != G_TYPE_STRING)
        return FALSE;
    const char* lock_state = json_node_get_string(lock_state_node);
    return g_str_equal(lock_state, "unlocked") || g_str_equal(lock_state, "covering") ||
           g_str_equal(lock_state, "locked") || g_str_equal(lock_state, "failsafe");
}

static int lua_cli_session_status_tostring(lua_State* state) {
    lua_pushliteral(state, "SessionStatus");
    return 1;
}

static void register_lua_cli_session_status_record(lua_State* state) {
    if (!luaL_newmetatable(state, GNOBLINCTL_SESSION_STATUS_RECORD_METATABLE)) {
        lua_pop(state, 1);
        return;
    }
    lua_pushcfunction(state, lua_cli_readonly_index);
    lua_setfield(state, -2, "__index");
    lua_pushcfunction(state, lua_cli_readonly_newindex);
    lua_setfield(state, -2, "__newindex");
    lua_pushcfunction(state, lua_cli_window_pairs);
    lua_setfield(state, -2, "__pairs");
    lua_pushcfunction(state, lua_cli_session_status_tostring);
    lua_setfield(state, -2, "__tostring");
    lua_pop(state, 1);
}

static int lua_cli_session_status(lua_State* state) {
    if (lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.session.status takes no arguments");
    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    JsonObject* arguments = json_object_new();
    g_autoptr(GError) call_error = NULL;
    g_autoptr(JsonNode) result =
        call_compositor(cli, "api", "session.status", arguments, &call_error);
    json_object_unref(arguments);
    if (!result)
        return luaL_error(state, "gnoblin.session.status failed: %s", call_error->message);
    if (!JSON_NODE_HOLDS_OBJECT(result) ||
        !lua_cli_session_status_valid(json_node_get_object(result)))
        return luaL_error(state, "gnoblin.session.status returned an invalid SessionStatus");

    json_to_lua(state, result);
    int backing = lua_absindex(state, -1);
    lua_newuserdatauv(state, 1, 1);
    int record = lua_absindex(state, -1);
    lua_pushvalue(state, backing);
    lua_setiuservalue(state, record, 1);
    luaL_getmetatable(state, GNOBLINCTL_SESSION_STATUS_RECORD_METATABLE);
    lua_setmetatable(state, record);
    lua_remove(state, backing);
    return 1;
}

static gboolean lua_cli_session_activity_valid(JsonObject* object) {
    static const char* const boolean_fields[] = {"available", "idle"};
    static const char* const integer_fields[] = {"threshold_ms", "idle_for_ms", "revision"};
    for (guint i = 0; i < G_N_ELEMENTS(boolean_fields); i++) {
        JsonNode* field = json_object_get_member(object, boolean_fields[i]);
        if (!field || !JSON_NODE_HOLDS_VALUE(field) ||
            json_node_get_value_type(field) != G_TYPE_BOOLEAN)
            return FALSE;
    }
    for (guint i = 0; i < G_N_ELEMENTS(integer_fields); i++) {
        JsonNode* field = json_object_get_member(object, integer_fields[i]);
        if (!field || !JSON_NODE_HOLDS_VALUE(field) ||
            (json_node_get_value_type(field) != G_TYPE_INT64 &&
             json_node_get_value_type(field) != G_TYPE_INT) ||
            json_node_get_int(field) < 0)
            return FALSE;
    }
    return TRUE;
}

static int lua_cli_session_activity_tostring(lua_State* state) {
    lua_pushliteral(state, "SessionActivity");
    return 1;
}

static void register_lua_cli_session_activity_record(lua_State* state) {
    if (!luaL_newmetatable(state, GNOBLINCTL_SESSION_ACTIVITY_RECORD_METATABLE)) {
        lua_pop(state, 1);
        return;
    }
    lua_pushcfunction(state, lua_cli_readonly_index);
    lua_setfield(state, -2, "__index");
    lua_pushcfunction(state, lua_cli_readonly_newindex);
    lua_setfield(state, -2, "__newindex");
    lua_pushcfunction(state, lua_cli_window_pairs);
    lua_setfield(state, -2, "__pairs");
    lua_pushcfunction(state, lua_cli_session_activity_tostring);
    lua_setfield(state, -2, "__tostring");
    lua_pop(state, 1);
}

static int lua_cli_session_activity(lua_State* state) {
    if (lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.session.activity takes no arguments");
    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    JsonObject* arguments = json_object_new();
    g_autoptr(GError) call_error = NULL;
    g_autoptr(JsonNode) result =
        call_compositor(cli, "api", "session.activity", arguments, &call_error);
    json_object_unref(arguments);
    if (!result)
        return luaL_error(state, "gnoblin.session.activity failed: %s", call_error->message);
    if (!JSON_NODE_HOLDS_OBJECT(result) ||
        !lua_cli_session_activity_valid(json_node_get_object(result)))
        return luaL_error(state, "gnoblin.session.activity returned an invalid SessionActivity");

    json_to_lua(state, result);
    int backing = lua_absindex(state, -1);
    lua_newuserdatauv(state, 1, 1);
    int record = lua_absindex(state, -1);
    lua_pushvalue(state, backing);
    lua_setiuservalue(state, record, 1);
    luaL_getmetatable(state, GNOBLINCTL_SESSION_ACTIVITY_RECORD_METATABLE);
    lua_setmetatable(state, record);
    lua_remove(state, backing);
    return 1;
}

static int lua_cli_focus_policy_property(lua_State* state) {
    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    JsonObject* arguments = json_object_new();
    g_autoptr(GError) call_error = NULL;
    g_autoptr(JsonNode) result =
        call_compositor(cli, "api", "focus.policy", arguments, &call_error);
    json_object_unref(arguments);
    if (!result)
        return luaL_error(state, "gnoblin.focus.policy failed: %s", call_error->message);
    if (!JSON_NODE_HOLDS_OBJECT(result))
        return luaL_error(state, "gnoblin.focus.policy returned an invalid snapshot");
    JsonObject* object = json_node_get_object(result);
    if (!lua_cli_focus_policy_valid(object))
        return luaL_error(state, "gnoblin.focus.policy returned an invalid FocusPolicy");

    json_to_lua(state, result);
    int backing = lua_absindex(state, -1);
    lua_newuserdatauv(state, 1, 1);
    int record = lua_absindex(state, -1);
    lua_pushvalue(state, backing);
    lua_setiuservalue(state, record, 1);
    luaL_getmetatable(state, GNOBLINCTL_FOCUS_POLICY_RECORD_METATABLE);
    lua_setmetatable(state, record);
    lua_remove(state, backing);
    return 1;
}

static int lua_cli_input_record_tostring(lua_State* state) {
    lua_getiuservalue(state, 1, 1);
    lua_getfield(state, -1, "id");
    const char* id = lua_tostring(state, -1);
    const char* type = luaL_testudata(state, 1, GNOBLINCTL_INPUT_DEVICE_RECORD_METATABLE)
                           ? "InputDevice"
                           : "InputSource";
    lua_pushfstring(state, "%s<%s>", type, id ? id : "unknown");
    return 1;
}

static int lua_cli_input_snapshot(lua_State* state) {
    if (lua_gettop(state) != 0)
        return luaL_error(state, "%s takes no arguments", lua_tostring(state, lua_upvalueindex(2)));
    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    const char* method = lua_tostring(state, lua_upvalueindex(2));
    g_autoptr(GError) call_error = NULL;
    JsonObject* arguments = json_object_new();
    g_autoptr(JsonNode) result = call_compositor(cli, "api", method, arguments, &call_error);
    json_object_unref(arguments);
    if (!result)
        return luaL_error(state, "%s failed: %s", method, call_error->message);
    if (!JSON_NODE_HOLDS_OBJECT(result))
        return luaL_error(state, "%s returned an invalid snapshot", method);

    JsonObject* snapshot = json_node_get_object(result);
    if (g_str_equal(method, "input.current_source")) {
        JsonNode* available = json_object_get_member(snapshot, "available");
        JsonObject* source = json_object_get_object_member(snapshot, "source");
        if (!available || !JSON_NODE_HOLDS_VALUE(available) ||
            json_node_get_value_type(available) != G_TYPE_BOOLEAN)
            return luaL_error(state, "%s returned an invalid availability state", method);
        if (!json_node_get_boolean(available)) {
            lua_pushnil(state);
            return 1;
        }
        if (!source)
            return luaL_error(state, "%s omitted its current source", method);
        gint64 revision = json_object_get_int_member_with_default(snapshot, "revision", -1);
        if (revision < 0)
            return luaL_error(state, "%s returned an invalid revision", method);
        lua_cli_push_input_record(state, source, revision, FALSE);
        return 1;
    }

    gboolean is_device = g_str_equal(method, "input.devices");
    const char* records_key = is_device ? "devices" : "sources";
    JsonArray* records = json_object_get_array_member(snapshot, records_key);
    gint64 revision = json_object_get_int_member_with_default(snapshot, "revision", -1);
    if (!records || revision < 0)
        return luaL_error(state, "%s returned an invalid snapshot", method);
    lua_createtable(state, json_array_get_length(records), 0);
    for (guint i = 0; i < json_array_get_length(records); i++) {
        JsonObject* record = json_array_get_object_element(records, i);
        if (!record)
            return luaL_error(state, "%s returned an invalid record", method);
        lua_cli_push_input_record(state, record, revision, is_device);
        lua_rawseti(state, -2, i + 1);
    }
    return 1;
}

static int lua_cli_input_select_source(lua_State* state) {
    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    if (lua_gettop(state) != 1 || !lua_istable(state, 1))
        return luaL_error(state, "input.select_source requires a {type, id} selector");
    const char* type = NULL;
    const char* id = NULL;
    lua_pushnil(state);
    while (lua_next(state, 1)) {
        if (lua_type(state, -2) != LUA_TSTRING) {
            lua_pop(state, 2);
            return luaL_error(state, "input selector accepts only type and id strings");
        }
        const char* key = lua_tostring(state, -2);
        if (g_str_equal(key, "type") && lua_type(state, -1) == LUA_TSTRING)
            type = lua_tostring(state, -1);
        else if (g_str_equal(key, "id") && lua_type(state, -1) == LUA_TSTRING)
            id = lua_tostring(state, -1);
        else {
            lua_pop(state, 2);
            return luaL_error(state, "input selector accepts only type and id strings");
        }
        lua_pop(state, 1);
    }
    if (!type || !*type || !id || !*id || (!g_str_equal(type, "xkb") && !g_str_equal(type, "ibus")))
        return luaL_error(state, "input.select_source requires type xkb or ibus and a nonempty id");

    JsonObject* arguments = json_object_new();
    json_object_set_string_member(arguments, "type", type);
    json_object_set_string_member(arguments, "id", id);
    g_autoptr(GError) call_error = NULL;
    g_autoptr(JsonNode) result =
        call_compositor(cli, "api", "input.select", arguments, &call_error);
    json_object_unref(arguments);
    if (!result)
        return luaL_error(state, "input.select_source failed: %s", call_error->message);
    if (!JSON_NODE_HOLDS_OBJECT(result))
        return luaL_error(state, "input.select_source returned an invalid InputSource");
    JsonObject* source = json_node_get_object(result);
    gint64 revision = json_object_get_int_member_with_default(source, "revision", -1);
    if (revision < 0)
        return luaL_error(state, "input.select_source returned an invalid InputSource");
    lua_cli_push_input_record(state, source, revision, FALSE);
    return 1;
}

static gboolean lua_cli_portal_grant_valid(JsonObject* object) {
    static const char* const string_fields[] = {"id", "kind", "requester"};
    for (guint i = 0; i < G_N_ELEMENTS(string_fields); i++) {
        JsonNode* field = json_object_get_member(object, string_fields[i]);
        if (!field || !JSON_NODE_HOLDS_VALUE(field) ||
            json_node_get_value_type(field) != G_TYPE_STRING)
            return FALSE;
    }
    const char* kind = member_string(object, "kind", NULL);
    if (!member_string(object, "id", NULL) || !*member_string(object, "id", "") ||
        !member_string(object, "requester", NULL) ||
        (!g_str_equal(kind, "screen-cast") && !g_str_equal(kind, "remote-desktop")))
        return FALSE;

    JsonNode* devices = json_object_get_member(object, "devices");
    JsonNode* clipboard = json_object_get_member(object, "clipboard");
    JsonNode* streams = json_object_get_member(object, "has_screen_streams");
    JsonNode* created_at = json_object_get_member(object, "created_at");
    JsonNode* revision = json_object_get_member(object, "revision");
    if (!devices || !JSON_NODE_HOLDS_ARRAY(devices) || !clipboard ||
        !JSON_NODE_HOLDS_VALUE(clipboard) ||
        json_node_get_value_type(clipboard) != G_TYPE_BOOLEAN || !streams ||
        !JSON_NODE_HOLDS_VALUE(streams) || json_node_get_value_type(streams) != G_TYPE_BOOLEAN ||
        !created_at || !JSON_NODE_HOLDS_VALUE(created_at) ||
        (json_node_get_value_type(created_at) != G_TYPE_INT64 &&
         json_node_get_value_type(created_at) != G_TYPE_INT) ||
        !revision || !JSON_NODE_HOLDS_VALUE(revision) ||
        (json_node_get_value_type(revision) != G_TYPE_INT64 &&
         json_node_get_value_type(revision) != G_TYPE_INT))
        return FALSE;

    JsonArray* device_array = json_node_get_array(devices);
    for (guint i = 0; i < json_array_get_length(device_array); i++) {
        JsonNode* device = json_array_get_element(device_array, i);
        if (!device || !JSON_NODE_HOLDS_VALUE(device) ||
            json_node_get_value_type(device) != G_TYPE_STRING)
            return FALSE;
    }
    return TRUE;
}

static void lua_cli_push_portal_grant_record(lua_State* state, Cli* cli, JsonObject* object) {
    if (!lua_cli_portal_grant_valid(object))
        luaL_error(state, "gnoblin.portals.grants returned an invalid PortalGrant");

    g_autoptr(JsonNode) node = json_node_new(JSON_NODE_OBJECT);
    json_node_set_object(node, object);
    json_to_lua(state, node);
    int backing = lua_absindex(state, -1);
    lua_newuserdatauv(state, 1, 2);
    int record = lua_absindex(state, -1);
    lua_pushvalue(state, backing);
    lua_setiuservalue(state, record, 1);
    lua_newtable(state);
    lua_pushlightuserdata(state, cli);
    lua_pushcclosure(state, lua_cli_portal_grant_revoke, 1);
    lua_setfield(state, -2, "revoke");
    lua_setiuservalue(state, record, 2);
    luaL_getmetatable(state, GNOBLINCTL_PORTAL_GRANT_RECORD_METATABLE);
    lua_setmetatable(state, record);
    lua_remove(state, backing);
}

static int lua_cli_portal_grant_revoke(lua_State* state) {
    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    luaL_checkudata(state, 1, GNOBLINCTL_PORTAL_GRANT_RECORD_METATABLE);
    if (lua_gettop(state) != 1)
        return luaL_error(state, "PortalGrant:revoke takes no arguments");

    lua_getiuservalue(state, 1, 1);
    JsonObject* arguments = json_object_new();
    const char* kind = NULL;
    const char* id = NULL;
    lua_getfield(state, -1, "kind");
    kind = lua_tostring(state, -1);
    if (kind)
        json_object_set_string_member(arguments, "kind", kind);
    lua_pop(state, 1);
    lua_getfield(state, -1, "id");
    id = lua_tostring(state, -1);
    if (id)
        json_object_set_string_member(arguments, "id", id);
    lua_pop(state, 1);
    lua_getfield(state, -1, "created_at");
    if (lua_isinteger(state, -1))
        json_object_set_int_member(arguments, "created_at", lua_tointeger(state, -1));
    else {
        json_object_unref(arguments);
        return luaL_error(state, "PortalGrant record has no creation timestamp");
    }
    lua_pop(state, 2);

    g_autoptr(GError) call_error = NULL;
    g_autoptr(JsonNode) result =
        call_compositor(cli, "api", "grant.revoke", arguments, &call_error);
    json_object_unref(arguments);
    if (!result)
        return luaL_error(state, "grant.revoke failed: %s", call_error->message);
    json_to_lua(state, result);
    return 1;
}

static int lua_cli_portal_grants(lua_State* state) {
    if (lua_gettop(state) > 1 || (lua_gettop(state) == 1 && !lua_istable(state, 1)))
        return luaL_error(state, "gnoblin.portals.grants accepts one optional filter table");

    JsonObject* arguments = json_object_new();
    if (lua_gettop(state) == 1) {
        lua_pushnil(state);
        while (lua_next(state, 1)) {
            gboolean valid = lua_type(state, -2) == LUA_TSTRING &&
                             g_str_equal(lua_tostring(state, -2), "kind") &&
                             lua_type(state, -1) == LUA_TSTRING &&
                             (g_str_equal(lua_tostring(state, -1), "screen-cast") ||
                              g_str_equal(lua_tostring(state, -1), "remote-desktop"));
            if (!valid) {
                json_object_unref(arguments);
                return luaL_error(
                    state, "portal grant filter accepts only kind = screen-cast or remote-desktop");
            }
            json_object_set_string_member(arguments, "kind", lua_tostring(state, -1));
            lua_pop(state, 1);
        }
    }

    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    g_autoptr(GError) call_error = NULL;
    g_autoptr(JsonNode) result =
        call_compositor(cli, "api", "portals.grants", arguments, &call_error);
    json_object_unref(arguments);
    if (!result)
        return luaL_error(state, "gnoblin.portals.grants failed: %s", call_error->message);
    if (!JSON_NODE_HOLDS_ARRAY(result))
        return luaL_error(state, "gnoblin.portals.grants returned an invalid snapshot");

    JsonArray* grants = json_node_get_array(result);
    lua_createtable(state, json_array_get_length(grants), 0);
    for (guint i = 0; i < json_array_get_length(grants); i++) {
        JsonObject* grant = json_array_get_object_element(grants, i);
        if (!grant)
            return luaL_error(state, "gnoblin.portals.grants returned an invalid record");
        lua_cli_push_portal_grant_record(state, cli, grant);
        lua_rawseti(state, -2, i + 1);
    }
    return 1;
}

static int lua_cli_layers_list(lua_State* state) {
    if (lua_gettop(state) > 1 || (lua_gettop(state) == 1 && !lua_istable(state, 1)))
        return luaL_error(state, "gnoblin.layers.list accepts one optional filter table");

    JsonObject* arguments = NULL;
    if (lua_gettop(state) == 1) {
        static const char* fields[] = {"monitor_id", "namespace", "layer"};
        lua_pushnil(state);
        while (lua_next(state, 1)) {
            const char* key = lua_type(state, -2) == LUA_TSTRING ? lua_tostring(state, -2) : NULL;
            gboolean known = FALSE;
            for (guint i = 0; key && i < G_N_ELEMENTS(fields); i++)
                known |= g_str_equal(key, fields[i]);
            if (!known || lua_type(state, -1) != LUA_TSTRING)
                return luaL_error(state, "gnoblin.layers.list filter fields must be monitor_id, "
                                         "namespace, or layer strings");
            lua_pop(state, 1);
        }
        arguments = lua_cli_table_object(state, 1, "layer filter");
    } else {
        arguments = json_object_new();
    }

    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    g_autoptr(GError) call_error = NULL;
    g_autoptr(JsonNode) result = call_compositor(cli, "api", "layers.list", arguments, &call_error);
    json_object_unref(arguments);
    if (!result)
        return luaL_error(state, "gnoblin.layers.list failed: %s", call_error->message);
    if (!JSON_NODE_HOLDS_ARRAY(result))
        return luaL_error(state, "gnoblin.layers.list returned an invalid snapshot");

    JsonArray* layers = json_node_get_array(result);
    lua_createtable(state, json_array_get_length(layers), 0);
    for (guint i = 0; i < json_array_get_length(layers); i++) {
        JsonObject* layer = json_array_get_object_element(layers, i);
        if (!layer)
            return luaL_error(state,
                              "gnoblin.layers.list returned an invalid layer surface record");
        lua_cli_push_layer_surface_record(state, layer);
        lua_rawseti(state, -2, i + 1);
    }
    return 1;
}

static void register_lua_cli_layer_surface_record(lua_State* state) {
    if (!luaL_newmetatable(state, GNOBLINCTL_LAYER_SURFACE_RECORD_METATABLE)) {
        lua_pop(state, 1);
        return;
    }
    lua_pushcfunction(state, lua_cli_window_index);
    lua_setfield(state, -2, "__index");
    lua_pushcfunction(state, lua_cli_window_newindex);
    lua_setfield(state, -2, "__newindex");
    lua_pushcfunction(state, lua_cli_window_len);
    lua_setfield(state, -2, "__len");
    lua_pushcfunction(state, lua_cli_window_pairs);
    lua_setfield(state, -2, "__pairs");
    lua_pushcfunction(state, lua_cli_layer_surface_tostring);
    lua_setfield(state, -2, "__tostring");
    lua_pop(state, 1);
}

static int lua_cli_animation_preview_tostring(lua_State* state) {
    lua_getiuservalue(state, 1, 1);
    lua_getfield(state, -1, "id");
    const char* id = lua_tostring(state, -1);
    lua_pushfstring(state, "AnimationPreview<%s>", id ? id : "unknown");
    return 1;
}

static int lua_cli_animation_preview_newindex(lua_State* state) {
    return luaL_error(state, "AnimationPreview records are read-only");
}

static void register_lua_cli_animation_preview_record(lua_State* state) {
    if (!luaL_newmetatable(state, GNOBLINCTL_ANIMATION_PREVIEW_RECORD_METATABLE)) {
        lua_pop(state, 1);
        return;
    }
    lua_pushcfunction(state, lua_cli_window_index);
    lua_setfield(state, -2, "__index");
    lua_pushcfunction(state, lua_cli_animation_preview_newindex);
    lua_setfield(state, -2, "__newindex");
    lua_pushcfunction(state, lua_cli_window_len);
    lua_setfield(state, -2, "__len");
    lua_pushcfunction(state, lua_cli_window_pairs);
    lua_setfield(state, -2, "__pairs");
    lua_pushcfunction(state, lua_cli_animation_preview_tostring);
    lua_setfield(state, -2, "__tostring");
    lua_pop(state, 1);
}

static int lua_cli_portal_grant_tostring(lua_State* state) {
    lua_getiuservalue(state, 1, 1);
    lua_getfield(state, -1, "id");
    const char* id = lua_tostring(state, -1);
    lua_pushfstring(state, "PortalGrant<%s>", id ? id : "unknown");
    return 1;
}

static int lua_cli_portal_grant_newindex(lua_State* state) {
    return luaL_error(state, "PortalGrant records are read-only");
}

static void register_lua_cli_portal_grant_record(lua_State* state) {
    if (!luaL_newmetatable(state, GNOBLINCTL_PORTAL_GRANT_RECORD_METATABLE)) {
        lua_pop(state, 1);
        return;
    }
    lua_pushcfunction(state, lua_cli_window_index);
    lua_setfield(state, -2, "__index");
    lua_pushcfunction(state, lua_cli_portal_grant_newindex);
    lua_setfield(state, -2, "__newindex");
    lua_pushcfunction(state, lua_cli_window_len);
    lua_setfield(state, -2, "__len");
    lua_pushcfunction(state, lua_cli_window_pairs);
    lua_setfield(state, -2, "__pairs");
    lua_pushcfunction(state, lua_cli_portal_grant_tostring);
    lua_setfield(state, -2, "__tostring");
    lua_pop(state, 1);
}

static void register_lua_cli_input_record(lua_State* state, const char* metatable) {
    if (!luaL_newmetatable(state, metatable)) {
        lua_pop(state, 1);
        return;
    }
    lua_pushcfunction(state, lua_cli_window_index);
    lua_setfield(state, -2, "__index");
    lua_pushcfunction(state, lua_cli_readonly_newindex);
    lua_setfield(state, -2, "__newindex");
    lua_pushcfunction(state, lua_cli_window_len);
    lua_setfield(state, -2, "__len");
    lua_pushcfunction(state, lua_cli_window_pairs);
    lua_setfield(state, -2, "__pairs");
    lua_pushcfunction(state, lua_cli_input_record_tostring);
    lua_setfield(state, -2, "__tostring");
    lua_pop(state, 1);
}

static JsonNode* lua_cli_monitor_snapshot(lua_State* state, Cli* cli, const char* operation) {
    JsonObject* arguments = json_object_new();
    g_autoptr(GError) call_error = NULL;
    JsonNode* result = call_compositor(cli, "api", "monitors.list", arguments, &call_error);
    json_object_unref(arguments);
    if (!result)
        luaL_error(state, "gnoblin.monitors.%s failed: %s", operation, call_error->message);
    if (!JSON_NODE_HOLDS_ARRAY(result)) {
        json_node_unref(result);
        luaL_error(state, "gnoblin.monitors.%s returned an invalid snapshot", operation);
    }
    return result;
}

static int lua_cli_monitors_list(lua_State* state) {
    if (lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.monitors.list takes no arguments");
    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    g_autoptr(JsonNode) result = lua_cli_monitor_snapshot(state, cli, "list");
    JsonArray* monitors = json_node_get_array(result);
    lua_createtable(state, json_array_get_length(monitors), 0);
    for (guint i = 0; i < json_array_get_length(monitors); i++) {
        JsonObject* monitor = json_array_get_object_element(monitors, i);
        if (!monitor)
            return luaL_error(state, "gnoblin.monitors.list returned an invalid monitor record");
        lua_cli_push_monitor_record(state, monitor);
        lua_rawseti(state, -2, i + 1);
    }
    return 1;
}

static int lua_cli_monitors_primary(lua_State* state) {
    if (lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.monitors.primary takes no arguments");
    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    g_autoptr(JsonNode) result = lua_cli_monitor_snapshot(state, cli, "primary");
    JsonArray* monitors = json_node_get_array(result);
    for (guint i = 0; i < json_array_get_length(monitors); i++) {
        JsonObject* monitor = json_array_get_object_element(monitors, i);
        if (monitor && json_object_get_boolean_member_with_default(monitor, "primary", FALSE)) {
            lua_cli_push_monitor_record(state, monitor);
            return 1;
        }
    }
    lua_pushnil(state);
    return 1;
}

static int lua_cli_workspaces_list(lua_State* state) {
    if (lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.workspaces.list takes no arguments");
    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    JsonObject* arguments = json_object_new();
    g_autoptr(GError) call_error = NULL;
    g_autoptr(JsonNode) result =
        call_compositor(cli, "api", "workspaces.list", arguments, &call_error);
    json_object_unref(arguments);
    if (!result)
        return luaL_error(state, "gnoblin.workspaces.list failed: %s", call_error->message);
    if (!JSON_NODE_HOLDS_ARRAY(result))
        return luaL_error(state, "gnoblin.workspaces.list returned an invalid snapshot");
    JsonArray* workspaces = json_node_get_array(result);
    lua_createtable(state, json_array_get_length(workspaces), 0);
    for (guint i = 0; i < json_array_get_length(workspaces); i++) {
        JsonObject* workspace = json_array_get_object_element(workspaces, i);
        if (!workspace)
            return luaL_error(state,
                              "gnoblin.workspaces.list returned an invalid workspace record");
        lua_cli_push_workspace_record(state, cli, workspace);
        lua_rawseti(state, -2, i + 1);
    }
    return 1;
}

static int lua_cli_workspaces_active(lua_State* state) {
    if (lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.workspaces.active takes no arguments");
    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    JsonObject* arguments = json_object_new();
    g_autoptr(GError) call_error = NULL;
    g_autoptr(JsonNode) result =
        call_compositor(cli, "api", "workspaces.list", arguments, &call_error);
    json_object_unref(arguments);
    if (!result)
        return luaL_error(state, "gnoblin.workspaces.active failed: %s", call_error->message);
    if (!JSON_NODE_HOLDS_ARRAY(result))
        return luaL_error(state, "gnoblin.workspaces.active returned an invalid snapshot");
    JsonArray* workspaces = json_node_get_array(result);
    for (guint i = 0; i < json_array_get_length(workspaces); i++) {
        JsonObject* workspace = json_array_get_object_element(workspaces, i);
        JsonNode* active = workspace ? json_object_get_member(workspace, "active") : NULL;
        if (active && JSON_NODE_HOLDS_VALUE(active) &&
            json_node_get_value_type(active) == G_TYPE_BOOLEAN && json_node_get_boolean(active)) {
            lua_cli_push_workspace_record(state, cli, workspace);
            return 1;
        }
    }
    lua_pushnil(state);
    return 1;
}

static int lua_cli_workspaces_by_id(lua_State* state) {
    if (lua_gettop(state) != 1 || lua_type(state, 1) != LUA_TSTRING || !*lua_tostring(state, 1))
        return luaL_error(state,
                          "gnoblin.workspaces.by_id requires one stable workspace ID string");
    const char* id = lua_tostring(state, 1);
    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    JsonObject* arguments = json_object_new();
    g_autoptr(GError) call_error = NULL;
    g_autoptr(JsonNode) result =
        call_compositor(cli, "api", "workspaces.list", arguments, &call_error);
    json_object_unref(arguments);
    if (!result)
        return luaL_error(state, "gnoblin.workspaces.by_id failed: %s", call_error->message);
    if (!JSON_NODE_HOLDS_ARRAY(result))
        return luaL_error(state, "gnoblin.workspaces.by_id returned an invalid snapshot");
    JsonArray* workspaces = json_node_get_array(result);
    for (guint i = 0; i < json_array_get_length(workspaces); i++) {
        JsonObject* workspace = json_array_get_object_element(workspaces, i);
        if (workspace && g_str_equal(member_string(workspace, "id", ""), id)) {
            lua_cli_push_workspace_record(state, cli, workspace);
            return 1;
        }
    }
    lua_pushnil(state);
    return 1;
}

static int lua_cli_windows_list(lua_State* state) {
    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    if (lua_gettop(state) > 1 || (lua_gettop(state) == 1 && !lua_istable(state, 1)))
        return luaL_error(state, "gnoblin.windows.list accepts one optional filter table");
    JsonObject* arguments =
        lua_gettop(state) ? lua_cli_table_object(state, 1, "window filter") : json_object_new();
    g_autoptr(GError) call_error = NULL;
    g_autoptr(JsonNode) result =
        call_compositor(cli, "api", "windows.list", arguments, &call_error);
    json_object_unref(arguments);
    if (!result)
        return luaL_error(state, "gnoblin.windows.list failed: %s", call_error->message);
    if (!JSON_NODE_HOLDS_ARRAY(result))
        return luaL_error(state, "gnoblin.windows.list returned an invalid snapshot");
    JsonArray* windows = json_node_get_array(result);
    lua_createtable(state, json_array_get_length(windows), 0);
    for (guint i = 0; i < json_array_get_length(windows); i++) {
        JsonObject* window = json_array_get_object_element(windows, i);
        if (!window)
            return luaL_error(state, "gnoblin.windows.list returned an invalid window record");
        lua_cli_push_window_record(state, cli, window);
        lua_rawseti(state, -2, i + 1);
    }
    return 1;
}

static int lua_cli_focus_history(lua_State* state) {
    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    if (lua_gettop(state) > 1 || (lua_gettop(state) == 1 && !lua_istable(state, 1)))
        return luaL_error(state, "gnoblin.focus.history accepts one optional filter table");
    JsonObject* arguments = lua_gettop(state)
                                ? lua_cli_table_object(state, 1, "focus history filter")
                                : json_object_new();
    g_autoptr(GError) call_error = NULL;
    g_autoptr(JsonNode) result =
        call_compositor(cli, "api", "focus.history", arguments, &call_error);
    json_object_unref(arguments);
    if (!result)
        return luaL_error(state, "gnoblin.focus.history failed: %s", call_error->message);
    if (!JSON_NODE_HOLDS_ARRAY(result))
        return luaL_error(state, "gnoblin.focus.history returned an invalid snapshot");

    JsonArray* windows = json_node_get_array(result);
    lua_createtable(state, json_array_get_length(windows), 0);
    for (guint i = 0; i < json_array_get_length(windows); i++) {
        JsonObject* window = json_array_get_object_element(windows, i);
        if (!window || !member_string(window, "id", NULL) || !*member_string(window, "id", ""))
            return luaL_error(state, "gnoblin.focus.history returned an invalid Window record");
        lua_cli_push_window_record(state, cli, window);
        lua_rawseti(state, -2, i + 1);
    }
    return 1;
}

static int lua_cli_windows_focused(lua_State* state) {
    if (lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.windows.focused takes no arguments");
    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    JsonObject* arguments = json_object_new();
    json_object_set_boolean_member(arguments, "focused", TRUE);
    g_autoptr(GError) call_error = NULL;
    g_autoptr(JsonNode) result =
        call_compositor(cli, "api", "windows.list", arguments, &call_error);
    json_object_unref(arguments);
    if (!result)
        return luaL_error(state, "gnoblin.windows.focused failed: %s", call_error->message);
    if (!JSON_NODE_HOLDS_ARRAY(result))
        return luaL_error(state, "gnoblin.windows.focused returned an invalid snapshot");
    JsonArray* windows = json_node_get_array(result);
    if (json_array_get_length(windows) == 0) {
        lua_pushnil(state);
        return 1;
    }
    JsonObject* window = json_array_get_object_element(windows, 0);
    if (!window)
        return luaL_error(state, "gnoblin.windows.focused returned an invalid window record");
    lua_cli_push_window_record(state, cli, window);
    return 1;
}

static int lua_cli_windows_by_id(lua_State* state) {
    if (lua_gettop(state) != 1 || lua_type(state, 1) != LUA_TSTRING)
        return luaL_error(state, "gnoblin.windows.by_id requires one stable window ID string");
    const char* id = lua_tostring(state, 1);
    if (!*id)
        return luaL_error(state, "gnoblin.windows.by_id requires one stable window ID string");
    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    JsonObject* arguments = json_object_new();
    g_autoptr(GError) call_error = NULL;
    g_autoptr(JsonNode) result =
        call_compositor(cli, "api", "windows.list", arguments, &call_error);
    json_object_unref(arguments);
    if (!result)
        return luaL_error(state, "gnoblin.windows.by_id failed: %s", call_error->message);
    if (!JSON_NODE_HOLDS_ARRAY(result))
        return luaL_error(state, "gnoblin.windows.by_id returned an invalid snapshot");
    JsonArray* windows = json_node_get_array(result);
    for (guint i = 0; i < json_array_get_length(windows); i++) {
        JsonObject* window = json_array_get_object_element(windows, i);
        if (window && g_str_equal(member_string(window, "id", ""), id)) {
            lua_cli_push_window_record(state, cli, window);
            return 1;
        }
    }
    lua_pushnil(state);
    return 1;
}

static int lua_api_call(lua_State* state) {
    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    const char* method = lua_tostring(state, lua_upvalueindex(2));
    if (lua_gettop(state) > 1 || (lua_gettop(state) == 1 && !lua_istable(state, 1)))
        return luaL_error(state, "Gnoblin API methods accept one optional argument table");

    JsonObject* arguments = json_object_new();
    if (lua_gettop(state) == 1) {
        g_autoptr(GError) conversion_error = NULL;
        g_autoptr(JsonNode) encoded = lua_to_json(state, 1, 0, &conversion_error);
        if (!encoded)
            return luaL_error(state, "invalid API arguments: %s", conversion_error->message);
        if (!JSON_NODE_HOLDS_OBJECT(encoded))
            return luaL_error(state, "Gnoblin API arguments must be a named Lua table");
        json_object_unref(arguments);
        arguments = json_object_ref(json_node_get_object(encoded));
    }

    g_autoptr(GError) call_error = NULL;
    g_autoptr(JsonNode) result = call_compositor(cli, "api", method, arguments, &call_error);
    json_object_unref(arguments);
    if (!result)
        return luaL_error(state, "Gnoblin API call failed: %s", call_error->message);
    json_to_lua(state, result);
    return 1;
}

static int lua_api_index(lua_State* state) {
    const char* prefix = lua_tostring(state, lua_upvalueindex(1));
    const char* name = luaL_checkstring(state, 2);
    if (!*prefix) {
        if (g_str_equal(name, "version")) {
            lua_pushlightuserdata(state, lua_touserdata(state, lua_upvalueindex(2)));
            lua_pushcclosure(state, lua_cli_version, 1);
            return 1;
        }
        if (g_str_equal(name, "settings")) {
            lua_pushlightuserdata(state, lua_touserdata(state, lua_upvalueindex(2)));
            lua_pushcclosure(state, lua_cli_settings_property, 1);
            lua_call(state, 0, 1);
            return 1;
        }
        lua_newtable(state);
        lua_newtable(state);
        lua_pushstring(state, name);
        lua_pushvalue(state, lua_upvalueindex(2));
        lua_pushcclosure(state, lua_api_index, 2);
        lua_setfield(state, -2, "__index");
        lua_setmetatable(state, -2);
        return 1;
    }
    if (g_str_equal(prefix, "windows")) {
        lua_pushlightuserdata(state, lua_touserdata(state, lua_upvalueindex(2)));
        if (g_str_equal(name, "list"))
            lua_pushcclosure(state, lua_cli_windows_list, 1);
        else if (g_str_equal(name, "focused"))
            lua_pushcclosure(state, lua_cli_windows_focused, 1);
        else if (g_str_equal(name, "by_id"))
            lua_pushcclosure(state, lua_cli_windows_by_id, 1);
        else
            lua_pop(state, 1);
        if (g_str_equal(name, "list") || g_str_equal(name, "focused") || g_str_equal(name, "by_id"))
            return 1;
    }
    if (g_str_equal(prefix, "workspaces")) {
        lua_pushlightuserdata(state, lua_touserdata(state, lua_upvalueindex(2)));
        if (g_str_equal(name, "list"))
            lua_pushcclosure(state, lua_cli_workspaces_list, 1);
        else if (g_str_equal(name, "active"))
            lua_pushcclosure(state, lua_cli_workspaces_active, 1);
        else if (g_str_equal(name, "by_id"))
            lua_pushcclosure(state, lua_cli_workspaces_by_id, 1);
        else
            lua_pop(state, 1);
        if (g_str_equal(name, "list") || g_str_equal(name, "active") || g_str_equal(name, "by_id"))
            return 1;
    }
    if (g_str_equal(prefix, "monitors")) {
        lua_pushlightuserdata(state, lua_touserdata(state, lua_upvalueindex(2)));
        if (g_str_equal(name, "list"))
            lua_pushcclosure(state, lua_cli_monitors_list, 1);
        else if (g_str_equal(name, "primary"))
            lua_pushcclosure(state, lua_cli_monitors_primary, 1);
        else
            lua_pop(state, 1);
        if (g_str_equal(name, "list") || g_str_equal(name, "primary"))
            return 1;
    }
    if (g_str_equal(prefix, "monitor") && g_str_equal(name, "list")) {
        lua_pushlightuserdata(state, lua_touserdata(state, lua_upvalueindex(2)));
        lua_pushcclosure(state, lua_cli_monitors_list, 1);
        return 1;
    }
    if (g_str_equal(prefix, "layers")) {
        lua_pushlightuserdata(state, lua_touserdata(state, lua_upvalueindex(2)));
        if (g_str_equal(name, "list"))
            lua_pushcclosure(state, lua_cli_layers_list, 1);
        else if (g_str_equal(name, "animation_policy"))
            lua_pushcclosure(state, lua_cli_layer_animation_policy, 1);
        else
            lua_pop(state, 1);
        if (g_str_equal(name, "list") || g_str_equal(name, "animation_policy"))
            return 1;
    }
    if (g_str_equal(prefix, "privacy") && g_str_equal(name, "state")) {
        lua_pushlightuserdata(state, lua_touserdata(state, lua_upvalueindex(2)));
        lua_pushcclosure(state, lua_cli_privacy_state, 1);
        return 1;
    }
    if (g_str_equal(prefix, "capabilities") && g_str_equal(name, "list")) {
        lua_pushlightuserdata(state, lua_touserdata(state, lua_upvalueindex(2)));
        lua_pushcclosure(state, lua_cli_capabilities_list, 1);
        return 1;
    }
    if (g_str_equal(prefix, "permissions") && g_str_equal(name, "list")) {
        lua_pushlightuserdata(state, lua_touserdata(state, lua_upvalueindex(2)));
        lua_pushcclosure(state, lua_cli_permissions_list, 1);
        return 1;
    }
    if (g_str_equal(prefix, "permissions") && g_str_equal(name, "policy")) {
        lua_pushlightuserdata(state, lua_touserdata(state, lua_upvalueindex(2)));
        lua_pushcclosure(state, lua_cli_permissions_policy, 1);
        return 1;
    }
    if (g_str_equal(prefix, "permissions") && g_str_equal(name, "check")) {
        lua_pushlightuserdata(state, lua_touserdata(state, lua_upvalueindex(2)));
        lua_pushcclosure(state, lua_cli_permissions_check, 1);
        return 1;
    }
    if (g_str_equal(prefix, "session") && g_str_equal(name, "status")) {
        lua_pushlightuserdata(state, lua_touserdata(state, lua_upvalueindex(2)));
        lua_pushcclosure(state, lua_cli_session_status, 1);
        return 1;
    }
    if (g_str_equal(prefix, "session") && g_str_equal(name, "activity")) {
        lua_pushlightuserdata(state, lua_touserdata(state, lua_upvalueindex(2)));
        lua_pushcclosure(state, lua_cli_session_activity, 1);
        return 1;
    }
    if (g_str_equal(prefix, "animations") && g_str_equal(name, "preview")) {
        lua_pushlightuserdata(state, lua_touserdata(state, lua_upvalueindex(2)));
        lua_pushcclosure(state, lua_cli_animations_preview, 1);
        return 1;
    }
    if (g_str_equal(prefix, "animations") &&
        (g_str_equal(name, "list") || g_str_equal(name, "get") || g_str_equal(name, "surfaces"))) {
        lua_pushlightuserdata(state, lua_touserdata(state, lua_upvalueindex(2)));
        lua_CFunction function = g_str_equal(name, "list")  ? lua_cli_animations_list
                                 : g_str_equal(name, "get") ? lua_cli_animations_get
                                                            : lua_cli_animations_surfaces;
        lua_pushcclosure(state, function, 1);
        return 1;
    }
    if (g_str_equal(prefix, "launches") &&
        (g_str_equal(name, "list") || g_str_equal(name, "snapshot"))) {
        lua_pushlightuserdata(state, lua_touserdata(state, lua_upvalueindex(2)));
        lua_pushboolean(state, g_str_equal(name, "snapshot"));
        lua_pushcclosure(state, lua_cli_launches_read, 2);
        return 1;
    }
    if (g_str_equal(prefix, "input")) {
        if (g_str_equal(name, "devices") || g_str_equal(name, "sources") ||
            g_str_equal(name, "current_source")) {
            g_autofree char* method = g_strdup_printf("input.%s", name);
            lua_pushlightuserdata(state, lua_touserdata(state, lua_upvalueindex(2)));
            lua_pushstring(state, method);
            lua_pushcclosure(state, lua_cli_input_snapshot, 2);
            return 1;
        }
        if (g_str_equal(name, "select_source")) {
            lua_pushlightuserdata(state, lua_touserdata(state, lua_upvalueindex(2)));
            lua_pushcclosure(state, lua_cli_input_select_source, 1);
            return 1;
        }
    }
    if (g_str_equal(prefix, "focus")) {
        if (g_str_equal(name, "policy")) {
            lua_pushlightuserdata(state, lua_touserdata(state, lua_upvalueindex(2)));
            lua_pushcclosure(state, lua_cli_focus_policy_property, 1);
            lua_call(state, 0, 1);
            return 1;
        }
        if (g_str_equal(name, "history")) {
            lua_pushlightuserdata(state, lua_touserdata(state, lua_upvalueindex(2)));
            lua_pushcclosure(state, lua_cli_focus_history, 1);
            return 1;
        }
    }
    if (g_str_equal(prefix, "shortcuts")) {
        if (g_str_equal(name, "list") || g_str_equal(name, "actions")) {
            lua_pushlightuserdata(state, lua_touserdata(state, lua_upvalueindex(2)));
            lua_CFunction function =
                g_str_equal(name, "list") ? lua_cli_shortcuts_list : lua_cli_shortcuts_actions;
            lua_pushcclosure(state, function, 1);
            return 1;
        }
        if (g_str_equal(name, "capture")) {
            lua_pushlightuserdata(state, lua_touserdata(state, lua_upvalueindex(2)));
            lua_pushcclosure(state, lua_cli_shortcuts_capture, 1);
            return 1;
        }
    }
    if (g_str_equal(prefix, "portals") && g_str_equal(name, "grants")) {
        lua_pushlightuserdata(state, lua_touserdata(state, lua_upvalueindex(2)));
        lua_pushcclosure(state, lua_cli_portal_grants, 1);
        return 1;
    }
    if (g_str_equal(prefix, "grant") && g_str_equal(name, "list")) {
        lua_pushlightuserdata(state, lua_touserdata(state, lua_upvalueindex(2)));
        lua_pushcclosure(state, lua_cli_portal_grants, 1);
        return 1;
    }
    if (g_str_equal(prefix, "layer") && g_str_equal(name, "list")) {
        lua_pushlightuserdata(state, lua_touserdata(state, lua_upvalueindex(2)));
        lua_pushcclosure(state, lua_cli_layers_list, 1);
        return 1;
    }
    g_autofree char* method = g_strdup_printf("%s.%s", prefix, name);
    lua_pushlightuserdata(state, lua_touserdata(state, lua_upvalueindex(2)));
    lua_pushstring(state, method);
    lua_pushcclosure(state, lua_api_call, 2);
    return 1;
}

static void register_lua_cli_window_record(lua_State* state) {
    if (!luaL_newmetatable(state, GNOBLINCTL_WINDOW_RECORD_METATABLE)) {
        lua_pop(state, 1);
        return;
    }
    lua_pushcfunction(state, lua_cli_window_index);
    lua_setfield(state, -2, "__index");
    lua_pushcfunction(state, lua_cli_window_newindex);
    lua_setfield(state, -2, "__newindex");
    lua_pushcfunction(state, lua_cli_window_len);
    lua_setfield(state, -2, "__len");
    lua_pushcfunction(state, lua_cli_window_pairs);
    lua_setfield(state, -2, "__pairs");
    lua_pushcfunction(state, lua_cli_window_tostring);
    lua_setfield(state, -2, "__tostring");
    lua_pop(state, 1);
}

static void register_lua_cli_workspace_record(lua_State* state) {
    if (!luaL_newmetatable(state, GNOBLINCTL_WORKSPACE_RECORD_METATABLE)) {
        lua_pop(state, 1);
        return;
    }
    lua_pushcfunction(state, lua_cli_window_index);
    lua_setfield(state, -2, "__index");
    lua_pushcfunction(state, lua_cli_window_newindex);
    lua_setfield(state, -2, "__newindex");
    lua_pushcfunction(state, lua_cli_window_len);
    lua_setfield(state, -2, "__len");
    lua_pushcfunction(state, lua_cli_window_pairs);
    lua_setfield(state, -2, "__pairs");
    lua_pushcfunction(state, lua_cli_workspace_tostring);
    lua_setfield(state, -2, "__tostring");
    lua_pop(state, 1);
}

static void register_lua_cli_monitor_record(lua_State* state) {
    if (!luaL_newmetatable(state, GNOBLINCTL_MONITOR_RECORD_METATABLE)) {
        lua_pop(state, 1);
        return;
    }
    lua_pushcfunction(state, lua_cli_window_index);
    lua_setfield(state, -2, "__index");
    lua_pushcfunction(state, lua_cli_window_newindex);
    lua_setfield(state, -2, "__newindex");
    lua_pushcfunction(state, lua_cli_window_len);
    lua_setfield(state, -2, "__len");
    lua_pushcfunction(state, lua_cli_window_pairs);
    lua_setfield(state, -2, "__pairs");
    lua_pushcfunction(state, lua_cli_monitor_tostring);
    lua_setfield(state, -2, "__tostring");
    lua_pop(state, 1);
}

static void print_lua_error(lua_State* state, const char* prefix) {
    const char* message = lua_tostring(state, -1);
    g_printerr("%s%s\n", prefix, message ? message : "Lua evaluation failed");
    lua_pop(state, 1);
}

static void print_lua_result(lua_State* state, int index) {
    index = lua_absindex(state, index);
    if (lua_istable(state, index)) {
        g_autoptr(GError) error = NULL;
        g_autoptr(JsonNode) node = lua_to_json(state, index, 0, &error);
        if (node) {
            g_autofree char* json = json_to_string(node, TRUE);
            g_print("%s\n", json);
            return;
        }
        g_printerr("gnoblinctl lua: cannot display result as JSON: %s\n", error->message);
        return;
    }

    lua_getglobal(state, "tostring");
    lua_pushvalue(state, index);
    if (lua_pcall(state, 1, 1, 0) != LUA_OK) {
        print_lua_error(state, "gnoblinctl lua: ");
        return;
    }
    const char* value = lua_tostring(state, -1);
    g_print("%s\n", value ? value : "nil");
    lua_pop(state, 1);
}

static gboolean evaluate_lua_line(lua_State* state, const char* line) {
    g_autofree char* expression = NULL;
    const char* chunk = line;
    if (line[0] == '=')
        chunk = expression = g_strdup_printf("return %s", line + 1);
    else
        chunk = expression = g_strdup_printf("return %s", line);

    if (luaL_loadbuffer(state, chunk, strlen(chunk), "=gnoblinctl") != LUA_OK) {
        lua_pop(state, 1);
        if (line[0] == '=') {
            g_autofree char* statement = g_strdup(line + 1);
            if (luaL_loadbuffer(state, statement, strlen(statement), "=gnoblinctl") != LUA_OK) {
                print_lua_error(state, "gnoblinctl lua: ");
                return FALSE;
            }
        } else if (luaL_loadbuffer(state, line, strlen(line), "=gnoblinctl") != LUA_OK) {
            print_lua_error(state, "gnoblinctl lua: ");
            return FALSE;
        }
    }

    if (lua_pcall(state, 0, LUA_MULTRET, 0) != LUA_OK) {
        print_lua_error(state, "gnoblinctl lua: ");
        return FALSE;
    }
    int result_count = lua_gettop(state);
    for (int i = 1; i <= result_count; i++)
        print_lua_result(state, i);
    lua_settop(state, 0);
    return TRUE;
}

static int run_lua_console(Cli* cli, const char* file) {
    lua_State* state = luaL_newstate();
    if (!state) {
        g_printerr("gnoblinctl lua: could not create Lua state\n");
        return 1;
    }
    luaL_openlibs(state);
    register_lua_cli_readonly_table(state);
    register_lua_cli_window_record(state);
    register_lua_cli_workspace_record(state);
    register_lua_cli_monitor_record(state);
    register_lua_cli_layer_surface_record(state);
    register_lua_cli_animation_preview_record(state);
    register_lua_cli_portal_grant_record(state);
    register_lua_cli_input_record(state, GNOBLINCTL_INPUT_DEVICE_RECORD_METATABLE);
    register_lua_cli_input_record(state, GNOBLINCTL_INPUT_SOURCE_RECORD_METATABLE);
    register_lua_cli_shortcut_record(state, GNOBLINCTL_SHORTCUT_STATE_RECORD_METATABLE);
    register_lua_cli_shortcut_record(state, GNOBLINCTL_SHORTCUT_ACTION_RECORD_METATABLE);
    register_lua_cli_focus_policy_record(state);
    register_lua_cli_settings_record(state);
    register_lua_cli_layer_animation_policy_record(state);
    register_lua_cli_privacy_state_record(state);
    register_lua_cli_capability_record(state);
    register_lua_cli_permission_policy_record(state);
    register_lua_cli_permission_decision_record(state);
    register_lua_cli_session_status_record(state);
    register_lua_cli_session_activity_record(state);
    lua_newtable(state);
    lua_newtable(state);
    lua_pushstring(state, "");
    lua_pushlightuserdata(state, cli);
    lua_pushcclosure(state, lua_api_index, 2);
    lua_setfield(state, -2, "__index");
    lua_setmetatable(state, -2);
    lua_setglobal(state, "gnoblin");

    if (file) {
        if (luaL_loadfile(state, file) != LUA_OK || lua_pcall(state, 0, 0, 0) != LUA_OK) {
            print_lua_error(state, "gnoblinctl lua: ");
            lua_close(state);
            return 1;
        }
        lua_close(state);
        return 0;
    }

    gboolean interactive = isatty(STDIN_FILENO);
    if (interactive)
        g_print("Gnoblin Lua console. Type :help for commands, :quit to exit.\n");
    char* line = NULL;
    size_t capacity = 0;
    ssize_t length;
    int exit_status = 0;
    while (TRUE) {
        if (interactive) {
            g_print("gnoblin> ");
            fflush(stdout);
        }
        length = getline(&line, &capacity, stdin);
        if (length < 0)
            break;
        g_strchomp(line);
        g_strstrip(line);
        if (!*line)
            continue;
        if (g_str_equal(line, ":quit") || g_str_equal(line, ":q"))
            break;
        if (g_str_equal(line, ":help")) {
            g_print("Enter Lua expressions or statements. Use gnoblin.<area>.<method>{...} "
                    "to call the typed session API.\n"
                    "Example: =gnoblin.windows.list{focused = true}\n"
                    "Calls use the running compositor's API validation. :quit exits.\n");
            continue;
        }
        if (!evaluate_lua_line(state, line))
            exit_status = 1;
    }
    free(line);
    lua_close(state);
    return exit_status;
}

static char* node_text(JsonNode* node) {
    if (!node)
        return g_strdup("-");
    if (JSON_NODE_HOLDS_NULL(node))
        return g_strdup("null");
    if (JSON_NODE_HOLDS_VALUE(node)) {
        GType type = json_node_get_value_type(node);
        if (type == G_TYPE_BOOLEAN)
            return g_strdup(json_node_get_boolean(node) ? "yes" : "no");
        if (type == G_TYPE_STRING) {
            const char* value = json_node_get_string(node);
            GString* clean = g_string_new(NULL);
            for (const char* p = value; *p; p = g_utf8_next_char(p)) {
                gunichar character = g_utf8_get_char_validated(p, -1);
                if (character == (gunichar)-1 || character == (gunichar)-2) {
                    g_string_append_c(clean, ' ');
                    continue;
                }
                if (g_unichar_isprint(character))
                    g_string_append_len(clean, p, g_utf8_next_char(p) - p);
                else
                    g_string_append_c(clean, ' ');
            }
            return g_string_free(clean, FALSE);
        }
    }
    return json_to_string(node, FALSE);
}

static char* human_label(const char* name) {
    GString* label = g_string_new(NULL);
    for (const char* p = name; *p; p++) {
        if (*p == '_')
            g_string_append_c(label, ' ');
        else if (g_ascii_isupper(*p)) {
            if (p != name)
                g_string_append_c(label, ' ');
            g_string_append_c(label, g_ascii_tolower(*p));
        } else
            g_string_append_c(label, *p);
    }
    return g_string_free(label, FALSE);
}

static char* table_header(const char* name) {
    if (g_str_equal(name, "appId") || g_str_equal(name, "app_id"))
        return g_strdup("APP ID");
    if (g_str_equal(name, "gtk_app_id"))
        return g_strdup("GTK APP ID");
    if (g_str_equal(name, "wm_class"))
        return g_strdup("WM CLASS");
    if (g_str_equal(name, "monitorId") || g_str_equal(name, "monitor_id"))
        return g_strdup("MONITOR ID");
    if (g_str_equal(name, "monitorIndex") || g_str_equal(name, "monitor_index"))
        return g_strdup("MONITOR INDEX");
    if (g_str_equal(name, "shortName"))
        return g_strdup("SHORT NAME");
    g_autofree char* label = human_label(name);
    return g_ascii_strup(label, -1);
}

static void print_cell(const char* value, guint width) {
    glong length = g_utf8_strlen(value, -1);
    if (length > width) {
        guint keep = width > 3 ? width - 3 : 0;
        g_autofree char* shortened = g_utf8_substring(value, 0, keep);
        g_print("%s%s", shortened, width > 3 ? "..." : "");
    } else {
        g_print("%s", value);
        for (glong i = length; i < width; i++)
            g_print(" ");
    }
}

static void print_table(JsonArray* array) {
    guint rows = json_array_get_length(array);
    if (rows == 0) {
        g_print("No items.\n");
        return;
    }
    JsonNode* first = json_array_get_element(array, 0);
    if (!JSON_NODE_HOLDS_OBJECT(first)) {
        for (guint row = 0; row < rows; row++) {
            g_autofree char* value = node_text(json_array_get_element(array, row));
            g_print("%s\n", value);
        }
        return;
    }

    JsonObject* first_object = json_node_get_object(first);
    g_autoptr(GPtrArray) fields = g_ptr_array_new_with_free_func(g_free);
    if (json_object_has_member(first_object, "app_id") ||
        json_object_has_member(first_object, "appId")) {
        const gboolean snake_case = json_object_has_member(first_object, "app_id");
        const char* with_monitor_id[] = {
            "id", "focused", "workspace", "monitor_id", "monitor_index", "app_id", "title"};
        const char* without_monitor_id[] = {"id",     "focused", "workspace", "monitor_index",
                                            "app_id", "title"};
        const char* legacy_with_monitor_id[] = {"id",           "focused", "workspace", "monitorId",
                                                "monitorIndex", "appId",   "title"};
        const char* legacy_without_monitor_id[] = {"id",           "focused", "workspace",
                                                   "monitorIndex", "appId",   "title"};
        gboolean has_monitor_id =
            json_object_has_member(first_object, snake_case ? "monitor_id" : "monitorId");
        const char* const* ordered = has_monitor_id ? with_monitor_id : without_monitor_id;
        if (!snake_case)
            ordered = has_monitor_id ? legacy_with_monitor_id : legacy_without_monitor_id;
        guint ordered_count =
            has_monitor_id ? G_N_ELEMENTS(with_monitor_id) : G_N_ELEMENTS(without_monitor_id);
        for (guint i = 0; i < ordered_count; i++)
            g_ptr_array_add(fields, g_strdup(ordered[i]));
    } else {
        GList* names = json_object_get_members(first_object);
        for (GList* item = names; item; item = item->next)
            g_ptr_array_add(fields, g_strdup(item->data));
        g_list_free(names);
    }
    if (fields->len == 0) {
        g_print("No items.\n");
        return;
    }
    g_autofree guint* widths = g_new0(guint, fields->len);
    for (guint column = 0; column < fields->len; column++) {
        const char* field = g_ptr_array_index(fields, column);
        g_autofree char* header = table_header(field);
        widths[column] = g_utf8_strlen(header, -1);
        for (guint row = 0; row < rows; row++) {
            JsonNode* item = json_array_get_element(array, row);
            JsonObject* object = JSON_NODE_HOLDS_OBJECT(item) ? json_node_get_object(item) : NULL;
            g_autofree char* value =
                node_text(object ? json_object_get_member(object, field) : NULL);
            widths[column] = MAX(widths[column], (guint)g_utf8_strlen(value, -1));
        }
    }
    struct winsize terminal = {0};
    guint columns =
        ioctl(STDOUT_FILENO, TIOCGWINSZ, &terminal) == 0 && terminal.ws_col ? terminal.ws_col : 120;
    guint available = MAX(40, columns) - 2 * (fields->len - 1);
    guint total = 0;
    for (guint i = 0; i < fields->len; i++)
        total += widths[i];
    while (total > available) {
        guint widest = 0;
        for (guint i = 1; i < fields->len; i++)
            if (widths[i] > widths[widest])
                widest = i;
        if (widths[widest] <= 10)
            break;
        widths[widest]--;
        total--;
    }
    for (gint row = -2; row < (gint)rows; row++) {
        for (guint column = 0; column < fields->len; column++) {
            if (column)
                g_print("  ");
            const char* field = g_ptr_array_index(fields, column);
            g_autofree char* value = NULL;
            if (row == -2)
                value = table_header(field);
            else if (row == -1)
                value = g_strnfill(widths[column], '-');
            else {
                JsonNode* item = json_array_get_element(array, row);
                JsonObject* object =
                    JSON_NODE_HOLDS_OBJECT(item) ? json_node_get_object(item) : NULL;
                value = node_text(object ? json_object_get_member(object, field) : NULL);
            }
            print_cell(value, widths[column]);
        }
        g_print("\n");
    }
}

static void print_grants(JsonArray* grants) {
    guint count = json_array_get_length(grants);
    if (count == 0) {
        g_print("No items.\n");
        return;
    }
    for (guint i = 0; i < count; i++) {
        JsonNode* node = json_array_get_element(grants, i);
        if (!JSON_NODE_HOLDS_OBJECT(node))
            continue;
        JsonObject* grant = json_node_get_object(node);
        const char* fields[] = {"id", "kind", "requester", "devices", "clipboard", "screenStreams"};
        for (guint field = 0; field < G_N_ELEMENTS(fields); field++) {
            if (!json_object_has_member(grant, fields[field]))
                continue;
            g_autofree char* label = human_label(fields[field]);
            g_autofree char* value = node_text(json_object_get_member(grant, fields[field]));
            g_print("%s: %s\n", label, value);
        }
        if (i + 1 < count)
            g_print("\n");
    }
}

static gboolean boolean_member(JsonObject* object, const char* name, gboolean* value) {
    JsonNode* node = object ? json_object_get_member(object, name) : NULL;
    if (!node || !JSON_NODE_HOLDS_VALUE(node) || json_node_get_value_type(node) != G_TYPE_BOOLEAN)
        return FALSE;
    *value = json_node_get_boolean(node);
    return TRUE;
}

static gboolean print_privacy_state(JsonObject* object) {
    JsonNode* available_node = json_object_get_member(object, "available");
    if (!available_node || !JSON_NODE_HOLDS_OBJECT(available_node) ||
        !json_object_has_member(object, "revision"))
        return FALSE;

    JsonObject* available = json_node_get_object(available_node);
    static const struct {
        const char* key;
        const char* label;
    } activities[] = {
        {"screen_sharing", "Screen sharing"},
        {"microphone_in_use", "Microphone"},
        {"camera_in_use", "Camera"},
        {"location_in_use", "Location"},
    };
    for (guint i = 0; i < G_N_ELEMENTS(activities); i++) {
        gboolean source_available;
        g_autofree char* status = NULL;
        if (!boolean_member(available, activities[i].key, &source_available))
            status = g_strdup("Unknown");
        else if (!source_available)
            status = g_strdup("Unavailable");
        else {
            gboolean active;
            if (!boolean_member(object, activities[i].key, &active))
                status = g_strdup("Unknown");
            else
                status = g_strdup(active ? "In use" : "Not in use");
        }
        g_print("%s: %s\n", activities[i].label, status);
    }
    g_autofree char* revision = node_text(json_object_get_member(object, "revision"));
    g_print("Revision: %s\n", revision);
    return TRUE;
}

static void render(JsonNode* result, const char* format, gboolean raw_string) {
    if (raw_string && JSON_NODE_HOLDS_VALUE(result)) {
        g_print("%s", json_node_get_string(result));
        return;
    }
    if (g_str_equal(format, "json") ||
        (g_str_equal(format, "auto") && !isatty(STDOUT_FILENO) && !JSON_NODE_HOLDS_VALUE(result))) {
        g_autofree char* encoded = json_to_string(result, FALSE);
        g_print("%s\n", encoded);
        return;
    }
    if (JSON_NODE_HOLDS_VALUE(result)) {
        g_autofree char* value = node_text(result);
        g_print("%s\n", value);
        return;
    }
    if (JSON_NODE_HOLDS_OBJECT(result)) {
        JsonObject* object = json_node_get_object(result);
        if (print_privacy_state(object))
            return;
        GList* members = json_object_get_members(object);
        const char* array_member = NULL;
        guint array_member_count = 0;
        for (GList* item = members; item; item = item->next) {
            if (JSON_NODE_HOLDS_ARRAY(json_object_get_member(object, item->data))) {
                array_member = item->data;
                array_member_count++;
            }
        }
        if (array_member_count == 1) {
            JsonArray* array = json_node_get_array(json_object_get_member(object, array_member));
            if (g_str_equal(array_member, "grants"))
                print_grants(array);
            else
                print_table(array);
            for (GList* item = members; item; item = item->next) {
                if (g_str_equal(item->data, array_member))
                    continue;
                g_autofree char* label = human_label(item->data);
                g_autofree char* value = node_text(json_object_get_member(object, item->data));
                g_print("%s: %s\n", label, value);
            }
        } else
            for (GList* item = members; item; item = item->next) {
                g_autofree char* label = human_label(item->data);
                g_autofree char* value = node_text(json_object_get_member(object, item->data));
                g_print("%s: %s\n", label, value);
            }
        g_list_free(members);
        return;
    }
    if (JSON_NODE_HOLDS_ARRAY(result)) {
        print_table(json_node_get_array(result));
        return;
    }
    g_autofree char* value = node_text(result);
    g_print("%s\n", value);
}

static const char* action_usage(const char* command, const char* action) {
    if (g_str_equal(command, "window")) {
        if (g_str_equal(action, "list"))
            return "[--app-id ID] [--title TEXT] [--focused]";
        if (g_str_equal(action, "match"))
            return "[WINDOW]";
        if (g_str_equal(action, "toggle-minimize"))
            return "WINDOW";
        if (g_str_equal(action, "move"))
            return "WINDOW X Y";
        if (g_str_equal(action, "resize"))
            return "WINDOW WIDTH HEIGHT";
        if (g_str_equal(action, "thumbnail"))
            return "WINDOW --output PATH [--width 1..480] [--height 1..320]";
        if (g_str_equal(action, "monitor"))
            return "WINDOW CONNECTOR_ID_OR_INDEX";
        if (g_str_equal(action, "workspace"))
            return "WINDOW (--id ID | --number N | N)";
        return "[WINDOW]";
    }
    if (g_str_equal(command, "workspace")) {
        if (g_str_equal(action, "create"))
            return "--name NAME [--id ID] [--activate]";
        if (g_str_equal(action, "rename"))
            return "(--id ID | --number N) --name NAME";
        if (g_str_equal(action, "remove"))
            return "(--id ID | --number N)";
        if (g_str_equal(action, "switch"))
            return "(--id ID | --number N | N)";
        if (g_str_equal(action, "move-active"))
            return "(--id ID | --number N) [--follow]";
    }
    if (g_str_equal(command, "animation")) {
        if (g_str_equal(action, "get"))
            return "NAME";
        if (g_str_equal(action, "inspect") || g_str_equal(action, "preview"))
            return "NAME [--event EVENT] [--window WINDOW | --layer LAYER | --namespace NAME] "
                   "[--autoplay for preview]";
        if (g_str_equal(action, "seek"))
            return "SESSION PERCENT";
        if (g_str_equal(action, "step"))
            return "SESSION MILLISECONDS";
        if (word_in("play pause stop", action))
            return "SESSION";
    }
    if (g_str_equal(command, "focus") && g_str_equal(action, "history"))
        return "[--workspace-id ID] [--monitor-id ID] [--limit 1..256]";
    if (g_str_equal(command, "shortcut") && g_str_equal(action, "actions"))
        return "[wm | mutter | wayland]";
    if (g_str_equal(command, "permissions") && g_str_equal(action, "check"))
        return "CAPABILITY IDENTITY";
    if (g_str_equal(command, "grant") && g_str_equal(action, "revoke"))
        return "(screen-cast | remote-desktop) ID";
    if (g_str_equal(command, "launch")) {
        if (g_str_equal(action, "begin"))
            return "TOKEN APPLICATION [MILLISECONDS]";
        if (g_str_equal(action, "end"))
            return "TOKEN";
    }
    if (g_str_equal(command, "input") && g_str_equal(action, "select"))
        return "TYPE ID";
    return NULL;
}

static void print_help(const char* command, const char* action) {
    const CommandSpec* spec = find_command(command);
    if (!spec) {
        g_print("Usage: gnoblinctl [--json | --format auto|json|table] [--timeout SECONDS] "
                "[--socket PATH] COMMAND\n\nCommands:\n");
        for (guint i = 0; i < G_N_ELEMENTS(commands); i++)
            g_print("  %s\n", commands[i].name);
        g_print("\nUse 'gnoblinctl help COMMAND' to see its actions.\n");
        return;
    }
    if (g_str_equal(command, "lua")) {
        g_print("Usage: gnoblinctl lua [FILE]\n\n"
                "Run a local Lua console or execute a Lua file with the Gnoblin session API.\n");
        return;
    }
    if (action && word_in(spec->actions, action)) {
        const char* arguments = action_usage(command, action);
        g_print("Usage: gnoblinctl %s %s%s%s\n\n", command, action, arguments ? " " : "",
                arguments ? arguments : "");
        g_print("Options: -j, --json; --format auto|json|table; --timeout 1..60; --socket PATH\n");
        return;
    }
    if (g_str_equal(command, "completion")) {
        g_print("Usage: gnoblinctl completion bash|zsh|fish\n");
        return;
    }
    g_print("Usage: gnoblinctl %s%s\n", command,
            spec->actions ? " ACTION [OPTIONS]" : " [OPTIONS]");
    if (spec->actions) {
        g_print("\nActions:\n");
        g_auto(GStrv) parts = g_strsplit(spec->actions, " ", -1);
        for (guint i = 0; parts[i]; i++)
            g_print("  %s\n", parts[i]);
    }
}

static void print_completion(const char* shell) {
    const char* common = "-h --help -j --json --format --timeout --socket";
    GString* names = g_string_new(NULL);
    for (guint i = 0; i < G_N_ELEMENTS(commands); i++) {
        if (i)
            g_string_append_c(names, ' ');
        g_string_append(names, commands[i].name);
    }
    if (g_str_equal(shell, "bash")) {
        g_print("_gnoblinctl() {\n  local words='%s %s'\n  if (( COMP_CWORD > 1 )); then\n  case "
                "\"${COMP_WORDS[1]}\" in\n",
                names->str, common);
        for (guint i = 0; i < G_N_ELEMENTS(commands); i++)
            if (commands[i].actions)
                g_print("    %s) words='%s %s' ;;\n", commands[i].name, commands[i].actions,
                        common);
        g_print("  esac\n  fi\n  if (( COMP_CWORD > 2 )); then\n    words='%s'\n    case "
                "\"${COMP_WORDS[1]}:${COMP_WORDS[2]}\" in\n",
                common);
        g_print(
            "      window:list) words=\"$words --app-id --title --focused\" ;;\n"
            "      window:workspace) words=\"$words --id --number\" ;;\n"
            "      workspace:create) words=\"$words --id --name --activate\" ;;\n"
            "      workspace:rename) words=\"$words --id --number --name\" ;;\n"
            "      workspace:remove|workspace:switch) words=\"$words --id --number\" ;;\n"
            "      workspace:move-active) words=\"$words --id --number --follow\" ;;\n"
            "      animation:inspect) words=\"$words --event --window --layer --namespace\" ;;\n"
            "      animation:preview) words=\"$words --event --window --layer --namespace "
            "--autoplay\" ;;\n"
            "    esac\n  fi\n"
            "  COMPREPLY=( $(compgen -W \"$words\" -- \"${COMP_WORDS[COMP_CWORD]}\") )\n"
            "}\ncomplete -F _gnoblinctl gnoblinctl\n");
    } else if (g_str_equal(shell, "zsh")) {
        g_print("#compdef gnoblinctl\n_gnoblinctl() {\n  local choices='%s %s'\n  if (( CURRENT > "
                "2 )); then\n  case \"${words[2]}\" in\n",
                names->str, common);
        for (guint i = 0; i < G_N_ELEMENTS(commands); i++)
            if (commands[i].actions)
                g_print("    %s) choices='%s %s' ;;\n", commands[i].name, commands[i].actions,
                        common);
        g_print("  esac\n  fi\n  compadd ${(s: :)choices}\n}\ncompdef _gnoblinctl gnoblinctl\n");
    } else {
        g_print("complete -c gnoblinctl -f\n");
        for (guint i = 0; i < G_N_ELEMENTS(commands); i++) {
            g_print("complete -c gnoblinctl -n '__fish_use_subcommand' -a '%s'\n",
                    commands[i].name);
            if (commands[i].actions)
                g_print("complete -c gnoblinctl -n '__fish_seen_subcommand_from %s' -a '%s'\n",
                        commands[i].name, commands[i].actions);
        }
        g_print("complete -c gnoblinctl -s j -l json -d 'JSON output'\n"
                "complete -c gnoblinctl -l format -r -a 'auto json table'\n"
                "complete -c gnoblinctl -l timeout -r\n"
                "complete -c gnoblinctl -l socket -r\n");
    }
    g_string_free(names, TRUE);
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
