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

static void render_outputs(JsonArray* outputs) {
    JsonArray* rows = json_array_new();
    for (guint i = 0; i < json_array_get_length(outputs); i++) {
        JsonObject* input = json_array_get_object_element(outputs, i);
        if (!input)
            continue;
        JsonObject* row = json_object_new();
        json_object_set_string_member(row, "id", spatial_value(input, "id"));
        json_object_set_string_member(row, "kind", spatial_value(input, "kind"));
        JsonObject* mode = member_object(input, "mode");
        g_autofree char* width = spatial_value(mode, "width");
        g_autofree char* height = spatial_value(mode, "height");
        g_autofree char* rate = spatial_value(mode, "refresh_rate");
        g_autofree char* resolution = g_strdup_printf("%sx%s@%s", width, height, rate);
        g_autofree char* x = spatial_value(input, "x");
        g_autofree char* y = spatial_value(input, "y");
        g_autofree char* position = g_strdup_printf("%s+%s", x, y);
        /* A headset has per-eye views and no place in the desktop layout. */
        JsonArray* views = json_object_has_member(input, "views")
                               ? json_object_get_array_member(input, "views")
                               : NULL;
        if (views) {
            JsonObject* first =
                json_array_get_length(views) > 0 ? json_array_get_object_element(views, 0) : NULL;
            g_autofree char* eye_width = spatial_value(first, "width");
            g_autofree char* eye_height = spatial_value(first, "height");
            g_free(resolution);
            resolution =
                g_strdup_printf("%u x %sx%s", json_array_get_length(views), eye_width, eye_height);
            g_free(position);
            position = g_strdup("-");
        }
        json_object_set_string_member(row, "resolution", resolution);
        json_object_set_string_member(row, "position", position);
        json_object_set_string_member(row, "scale", spatial_value(input, "scale"));
        gboolean primary = json_object_get_boolean_member_with_default(input, "primary", FALSE);
        gboolean enabled = json_object_get_boolean_member_with_default(input, "enabled", FALSE);
        json_object_set_string_member(row, "primary", primary ? "yes" : "no");
        json_object_set_string_member(row, "enabled", enabled ? "yes" : "no");
        JsonObject* renderer = member_object(input, "renderer");
        g_autofree char* accelerated = g_strdup(
            renderer && json_object_get_boolean_member_with_default(renderer, "accelerated", FALSE)
                ? "yes"
                : "no");
        if (views) {
            g_free(accelerated);
            accelerated = g_strdup("-");
        }
        json_object_set_string_member(row, "accelerated", accelerated);
        json_object_set_string_member(row, "renderer", spatial_value(renderer, "name"));
        JsonNode* row_node = json_node_new(JSON_NODE_OBJECT);
        json_node_take_object(row_node, row);
        json_array_add_element(rows, row_node);
    }
    print_table(rows);
    json_array_unref(rows);
}

static void render_output_primary(JsonObject* output) {
    if (!output)
        return;
    const char* labels[] = {"id",     "kind",  "name", "make", "model",
                            "serial", "scale", "x",    "y",    "monitor_id"};
    for (guint i = 0; i < G_N_ELEMENTS(labels); i++) {
        const char* member = labels[i];
        JsonNode* value = json_object_get_member(output, member);
        if (!value || JSON_NODE_HOLDS_NULL(value))
            continue;
        g_autofree char* label = human_label(member);
        g_autofree char* text = node_text(value);
        g_print("%s: %s\n", label, text);
    }
    JsonObject* mode = member_object(output, "mode");
    if (mode) {
        g_autofree char* width = spatial_value(mode, "width");
        g_autofree char* height = spatial_value(mode, "height");
        g_autofree char* rate = spatial_value(mode, "refresh_rate");
        g_print("Mode: %sx%s@%s\n", width, height, rate);
    }
    JsonObject* renderer = member_object(output, "renderer");
    if (renderer) {
        g_autofree char* name = spatial_value(renderer, "name");
        g_autofree char* vendor = spatial_value(renderer, "vendor");
        g_autofree char* api = spatial_value(renderer, "api");
        gboolean accelerated =
            json_object_get_boolean_member_with_default(renderer, "accelerated", FALSE);
        g_print("Renderer: %s%s\n", name, accelerated ? " (accelerated)" : " (software)");
        if (vendor && *vendor)
            g_print("Renderer vendor: %s\n", vendor);
        if (api && *api)
            g_print("Renderer API: %s\n", api);
    }
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
    if (is(command, "output") && is(action, "list")) {
        render_outputs(spatial_array(result, "outputs"));
        return TRUE;
    }
    if (is(command, "output") && is(action, "primary") && JSON_NODE_HOLDS_OBJECT(result)) {
        render_output_primary(json_node_get_object(result));
        return TRUE;
    }
    if (is(command, "workspace") && is(action, "list")) {
        render_workspaces(spatial_array(result, "workspaces"));
        return TRUE;
    }
    return FALSE;
}
