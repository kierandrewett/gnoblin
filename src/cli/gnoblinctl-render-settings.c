#include "gnoblinctl-internal.h"

static JsonArray* settings_array(JsonNode* result, const char* member) {
    if (JSON_NODE_HOLDS_ARRAY(result))
        return json_node_get_array(result);
    if (!JSON_NODE_HOLDS_OBJECT(result))
        return NULL;
    JsonNode* node = json_object_get_member(json_node_get_object(result), member);
    return node && JSON_NODE_HOLDS_ARRAY(node) ? json_node_get_array(node) : NULL;
}

static void settings_flatten(JsonNode* node, const char* prefix, GPtrArray* lines) {
    if (JSON_NODE_HOLDS_OBJECT(node)) {
        JsonObject* object = json_node_get_object(node);
        GList* members = json_object_get_members(object);
        for (GList* item = members; item; item = item->next) {
            const char* name = item->data;
            g_autofree char* path =
                prefix && *prefix ? g_strdup_printf("%s.%s", prefix, name) : g_strdup(name);
            settings_flatten(json_object_get_member(object, name), path, lines);
        }
        g_list_free(members);
    } else if (JSON_NODE_HOLDS_ARRAY(node)) {
        JsonArray* array = json_node_get_array(node);
        for (guint i = 0; i < json_array_get_length(array); i++) {
            g_autofree char* path = g_strdup_printf("%s[%u]", prefix, i);
            settings_flatten(json_array_get_element(array, i), path, lines);
        }
    } else {
        g_autofree char* value = node_text(node);
        g_ptr_array_add(lines, g_strdup_printf("%s: %s", prefix, value));
    }
}

static gint settings_line_compare(gconstpointer a, gconstpointer b) {
    return g_strcmp0(*(char* const*)a, *(char* const*)b);
}

static void settings_print_flat(JsonNode* node) {
    g_autoptr(GPtrArray) lines = g_ptr_array_new_with_free_func(g_free);
    settings_flatten(node, "", lines);
    g_ptr_array_sort(lines, settings_line_compare);
    for (guint i = 0; i < lines->len; i++)
        g_print("%s\n", (char*)g_ptr_array_index(lines, i));
}

static void settings_table(JsonArray* source, const char* const* fields, guint count) {
    g_autoptr(JsonArray) rows = json_array_new();
    for (guint i = 0; source && i < json_array_get_length(source); i++) {
        JsonObject* input = json_array_get_object_element(source, i);
        JsonObject* row = json_object_new();
        for (guint j = 0; j < count; j++) {
            JsonNode* node = input ? json_object_get_member(input, fields[j]) : NULL;
            if (node)
                json_object_set_member(row, fields[j], json_node_copy(node));
            else
                json_object_set_string_member(row, fields[j], "-");
        }
        json_array_add_object_element(rows, row);
    }
    print_table(rows);
}

static gboolean settings_render_config(const char* action, JsonNode* result) {
    if (g_str_equal(action, "show")) {
        if (!JSON_NODE_HOLDS_OBJECT(result))
            return FALSE;
        settings_print_flat(result);
        return TRUE;
    }
    return FALSE;
}

static gboolean settings_render_shortcuts(const char* action, JsonNode* result) {
    if (!g_str_equal(action, "list"))
        return FALSE;
    JsonArray* array = settings_array(result, "shortcuts");
    if (!array)
        return FALSE;
    const char* fields[] = {"name", "binding", "trigger", "enabled"};
    settings_table(array, fields, G_N_ELEMENTS(fields));
    return TRUE;
}

static gboolean settings_render_input(const char* action, JsonNode* result) {
    if (!g_str_equal(action, "list"))
        return FALSE;
    JsonArray* array = settings_array(result, "sources");
    if (!array)
        return FALSE;
    const char* fields[] = {"id", "type", "name", "current"};
    settings_table(array, fields, G_N_ELEMENTS(fields));
    return TRUE;
}

static gboolean settings_render_animations(const char* action, JsonNode* result) {
    if (!g_str_equal(action, "list"))
        return FALSE;
    JsonArray* array = settings_array(result, "animations");
    if (!array)
        return FALSE;
    const char* fields[] = {"name", "event", "duration", "ease", "previewable"};
    settings_table(array, fields, G_N_ELEMENTS(fields));
    return TRUE;
}

gboolean render_settings(const char* command, const char* action, JsonNode* result) {
    if (is(command, "config") && action)
        return settings_render_config(action, result);
    if (is(command, "shortcut") && action)
        return settings_render_shortcuts(action, result);
    if (is(command, "input") && action)
        return settings_render_input(action, result);
    if (is(command, "animation") && action)
        return settings_render_animations(action, result);
    return FALSE;
}
