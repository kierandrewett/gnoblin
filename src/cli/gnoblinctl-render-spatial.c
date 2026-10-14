#include "gnoblinctl-internal.h"

static JsonArray* spatial_array(JsonNode* result, const char* key) {
    JsonObject* object = JSON_NODE_HOLDS_OBJECT(result) ? json_node_get_object(result) : NULL;
    JsonNode* node = object ? json_object_get_member(object, key) : NULL;
    return node && JSON_NODE_HOLDS_ARRAY(node) ? json_node_get_array(node) : NULL;
}

static JsonNode* spatial_member(JsonObject* object, const char* key) {
    return object ? json_object_get_member(object, key) : NULL;
}

static char* spatial_value(JsonObject* object, const char* key) {
    return node_text(spatial_member(object, key));
}

static char* spatial_geometry(JsonObject* object, const char* key) {
    JsonObject* geometry = member_object(object, key);
    if (!geometry)
        return g_strdup("-");
    g_autofree char* width = spatial_value(geometry, "width");
    g_autofree char* height = spatial_value(geometry, "height");
    g_autofree char* x = spatial_value(geometry, "x");
    g_autofree char* y = spatial_value(geometry, "y");
    return g_strdup_printf("%sx%s+%s+%s", width, height, x, y);
}

static char* spatial_anchor(JsonObject* object) {
    JsonNode* node = spatial_member(object, "anchor");
    if (!node || !JSON_NODE_HOLDS_ARRAY(node))
        return g_strdup("-");
    JsonArray* anchors = json_node_get_array(node);
    gboolean top = FALSE, bottom = FALSE, left = FALSE, right = FALSE;
    for (guint i = 0; i < json_array_get_length(anchors); i++) {
        JsonNode* item = json_array_get_element(anchors, i);
        const char* value = item && JSON_NODE_HOLDS_VALUE(item) ? json_node_get_string(item) : "";
        top |= g_str_equal(value, "top");
        bottom |= g_str_equal(value, "bottom");
        left |= g_str_equal(value, "left");
        right |= g_str_equal(value, "right");
    }
    if (top && bottom && left && right)
        return g_strdup("all");
    GString* value = g_string_new(NULL);
    const char* names[] = {"top", "bottom", "left", "right"};
    gboolean flags[] = {top, bottom, left, right};
    for (guint i = 0; i < G_N_ELEMENTS(names); i++) {
        if (!flags[i])
            continue;
        if (value->len)
            g_string_append_c(value, ',');
        g_string_append(value, names[i]);
    }
    return g_string_free(value, FALSE);
}

static JsonArray* spatial_table(JsonArray* source, const char* const* fields, guint count) {
    JsonArray* rows = json_array_new();
    for (guint i = 0; source && i < json_array_get_length(source); i++) {
        JsonObject* input = json_array_get_object_element(source, i);
        JsonObject* row = json_object_new();
        for (guint j = 0; input && j < count; j++) {
            const char* field = fields[j];
            JsonNode* value = json_object_get_member(input, field);
            if (g_str_equal(field, "geometry")) {
                JsonObject* geometry = member_object(input, "geometry");
                if (!geometry)
                    geometry = member_object(input, "frame");
                g_autofree char* text = spatial_geometry(
                    input, geometry == member_object(input, "frame") ? "frame" : "geometry");
                json_object_set_string_member(row, field, text);
            } else if (g_str_equal(field, "anchor")) {
                g_autofree char* text = spatial_anchor(input);
                json_object_set_string_member(row, field, text);
            } else if (g_str_equal(field, "resolution")) {
                g_autofree char* w = spatial_value(input, "width");
                g_autofree char* h = spatial_value(input, "height");
                g_autofree char* text = g_strdup_printf("%sx%s", w, h);
                json_object_set_string_member(row, field, text);
            } else if (g_str_equal(field, "position")) {
                g_autofree char* x = spatial_value(input, "x");
                g_autofree char* y = spatial_value(input, "y");
                g_autofree char* text = g_strdup_printf("%s+%s", x, y);
                json_object_set_string_member(row, field, text);
            } else if (g_str_equal(field, "workspace")) {
                value = json_object_get_member(input, "workspace_number");
                if (!value)
                    value = json_object_get_member(input, "workspace_id");
                if (value)
                    json_object_set_member(row, field, json_node_copy(value));
            } else if (g_str_equal(field, "app")) {
                value = json_object_get_member(input, "app_id");
                if (value)
                    json_object_set_member(row, field, json_node_copy(value));
            } else if (value) {
                json_object_set_member(row, field, json_node_copy(value));
            }
            if (!json_object_has_member(row, field))
                json_object_set_string_member(row, field, "-");
        }
        json_array_add_object_element(rows, row);
    }
    return rows;
}

