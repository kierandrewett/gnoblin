#include "gnoblinctl-internal.h"

char* focused_window_id(Cli* cli, GError** error) {
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

JsonNode* window_match_from_snapshot(JsonArray* windows, const char* selector, GError** error) {
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

char* monitor_id_for_index(Cli* cli, guint monitor_index, GError** error) {
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

gboolean is(const char* left, const char* right) {
    return g_strcmp0(left, right) == 0;
}

const char* arg(Cli* cli, guint index) {
    const CommandSpec* spec = find_command(cli->command);
    guint offset = (spec && spec->actions ? 2 : 1) + index;
    return offset < cli->positionals->len ? g_ptr_array_index(cli->positionals, offset) : NULL;
}

guint arg_count(Cli* cli) {
    const CommandSpec* spec = find_command(cli->command);
    guint offset = spec && spec->actions ? 2 : 1;
    return cli->positionals->len > offset ? cli->positionals->len - offset : 0;
}

gboolean validate_cli(Cli* cli, GError** error) {
    const CommandSpec* spec = find_command(cli->command);
    if (!spec) {
        g_autoptr(GString) names = g_string_new(NULL);
        for (guint i = 0; i < commands_count; i++)
            g_string_append_printf(names, "%s%s", i ? " " : "", commands[i].name);
        g_autofree char* guess = closest_word(cli->command, names->str);
        g_autofree char* hint = guess ? g_strdup_printf(" (did you mean '%s'?)", guess) : NULL;
        g_set_error(error, G_OPTION_ERROR, G_OPTION_ERROR_UNKNOWN_OPTION,
                    "unknown command: %s%s\nRun 'gnoblinctl help' to list the commands.",
                    cli->command, hint ? hint : "");
        return FALSE;
    }
    if (spec->actions && cli->action && !word_in(spec->actions, cli->action)) {
        g_autofree char* guess = closest_word(cli->action, spec->actions);
        g_autofree char* hint = guess ? g_strdup_printf(" (did you mean '%s'?)", guess) : NULL;
        g_set_error(error, G_OPTION_ERROR, G_OPTION_ERROR_UNKNOWN_OPTION,
                    "unknown %s action: %s%s\nRun 'gnoblinctl help %s' to list the actions.",
                    cli->command, cli->action, hint ? hint : "", cli->command);
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

gboolean require_count(Cli* cli, guint minimum, guint maximum, GError** error) {
    guint count = arg_count(cli);
    if (count >= minimum && count <= maximum)
        return TRUE;
    g_set_error(error, G_OPTION_ERROR, G_OPTION_ERROR_BAD_VALUE, "%s %s expects %u%s argument%s",
                cli->command, cli->action ? cli->action : "", minimum,
                minimum == maximum ? "" : " or more", minimum == 1 ? "" : "s");
    return FALSE;
}

gboolean number_arg(Cli* cli, guint index, const char* name, gint low, gint high, gint* result,
                    GError** error) {
    if (parse_int(arg(cli, index), low, high, result))
        return TRUE;
    g_set_error(error, G_OPTION_ERROR, G_OPTION_ERROR_BAD_VALUE, "%s must be between %d and %d",
                name, low, high);
    return FALSE;
}

JsonNode* string_node(const char* value) {
    JsonNode* node = json_node_new(JSON_NODE_VALUE);
    json_node_set_string(node, value);
    return node;
}

static JsonNode* object_node(JsonObject* object) {
    JsonNode* node = json_node_new(JSON_NODE_OBJECT);
    json_node_take_object(node, object);
    return node;
}

JsonObject* new_object(void) {
    return json_object_new();
}

void set_string(JsonObject* object, const char* name, const char* value) {
    json_object_set_string_member(object, name, value);
}

void set_boolean(JsonObject* object, const char* name, gboolean value) {
    json_object_set_boolean_member(object, name, value);
}

void set_number(JsonObject* object, const char* name, gint64 value) {
    json_object_set_int_member(object, name, value);
}

void set_if(JsonObject* object, const char* name, const char* value) {
    if (value)
        set_string(object, name, value);
}

gboolean workspace_selector(Cli* cli, guint positional, JsonObject* object, GError** error) {
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

char* config_path(void) {
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

char* default_config(GError** error) {
    (void)error;
    gsize length = 0;
    const char* source = gnoblin_config_default_lua(&length);
    return g_strndup(source, length);
}

/* A bundled default is a directory tree.  Keep whole-tree operations confined
 * to the standard XDG Gnoblin directory; GNOBLIN_CONFIG may name one Lua file
 * and must never make restore-default replace an arbitrary parent directory. */
static char* default_config_directory(GError** error) {
    g_autofree char* path = config_path();
    const char* config_home = g_getenv("XDG_CONFIG_HOME");
    g_autofree char* expected =
        config_home && *config_home
            ? g_build_filename(config_home, "gnoblin", "init.lua", NULL)
            : g_build_filename(g_get_home_dir(), ".config", "gnoblin", "init.lua", NULL);
    g_autofree char* canonical_path = g_canonicalize_filename(path, NULL);
    g_autofree char* canonical_expected = g_canonicalize_filename(expected, NULL);
    if (!g_str_equal(canonical_path, canonical_expected)) {
        g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                    "gnoblinctl init and config restore-default require %s; GNOBLIN_CONFIG "
                    "selects a single file and cannot safely replace its parent directory",
                    canonical_expected);
        return NULL;
    }
    return g_path_get_dirname(canonical_path);
}

static gboolean write_default_config_tree(const char* directory, GError** error) {
    gsize count = 0;
    const GnoblinConfigDefaultFile* files = gnoblin_config_default_files(&count);
    for (gsize i = 0; i < count; i++) {
        if (!files[i].path || !*files[i].path || g_path_is_absolute(files[i].path) ||
            strstr(files[i].path, "..")) {
            g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                                "embedded default config has an unsafe path");
            return FALSE;
        }
        g_autofree char* target = g_build_filename(directory, files[i].path, NULL);
        g_autofree char* parent = g_path_get_dirname(target);
        if (g_mkdir_with_parents(parent, 0700) != 0) {
            g_set_error(error, G_FILE_ERROR, g_file_error_from_errno(errno),
                        "could not create default config directory %s: %s", parent,
                        g_strerror(errno));
            return FALSE;
        }
        if (!g_file_set_contents_full(target, files[i].contents, (gssize)files[i].length,
                                      G_FILE_SET_CONTENTS_CONSISTENT | G_FILE_SET_CONTENTS_DURABLE,
                                      0600, error))
            return FALSE;
    }
    return TRUE;
}

static char* stage_default_config_tree(const char* config_directory, GError** error) {
    g_autofree char* parent = g_path_get_dirname(config_directory);
    if (g_mkdir_with_parents(parent, 0700) != 0) {
        g_set_error(error, G_FILE_ERROR, g_file_error_from_errno(errno),
                    "could not create config parent directory %s: %s", parent, g_strerror(errno));
        return NULL;
    }
    g_autofree char* template = g_build_filename(parent, ".gnoblin.default.XXXXXX", NULL);
    char* staged = g_mkdtemp(template);
    if (!staged) {
        g_set_error(error, G_FILE_ERROR, g_file_error_from_errno(errno),
                    "could not stage embedded default config: %s", g_strerror(errno));
        return NULL;
    }
    if (!write_default_config_tree(staged, error)) {
        g_autoptr(GFile) staged_file = g_file_new_for_path(staged);
        g_file_delete(staged_file, NULL, NULL);
        return NULL;
    }
    return g_steal_pointer(&template);
}

gboolean initialize_config(GError** error) {
    g_autofree char* directory = default_config_directory(error);
    if (!directory)
        return FALSE;
    if (g_file_test(directory, G_FILE_TEST_EXISTS)) {
        g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_EXIST,
                    "configuration directory already exists at %s; it was left unchanged",
                    directory);
        return FALSE;
    }
    g_autofree char* staged = stage_default_config_tree(directory, error);
    if (!staged)
        return FALSE;
    if (g_rename(staged, directory) != 0) {
        g_set_error(error, G_FILE_ERROR, g_file_error_from_errno(errno),
                    "could not initialize config at %s: %s", directory, g_strerror(errno));
        return FALSE;
    }
    g_print("Initialized Gnoblin config at %s\n", directory);
    return TRUE;
}

gboolean restore_default_config(GError** error) {
    g_autofree char* directory = default_config_directory(error);
    if (!directory)
        return FALSE;
    if (!g_file_test(directory, G_FILE_TEST_EXISTS))
        return initialize_config(error);
    if (!g_file_test(directory, G_FILE_TEST_IS_DIR)) {
        g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                    "configuration path %s is not a directory", directory);
        return FALSE;
    }
    g_autofree char* staged = stage_default_config_tree(directory, error);
    if (!staged)
        return FALSE;
    g_autofree char* uuid = g_uuid_string_random();
    g_autofree char* backup = g_strdup_printf("%s.recovery-%s", directory, uuid);
    if (g_rename(directory, backup) != 0) {
        g_set_error(error, G_FILE_ERROR, g_file_error_from_errno(errno),
                    "could not back up config directory %s: %s", directory, g_strerror(errno));
        return FALSE;
    }
    if (g_rename(staged, directory) != 0) {
        int saved_errno = errno;
        if (g_rename(backup, directory) != 0)
            g_warning("gnoblinctl: could not restore original config from %s", backup);
        g_set_error(error, G_FILE_ERROR, g_file_error_from_errno(saved_errno),
                    "could not restore embedded default config at %s: %s", directory,
                    g_strerror(saved_errno));
        return FALSE;
    }
    g_print("Restored the embedded default config at %s\n", directory);
    g_print("Previous config saved to %s\n", backup);
    return TRUE;
}
