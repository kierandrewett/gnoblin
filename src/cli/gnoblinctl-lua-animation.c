#include "gnoblinctl-internal.h"

int lua_cli_animations_preview(lua_State* state) {
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

int lua_cli_animations_control(lua_State* state) {
    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    const char* method = lua_tostring(state, lua_upvalueindex(2));
    if (lua_gettop(state) != 1 || !lua_istable(state, 1))
        return luaL_error(state, "gnoblin.animations.%s requires one argument table",
                          method + strlen("animation."));

    g_autoptr(JsonObject) arguments = lua_cli_table_object(state, 1, "animation control arguments");
    gboolean takes_progress = g_str_equal(method, "animation.seek");
    gboolean takes_milliseconds = g_str_equal(method, "animation.step");
    static const char* const session_fields[] = {"session", NULL};
    static const char* const progress_fields[] = {"session", "progress", NULL};
    static const char* const step_fields[] = {"session", "milliseconds", NULL};
    const char* const* allowed = takes_progress       ? progress_fields
                                 : takes_milliseconds ? step_fields
                                                      : session_fields;
    JsonNode* session_node = json_object_get_member(arguments, "session");
    if (!lua_cli_workspace_object_has_only_keys(arguments, allowed) || !session_node ||
        !JSON_NODE_HOLDS_VALUE(session_node) ||
        json_node_get_value_type(session_node) != G_TYPE_STRING ||
        !*json_node_get_string(session_node))
        return luaL_error(state,
                          "gnoblin.animations.%s requires a nonempty session and valid fields",
                          method + strlen("animation."));

    if (takes_progress) {
        JsonNode* progress_node = json_object_get_member(arguments, "progress");
        if (!progress_node || !JSON_NODE_HOLDS_VALUE(progress_node) ||
            (json_node_get_value_type(progress_node) != G_TYPE_DOUBLE &&
             json_node_get_value_type(progress_node) != G_TYPE_INT &&
             json_node_get_value_type(progress_node) != G_TYPE_INT64))
            return luaL_error(state, "gnoblin.animations.seek requires progress from 0 to 1");
        double progress = json_node_get_double(progress_node);
        if (!isfinite(progress) || progress < 0.0 || progress > 1.0)
            return luaL_error(state, "gnoblin.animations.seek requires progress from 0 to 1");
    } else if (takes_milliseconds) {
        JsonNode* milliseconds_node = json_object_get_member(arguments, "milliseconds");
        if (!milliseconds_node || !JSON_NODE_HOLDS_VALUE(milliseconds_node) ||
            (json_node_get_value_type(milliseconds_node) != G_TYPE_INT &&
             json_node_get_value_type(milliseconds_node) != G_TYPE_INT64) ||
            json_node_get_int(milliseconds_node) < 1 ||
            json_node_get_int(milliseconds_node) > 60000)
            return luaL_error(state,
                              "gnoblin.animations.step requires milliseconds from 1 to 60000");
    }

    g_autoptr(GError) call_error = NULL;
    g_autoptr(JsonNode) result = call_compositor(cli, "api", method, arguments, &call_error);
    if (!result)
        return luaL_error(state, "gnoblin.animations.%s failed: %s", method + strlen("animation."),
                          call_error->message);
    if (g_str_equal(method, "animation.stop")) {
        JsonObject* stopped = JSON_NODE_HOLDS_OBJECT(result) ? json_node_get_object(result) : NULL;
        if (!stopped || !json_object_get_boolean_member_with_default(stopped, "ok", FALSE) ||
            !g_str_equal(member_string(stopped, "session", ""), json_node_get_string(session_node)))
            return luaL_error(state, "gnoblin.animations.stop returned an invalid result");
        json_to_lua(state, result);
        lua_cli_push_readonly_value(state, -1);
        lua_remove(state, -2);
        return 1;
    }
    if (!JSON_NODE_HOLDS_OBJECT(result))
        return luaL_error(state, "gnoblin.animations.%s returned an invalid AnimationPreview",
                          method + strlen("animation."));
    JsonObject* preview = json_node_get_object(result);
    const char* session = member_string(preview, "session", NULL);
    if (!session)
        session = member_string(preview, "id", NULL);
    lua_cli_push_animation_preview_record(state, cli, preview, session);
    return 1;
}

int lua_cli_animations_list(lua_State* state) {
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

int lua_cli_animations_get(lua_State* state) {
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

static gboolean lua_cli_animation_inspect_arguments_valid(JsonObject* arguments) {
    static const char* const fields[] = {"name", "target", "event", "target_type", NULL};
    GList* members = json_object_get_members(arguments);
    gboolean valid = TRUE;
    for (GList* item = members; item && valid; item = item->next) {
        gboolean found = FALSE;
        for (guint i = 0; fields[i]; i++)
            found |= g_str_equal(item->data, fields[i]);
        valid = found;
    }
    g_list_free(members);
    const char* name = member_string(arguments, "name", NULL);
    const char* target = member_string(arguments, "target", "active");
    const char* event = member_string(arguments, "event", NULL);
    const char* target_type = member_string(arguments, "target_type", "window");
    const char* optional_fields[] = {"target", "event", "target_type", NULL};
    for (guint i = 0; optional_fields[i]; i++) {
        JsonNode* field = json_object_get_member(arguments, optional_fields[i]);
        if (field &&
            (!JSON_NODE_HOLDS_VALUE(field) || json_node_get_value_type(field) != G_TYPE_STRING))
            return FALSE;
    }
    return valid && name && g_regex_match_simple("^[A-Za-z0-9_-]{1,80}$", name, 0, 0) && target &&
           *target && g_utf8_validate(target, -1, NULL) && strlen(target) <= 128 &&
           (!json_object_has_member(arguments, "event") || (event && *event)) &&
           (g_str_equal(target_type, "window") || g_str_equal(target_type, "layer") ||
            g_str_equal(target_type, "namespace"));
}

static gboolean lua_cli_animation_inspection_valid(JsonObject* result, JsonObject* arguments) {
    const char* name = member_string(arguments, "name", NULL);
    const char* requested_event = member_string(arguments, "event", NULL);
    const char* requested_target_type = member_string(arguments, "target_type", "window");
    JsonNode* duration = json_object_get_member(result, "duration");
    JsonNode* ease = json_object_get_member(result, "ease");
    JsonNode* from = json_object_get_member(result, "from");
    JsonNode* to = json_object_get_member(result, "to");
    JsonNode* properties = json_object_get_member(result, "properties");
    JsonNode* context = json_object_get_member(result, "context");
    JsonNode* spec = json_object_get_member(result, "spec");
    const char* event = member_string(result, "event", NULL);
    const char* target = member_string(result, "target", NULL);
    const char* target_type = member_string(result, "target_type", NULL);
    gboolean valid_ease = FALSE;
    if (ease && JSON_NODE_HOLDS_VALUE(ease) && json_node_get_value_type(ease) == G_TYPE_STRING) {
        const char* curves[] = {"linear",
                                "ease-in-quad",
                                "ease-out-quad",
                                "ease-in-out-cubic",
                                "ease-in-cubic",
                                "ease-out-cubic",
                                "ease-out-expo",
                                "ease-out-back",
                                NULL};
        const char* curve = json_node_get_string(ease);
        for (guint i = 0; curves[i]; i++)
            valid_ease |= g_str_equal(curve, curves[i]);
    } else if (ease && JSON_NODE_HOLDS_OBJECT(ease)) {
        valid_ease =
            g_str_equal(member_string(json_node_get_object(ease), "type", ""), "cubic-bezier");
    }
    if (!name || !g_str_equal(member_string(result, "name", ""), name) || !event || !*event ||
        (requested_event && !g_str_equal(event, requested_event)) || !target || !*target ||
        !target_type || !g_str_equal(target_type, requested_target_type) || !duration ||
        !JSON_NODE_HOLDS_VALUE(duration) ||
        (json_node_get_value_type(duration) != G_TYPE_INT &&
         json_node_get_value_type(duration) != G_TYPE_INT64) ||
        json_node_get_int(duration) < 0 || !valid_ease || !from || !JSON_NODE_HOLDS_OBJECT(from) ||
        !to || !JSON_NODE_HOLDS_OBJECT(to) || !properties || !JSON_NODE_HOLDS_OBJECT(properties) ||
        !context || !JSON_NODE_HOLDS_OBJECT(context) || !spec || !JSON_NODE_HOLDS_OBJECT(spec))
        return FALSE;
    JsonNode* keyframes = json_object_get_member(result, "keyframes");
    return !keyframes || JSON_NODE_HOLDS_ARRAY(keyframes);
}

int lua_cli_animations_inspect(lua_State* state) {
    if (lua_gettop(state) != 1 || !lua_istable(state, 1))
        return luaL_error(state, "gnoblin.animations.inspect requires one spec table");
    g_autoptr(JsonObject) arguments = lua_cli_table_object(state, 1, "animation inspection spec");
    if (!lua_cli_animation_inspect_arguments_valid(arguments))
        return luaL_error(state, "gnoblin.animations.inspect received invalid arguments");
    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    g_autoptr(GError) call_error = NULL;
    g_autoptr(JsonNode) result =
        call_compositor(cli, "api", "animation.inspect", arguments, &call_error);
    if (!result)
        return luaL_error(state, "gnoblin.animations.inspect failed: %s", call_error->message);
    if (!JSON_NODE_HOLDS_OBJECT(result) ||
        !lua_cli_animation_inspection_valid(json_node_get_object(result), arguments))
        return luaL_error(state, "gnoblin.animations.inspect returned an invalid inspection");
    json_to_lua(state, result);
    lua_cli_push_readonly_value(state, -1);
    lua_remove(state, -2);
    return 1;
}

int lua_cli_animations_surfaces(lua_State* state) {
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

int lua_cli_launches_read(lua_State* state) {
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

int lua_cli_launches_operation(lua_State* state) {
    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    const char* method = lua_tostring(state, lua_upvalueindex(2));
    gboolean beginning = g_str_equal(method, "launch.begin");
    JsonObject* arguments = NULL;
    static const char* const begin_fields[] = {"token", "application", "timeout_ms", NULL};

    if (beginning) {
        if (lua_gettop(state) != 1 || !lua_istable(state, 1))
            return luaL_error(state, "gnoblin.launches.begin requires an options table");
        g_autoptr(JsonObject) options = lua_cli_table_object(state, 1, "launch options");
        const char* token = member_string(options, "token", NULL);
        const char* application = member_string(options, "application", NULL);
        JsonNode* timeout = json_object_get_member(options, "timeout_ms");
        gboolean valid = lua_cli_workspace_object_has_only_keys(options, begin_fields) && token &&
                         *token && g_utf8_validate(token, -1, NULL) &&
                         g_utf8_strlen(token, -1) <= 128 && application && *application &&
                         g_utf8_validate(application, -1, NULL) &&
                         g_utf8_strlen(application, -1) <= 512;
        if (timeout)
            valid = valid && JSON_NODE_HOLDS_VALUE(timeout) &&
                    (json_node_get_value_type(timeout) == G_TYPE_INT ||
                     json_node_get_value_type(timeout) == G_TYPE_INT64) &&
                    json_node_get_int(timeout) >= 100 && json_node_get_int(timeout) <= 10000;
        if (!valid)
            return luaL_error(state, "gnoblin.launches.begin received invalid launch options");

        arguments = json_object_new();
        json_object_set_string_member(arguments, "token", token);
        json_object_set_string_member(arguments, "application", application);
        if (timeout)
            json_object_set_int_member(arguments, "milliseconds", json_node_get_int(timeout));
    } else {
        if (lua_gettop(state) != 1 || lua_type(state, 1) != LUA_TSTRING ||
            !*lua_tostring(state, 1) || g_utf8_strlen(lua_tostring(state, 1), -1) > 128)
            return luaL_error(state,
                              "gnoblin.launches.finish requires a non-empty launch token of at "
                              "most 128 characters");
        arguments = json_object_new();
        json_object_set_string_member(arguments, "token", lua_tostring(state, 1));
    }

    g_autoptr(GError) call_error = NULL;
    g_autoptr(JsonNode) result = call_compositor(cli, "api", method, arguments, &call_error);
    json_object_unref(arguments);
    if (!result)
        return luaL_error(state, "%s failed: %s", method, call_error->message);

    JsonObject* response = JSON_NODE_HOLDS_OBJECT(result) ? json_node_get_object(result) : NULL;
    if (beginning) {
        if (!response || !lua_cli_launch_record_valid(response))
            return luaL_error(state, "launch.begin returned an invalid Launch record");
    } else {
        JsonNode* ok = response ? json_object_get_member(response, "ok") : NULL;
        if (!response || json_object_get_size(response) != 2 || !ok || !JSON_NODE_HOLDS_VALUE(ok) ||
            json_node_get_value_type(ok) != G_TYPE_BOOLEAN || !json_node_get_boolean(ok) ||
            !g_str_equal(member_string(response, "token", ""), lua_tostring(state, 1)))
            return luaL_error(state, "launch.end returned an invalid result");
    }

    json_to_lua(state, result);
    lua_cli_push_readonly_value(state, -1);
    lua_remove(state, -2);
    return 1;
}
