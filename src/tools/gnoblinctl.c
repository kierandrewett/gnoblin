/* Gnoblin control client. Uses the libraries already required by the session. */
#include <gio/gio.h>
#include <gio/gunixsocketaddress.h>
#include <json-glib/json-glib.h>
#include <glib/gstdio.h>
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
    {"window", "list match menu interactive-move interactive-resize above unabove stick unstick "
               "focus close minimize toggle-minimize restore-or-minimize restore maximize "
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
        {"window.restore_or_minimize", 38},
        {"privacy.stop_sharing", 31},
        {"privacy.stop_recording", 31},
        {"window.thumbnail", 23},
        {"shortcut.actions", 5},
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
        {"capabilities.list", 19},
        {"focus.history", 19},
        {"focus.policy", 19},
        {"settings", 19},
        {"privacy.state", 17},
        {"permissions.policy", 16},
        {"grant.list", 14},
        {"grant.revoke", 14},
        {"layer.list", 2},
        {"input.devices", 3},
        {"input.sources", 6},
        {"input.current_source", 6},
        {"input.select_source", 6},
        {"launch.status", 8},
    };

    if (g_str_has_prefix(method, "animation."))
        return 18;
    for (guint i = 0; i < G_N_ELEMENTS(methods); i++)
        if (g_str_equal(method, methods[i].name))
            return methods[i].minor;
    return 8;
}

static JsonNode* call_compositor(Cli* cli, const char* op, const char* method,
                                 JsonObject* arguments, GError** error) {
    const char* method_name = method ? method : "";
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
        g_str_equal(method_name, "input.select_source") ||
        g_str_equal(method_name, "shortcut.actions") ||
        g_str_equal(method_name, "shortcuts.actions") ||
        g_str_equal(method_name, "shortcut.capture") || g_str_equal(method_name, "shortcut.list") ||
        g_str_equal(method_name, "grant.list") || g_str_equal(method_name, "grant.revoke") ||
        g_str_equal(method_name, "permissions.policy") ||
        g_str_equal(method_name, "privacy.state") || g_str_equal(method_name, "version") ||
        g_str_equal(method_name, "session.status") || g_str_equal(method_name, "session.logout") ||
        g_str_equal(method_name, "session.activity") || g_str_equal(method_name, "session.lock") ||
        g_str_equal(method_name, "privacy.stop_sharing") ||
        g_str_equal(method_name, "privacy.stop_recording") ||
        g_str_equal(method_name, "launch.status") ||
        g_str_equal(method_name, "launches.snapshot") ||
        g_str_equal(method_name, "shortcuts.actions") ||
        g_str_equal(method_name, "shortcuts.list") ||
        g_str_equal(method_name, "capabilities.list") || g_str_equal(method_name, "windows.list") ||
        g_str_equal(method_name, "workspaces.list") || g_str_equal(method_name, "monitors.list") ||
        g_str_equal(method_name, "layers.list") || g_str_equal(method_name, "focus.history") ||
        g_str_equal(method_name, "focus.policy") || g_str_equal(method_name, "settings") ||
        g_str_equal(method_name, "runtime.reload_config") ||
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
        json_builder_end_array(subscription_builder);
        json_builder_end_object(subscription_builder);
        g_autoptr(JsonNode) subscription_request = json_builder_get_root(subscription_builder);
        g_autofree char* subscription_encoded = json_to_string(subscription_request, FALSE);
        payload = g_strdup_printf("%s\n%s\n", subscription_encoded, encoded);
    } else {
        payload = g_strconcat(encoded, "\n", NULL);
    }

    gboolean waits_for_operation =
        g_str_equal(method_name, "window.thumbnail") ||
        g_str_equal(method_name, "input.select_source") ||
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
                g_str_equal(member_string(response, "method", ""), method_name) &&
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
                        !g_str_equal(member_string(result, "method", ""), method_name)) {
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

    if (!spec->actions && g_strcmp0(cli->command, "completion") != 0 && arg_count(cli) != 0) {
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
            if (is(action, "toggle-minimize") && is(window, "active")) {
                g_set_error_literal(error, G_OPTION_ERROR, G_OPTION_ERROR_BAD_VALUE,
                                    "window toggle-minimize requires a stable window ID");
                goto invalid;
            }
            gboolean active_window = is(window, "active");
            gboolean typed = !word_in("focus menu interactive-move interactive-resize", action);
            g_autofree char* resolved_window = NULL;
            if (typed && is(window, "active")) {
                resolved_window = focused_window_id(cli, error);
                if (!resolved_window)
                    goto invalid;
                window = resolved_window;
            }
            if (!typed) {
                set_string(arguments, "action", action);
                set_string(arguments, "window", window);
                method = "window.action";
            } else {
                set_string(arguments, "id", window);
                if (is(action, "close"))
                    method = "window.close";
                else if (is(action, "minimize"))
                    method = "window.minimize";
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
            }
            if (is(action, "move") || is(action, "resize") || is(action, "monitor")) {
                if (is(action, "monitor") && typed) {
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
                    const char* first = is(action, "move")     ? "x"
                                        : is(action, "resize") ? "width"
                                                               : "monitor";
                    const char* second = is(action, "move") ? "y" : "height";
                    gint value;
                    gint low = is(action, "move") ? -100000 : is(action, "monitor") ? 0 : 1;
                    gint high = is(action, "move") ? 100000 : is(action, "monitor") ? 1024 : 32768;
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
            method = "input.select_source";
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
        (find_command(cli.command) && find_command(cli.command)->actions && !cli.action)) {
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