static void render_windows(JsonArray* windows) {
    const char* fields[] = {"id", "focused", "workspace", "monitor_id", "app", "title", "geometry"};
    g_autoptr(JsonArray) rows = spatial_table(windows, fields, G_N_ELEMENTS(fields));
    print_table(rows);
}

static void render_window_match(JsonNode* result) {
    JsonObject* object = JSON_NODE_HOLDS_OBJECT(result) ? json_node_get_object(result) : NULL;
    JsonObject* match = member_object(object, "match");
    if (!object || !match)
        return;
    g_autofree char* id = spatial_value(object, "id");
    g_autofree char* app = spatial_value(match, "app_id");
    g_autofree char* title = spatial_value(match, "title");
    g_autofree char* focused = spatial_value(match, "focused");
    g_print("ID: %s\nAPP ID: %s\nTITLE: %s\nFOCUSED: %s\n", id, app, title, focused);
}

static void render_window_result(JsonNode* result) {
    JsonObject* object = JSON_NODE_HOLDS_OBJECT(result) ? json_node_get_object(result) : NULL;
    if (!object)
        return;
    GList* keys = json_object_get_members(object);
    for (GList* item = keys; item; item = item->next) {
        const char* key = item->data;
        JsonNode* value = json_object_get_member(object, key);
        if (JSON_NODE_HOLDS_OBJECT(value) || JSON_NODE_HOLDS_ARRAY(value))
            continue;
        g_autofree char* label = human_label(key);
        g_autofree char* text = node_text(value);
        g_print("%s: %s\n", label, text);
    }
    g_list_free(keys);
}

static void render_layers(JsonArray* layers) {
    const char* fields[] = {"id",       "namespace", "layer", "monitor_id",
                            "geometry", "anchor",    "mapped"};
    g_autoptr(JsonArray) rows = spatial_table(layers, fields, G_N_ELEMENTS(fields));
    print_table(rows);
}

static void render_monitors(JsonArray* monitors) {
    const char* fields[] = {"id",       "index", "name",    "resolution",
                            "position", "scale", "primary", "enabled"};
    g_autoptr(JsonArray) rows = spatial_table(monitors, fields, G_N_ELEMENTS(fields));
    print_table(rows);
}

static void render_workspaces(JsonArray* workspaces) {
    const char* fields[] = {"id", "number", "name", "active", "windows"};
    g_autoptr(JsonArray) rows = spatial_table(workspaces, fields, G_N_ELEMENTS(fields));
    print_table(rows);
}

gboolean render_spatial(const char* command, const char* action, JsonNode* result) {
    if (is(command, "window") && is(action, "list")) {
        render_windows(spatial_array(result, "windows"));
        return TRUE;
    }
    if (is(command, "window") && is(action, "match")) {
        render_window_match(result);
        return TRUE;
    }
    if (is(command, "window") && JSON_NODE_HOLDS_OBJECT(result)) {
        render_window_result(result);
        return TRUE;
    }
    if (is(command, "layer") && is(action, "list")) {
        render_layers(spatial_array(result, "layers"));
        return TRUE;
    }
    if (is(command, "monitor") && is(action, "list")) {
        render_monitors(spatial_array(result, "monitors"));
        return TRUE;
    }
    if (is(command, "workspace") && is(action, "list")) {
        render_workspaces(spatial_array(result, "workspaces"));
        return TRUE;
    }
    return FALSE;
}
