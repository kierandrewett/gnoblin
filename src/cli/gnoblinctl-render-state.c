#include "gnoblinctl-internal.h"

static JsonObject* state_object(JsonNode* node) {
    return node && JSON_NODE_HOLDS_OBJECT(node) ? json_node_get_object(node) : NULL;
}

static void state_field(JsonObject* object, const char* key, const char* label) {
    print_key_value(object, key, label);
}

static void state_fields(JsonObject* object, const char* const* keys, const char* const* labels,
                         guint count) {
    print_key_values(object, keys, labels, count);
}

static void state_print_array(JsonNode* result) {
    if (result && JSON_NODE_HOLDS_ARRAY(result))
        print_table(json_node_get_array(result));
    else
        g_print("No items.\n");
}

static void render_capabilities(JsonNode* result) {
    JsonArray* source = JSON_NODE_HOLDS_ARRAY(result) ? json_node_get_array(result) : NULL;
    g_autoptr(JsonArray) rows = json_array_new();
    const char* fields[] = {"id", "available", "description"};
    for (guint i = 0; source && i < json_array_get_length(source); i++) {
        JsonObject* item = json_array_get_object_element(source, i);
        JsonObject* row = json_object_new();
        for (guint j = 0; j < G_N_ELEMENTS(fields); j++) {
            JsonNode* value = item ? json_object_get_member(item, fields[j]) : NULL;
            if (value)
                json_object_set_member(row, fields[j], json_node_copy(value));
            else
                json_object_set_string_member(row, fields[j], "-");
        }
        json_array_add_object_element(rows, row);
    }
    print_table(rows);
}

static void render_status(JsonNode* result) {
    const char* keys[] = {"state", "lock_state", "lock_available"};
    const char* labels[] = {"state", "lock state", "lock available"};
    state_fields(state_object(result), keys, labels, G_N_ELEMENTS(keys));
}

static void render_version(JsonNode* result) {
    const char* keys[] = {"gnoblin", "gnome",    "mutter",  "lua",
                          "api",     "build_id", "git_sha", "git_remote"};
    const char* labels[] = {"Gnoblin", "GNOME", "Mutter",  "Lua",
                            "API",     "Build", "Git SHA", "Git remote"};
    state_fields(state_object(result), keys, labels, G_N_ELEMENTS(keys));
}

static void render_focus_policy(JsonNode* result) {
    const char* keys[] = {"focus_mode", "focus_new_windows", "raise_on_click",
                          "auto_raise", "auto_raise_delay",  "focus_change_on_pointer_rest"};
    const char* labels[] = {"mode",       "new windows", "raise on click",
                            "auto raise", "raise delay", "pointer rest"};
    state_fields(state_object(result), keys, labels, G_N_ELEMENTS(keys));
}

static void render_activity(JsonNode* result) {
    const char* keys[] = {"available", "idle", "idle_for_ms", "threshold_ms"};
    const char* labels[] = {"available", "idle", "idle for (ms)", "threshold (ms)"};
    state_fields(state_object(result), keys, labels, G_N_ELEMENTS(keys));
}

static void render_privacy(JsonNode* result) {
    JsonObject* object = state_object(result);
    const char* keys[] = {"screen_sharing", "recording", "camera_in_use", "microphone_in_use"};
    const char* labels[] = {"screen sharing", "recording", "camera", "microphone"};
    state_fields(object, keys, labels, G_N_ELEMENTS(keys));
    JsonObject* available = member_object(object, "available");
    if (available) {
        g_print("\nAVAILABLE\n");
        const char* sources[] = {"screen_sharing", "recording", "camera_in_use",
                                 "microphone_in_use", "location_in_use"};
        const char* names[] = {"screen sharing", "recording", "camera", "microphone", "location"};
        state_fields(available, sources, names, G_N_ELEMENTS(sources));
    }
}

