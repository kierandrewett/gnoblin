#include "gnoblinctl-internal.h"

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

int run_lua_console(Cli* cli, const char* file) {
    lua_State* state = luaL_newstate();
    if (!state) {
        g_printerr("gnoblinctl lua: could not create Lua state\n");
        return 1;
    }
    cli->lua_state = state;
    cli->lua_event_loop = g_main_loop_new(NULL, FALSE);
    cli->lua_event_subscriptions = g_ptr_array_new_with_free_func(cli_lua_event_subscription_free);
    luaL_openlibs(state);
    register_lua_cli_readonly_table(state);
    register_lua_cli_event_subscription(state);
    register_lua_cli_focus_context(state);
    register_lua_cli_menu_context(state);
    register_lua_cli_snap_context(state);
    register_lua_cli_text_target(state);
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
            cli_lua_events_clear(cli);
            cli->lua_state = NULL;
            lua_close(state);
            return 1;
        }
        gboolean succeeded = cli_lua_event_loop(cli);
        cli_lua_events_clear(cli);
        cli->lua_state = NULL;
        lua_close(state);
        return succeeded ? 0 : 1;
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
        if ((cli->active_lua_event_subscriptions ||
             (cli->lua_shortcut_bindings && g_hash_table_size(cli->lua_shortcut_bindings) > 0)) &&
            !cli_lua_event_loop(cli))
            exit_status = 1;
    }
    free(line);
    cli_lua_events_clear(cli);
    cli->lua_state = NULL;
    lua_close(state);
    return exit_status;
}

char* node_text(JsonNode* node) {
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

char* human_label(const char* name) {
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
    if (g_str_equal(name, "app"))
        return g_strdup("APP ID");
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
    if (!value || !*value)
        value = "-";
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

void print_table(JsonArray* array) {
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

void render(const char* command, const char* action, JsonNode* result, const char* format,
            gboolean raw_string) {
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
    if (render_command(command, action, result))
        return;
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
    if (g_str_equal(command, "input") && g_str_equal(action, "orientation-lock"))
        return "[on | off | inherit]";
    return NULL;
}

static void print_options(void) {
    g_print(
        "Options:\n"
        "  -j, --json           Print JSON, including in a terminal\n"
        "      --format FORMAT  auto, json or table. auto prints lists and records as tables in a\n"
        "                       terminal and as JSON in a pipe; a single value stays plain text\n"
        "      --timeout SECS   Request timeout, 1 to 60 (default 5; 30 for shortcut capture)\n"
        "      --socket PATH    Compositor socket path\n"
        "  -h, --help           Show this help\n");
}

void print_help(const char* command, const char* action) {
    const CommandSpec* spec = find_command(command);
    if (!spec) {
        g_print("gnoblinctl - control a running Gnoblin session\n\n"
                "Usage: gnoblinctl [OPTIONS] COMMAND [ACTION] [ARGUMENTS]\n\nCommands:\n");
        for (guint i = 0; i < commands_count; i++)
            g_print("  %-14s%s\n", commands[i].name, help_summary(commands[i].name, NULL));
        g_print("\n");
        print_options();
        g_print("      --version        Show the installed build, without a running session\n"
                "\nExamples:\n"
                "  gnoblinctl ping\n"
                "  gnoblinctl window list\n"
                "  gnoblinctl config path\n"
                "\nUse 'gnoblinctl help COMMAND' to list the actions of a command.\n");
        return;
    }
    const char* summary = help_summary(command, NULL);
    if (g_str_equal(command, "lua")) {
        g_print("%s.\n\nUsage: gnoblinctl lua [FILE]\n", summary);
        return;
    }
    if (g_str_equal(command, "completion")) {
        g_print("%s.\n\nUsage: gnoblinctl completion bash|zsh|fish\n", summary);
        return;
    }
    if (action && word_in(spec->actions, action)) {
        const char* arguments = action_usage(command, action);
        g_print("%s.\n\nUsage: gnoblinctl %s %s%s%s\n\n", help_summary(command, action), command,
                action, arguments ? " " : "", arguments ? arguments : "");
        print_options();
        return;
    }
    g_print("%s.\n\nUsage: gnoblinctl %s%s\n", summary, command,
            spec->actions ? " ACTION [OPTIONS]" : " [OPTIONS]");
    if (spec->actions) {
        if (g_str_equal(command, "privacy"))
            g_print("\nWith no action, it shows the privacy indicators.\n");
        g_print("\nActions:\n");
        g_auto(GStrv) parts = g_strsplit(spec->actions, " ", -1);
        guint width = 0;
        for (guint i = 0; parts[i]; i++)
            width = MAX(width, (guint)strlen(parts[i]));
        for (guint i = 0; parts[i]; i++) {
            const char* line = help_summary(command, parts[i]);
            g_print("  %-*s  %s\n", (int)width, parts[i], line ? line : "");
        }
        g_print("\nUse 'gnoblinctl help %s ACTION' for the arguments of an action.\n", command);
    } else {
        g_print("\n");
        print_options();
    }
}

void print_completion(const char* shell) {
    const char* common = "-h --help -j --json --format --timeout --socket";
    GString* names = g_string_new(NULL);
    for (guint i = 0; i < commands_count; i++) {
        if (i)
            g_string_append_c(names, ' ');
        g_string_append(names, commands[i].name);
    }
    if (g_str_equal(shell, "bash")) {
        g_print("_gnoblinctl() {\n  local words='%s %s'\n  if (( COMP_CWORD > 1 )); then\n  case "
                "\"${COMP_WORDS[1]}\" in\n",
                names->str, common);
        for (guint i = 0; i < commands_count; i++)
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
        for (guint i = 0; i < commands_count; i++)
            if (commands[i].actions)
                g_print("    %s) choices='%s %s' ;;\n", commands[i].name, commands[i].actions,
                        common);
        g_print("  esac\n  fi\n  compadd ${(s: :)choices}\n}\ncompdef _gnoblinctl gnoblinctl\n");
    } else {
        g_print("complete -c gnoblinctl -f\n");
        for (guint i = 0; i < commands_count; i++) {
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
