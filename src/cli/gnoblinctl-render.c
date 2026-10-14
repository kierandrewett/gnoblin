#include "gnoblinctl-internal.h"

/* Terminal output for one command. Each group lives in its own file so that renderers
 * stay small: spatial (window, layer, monitor, workspace), state (status, session, privacy,
 * permissions, grants, focus, launch) and settings (config, shortcut, input, animation). */
gboolean render_command(const char* command, const char* action, JsonNode* result) {
    return render_spatial(command, action, result) || render_state(command, action, result) ||
           render_settings(command, action, result);
}

void print_key_value(JsonObject* object, const char* key, const char* label) {
    JsonNode* node = object ? json_object_get_member(object, key) : NULL;
    if (!node)
        return;
    g_autofree char* value = node_text(node);
    g_autofree char* name = g_ascii_strup(label ? label : key, -1);
    g_print("%s  %s\n", name, value);
}

void print_key_values(JsonObject* object, const char* const* keys, const char* const* labels,
                      guint count) {
    guint width = 0;
    for (guint i = 0; object && i < count; i++) {
        if (!json_object_has_member(object, keys[i]))
            continue;
        g_autofree char* name = g_ascii_strup(labels ? labels[i] : keys[i], -1);
        width = MAX(width, (guint)g_utf8_strlen(name, -1));
    }
    for (guint i = 0; object && i < count; i++) {
        JsonNode* node = json_object_get_member(object, keys[i]);
        if (!node)
            continue;
        g_autofree char* name = g_ascii_strup(labels ? labels[i] : keys[i], -1);
        g_autofree char* value = node_text(node);
        g_print("%-*s  %s\n", width, name, value);
    }
}