static void render_permissions_list(JsonNode* result) {
    JsonObject* object = state_object(result);
    JsonObject* policy = member_object(object, "policy");
    if (policy) {
        state_field(policy, "default", "default");
        JsonNode* rules = json_object_get_member(policy, "rules");
        if (rules && JSON_NODE_HOLDS_ARRAY(rules)) {
            g_print("\nRULES\n");
            state_print_array(rules);
        }
    }
    state_field(object, "capabilities", "capabilities");
    state_field(object, "levels", "levels");
    state_field(object, "path", "path");
}

static void render_grant(JsonNode* result) {
    JsonObject* object = state_object(result);
    JsonNode* grants = object ? json_object_get_member(object, "grants") : NULL;
    if (grants)
        state_print_array(grants);
    else {
        const char* keys[] = {"ok", "revoked", "kind", "id", "revision"};
        const char* labels[] = {"ok", "revoked", "kind", "id", "revision"};
        state_fields(object, keys, labels, G_N_ELEMENTS(keys));
    }
}

static void render_launch(JsonNode* result) {
    JsonObject* object = state_object(result);
    JsonNode* launches = object ? json_object_get_member(object, "launches") : NULL;
    if (launches) {
        state_print_array(launches);
    } else {
        const char* keys[] = {"ok",    "started",    "ended",      "token",   "application",
                              "state", "started_at", "timeout_ms", "revision"};
        state_fields(object, keys, NULL, G_N_ELEMENTS(keys));
    }
}

gboolean render_state(const char* command, const char* action, JsonNode* result) {
    if (is(command, "status")) {
        render_status(result);
        return TRUE;
    }
    if (is(command, "ping")) {
        const char* value = result && JSON_NODE_HOLDS_VALUE(result) &&
                                    json_node_get_value_type(result) == G_TYPE_STRING
                                ? json_node_get_string(result)
                                : "pong";
        g_print("%s\n", value);
        return TRUE;
    }
    if (is(command, "version")) {
        render_version(result);
        return TRUE;
    }
    if (is(command, "capabilities")) {
        render_capabilities(result);
        return TRUE;
    }
    if (is(command, "focus")) {
        if (is(action, "history"))
            state_print_array(result);
        else if (is(action, "policy"))
            render_focus_policy(result);
        else
            return FALSE;
        return TRUE;
    }
    if (is(command, "session") && is(action, "activity")) {
        render_activity(result);
        return TRUE;
    }
    if (is(command, "session") && (is(action, "lock") || is(action, "logout"))) {
        const char* keys[] = {"ok", "requested", "state", "available", "revision"};
        state_fields(state_object(result), keys, NULL, G_N_ELEMENTS(keys));
        return TRUE;
    }
    if (is(command, "privacy")) {
        if (!action || is(action, "state"))
            render_privacy(result);
        else {
            const char* keys[] = {"ok", "stopped", "count", "revision"};
            state_fields(state_object(result), keys, NULL, G_N_ELEMENTS(keys));
        }
        return TRUE;
    }
    if (is(command, "permissions")) {
        if (is(action, "list"))
            render_permissions_list(result);
        else if (is(action, "policy")) {
            JsonObject* object = state_object(result);
            state_field(object, "default", "default");
            state_field(object, "revision", "revision");
            JsonNode* rules = object ? json_object_get_member(object, "rules") : NULL;
            if (rules) {
                g_print("\nRULES\n");
                state_print_array(rules);
            }
        } else {
            const char* keys[] = {"level", "rule", "revision", "devices", "monitors", "clipboard"};
            const char* labels[] = {"level",   "rule",     "revision",
                                    "devices", "monitors", "clipboard"};
            state_fields(state_object(result), keys, labels, G_N_ELEMENTS(keys));
        }
        return TRUE;
    }
    if (is(command, "grant")) {
        render_grant(result);
        return TRUE;
    }
    if (is(command, "launch")) {
        render_launch(result);
        return TRUE;
    }
    return FALSE;
}
