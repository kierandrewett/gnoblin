/* Dynamic shortcut bindings: the shortcut.bind, shortcut.unbind and shortcut.session.end socket
 * methods, the bare-Super arming, the lookup and cleanup of bound shortcuts per client and per
 * Lua owner, and the binding validators. Session start and end, key capture, activation and the
 * event routing stay in gnoblin-native-control.c because tests read their text there. */
#ifndef _GNU_SOURCE
#define _GNU_SOURCE
#endif
#include "config.h"

#include "core/gnoblin-control-internal.h"

#include "meta/util.h"

#define MAX_DYNAMIC_SHORTCUTS_PER_CLIENT 32
#define MAX_DYNAMIC_SHORTCUTS 128

void gnoblin_control_dynamic_shortcut_free(gpointer data) {
    NativeDynamicShortcut* shortcut = data;
    if (!shortcut)
        return;
    if (shortcut->timeout_id)
        g_source_remove(shortcut->timeout_id);
    if (shortcut->trigger_event)
        clutter_event_free(shortcut->trigger_event);
    g_free(shortcut->id);
    g_free(shortcut->accelerator);
    g_free(shortcut->owner_id);
    g_free(shortcut);
}

guint gnoblin_control_dynamic_shortcut_count(GnoblinNativeControl* control, Client* owner) {
    guint count = 0;
    if (!control || !control->dynamic_shortcuts)
        return 0;
    GHashTableIter iter;
    gpointer value;
    g_hash_table_iter_init(&iter, control->dynamic_shortcuts);
    while (g_hash_table_iter_next(&iter, NULL, &value)) {
        NativeDynamicShortcut* shortcut = value;
        if (!owner || shortcut->client == owner)
            count++;
    }
    if (control->bare_super_shortcut && (!owner || control->bare_super_shortcut->client == owner))
        count++;
    return count;
}

NativeDynamicShortcut* gnoblin_control_find_dynamic_shortcut(Client* owner, const char* id) {
    if (!owner || !owner->control || !owner->control->dynamic_shortcuts || !id)
        return NULL;
    GHashTableIter iter;
    gpointer value;
    g_hash_table_iter_init(&iter, owner->control->dynamic_shortcuts);
    while (g_hash_table_iter_next(&iter, NULL, &value)) {
        NativeDynamicShortcut* shortcut = value;
        if (shortcut->client == owner && g_str_equal(shortcut->id, id))
            return shortcut;
    }
    if (owner->control->bare_super_shortcut &&
        owner->control->bare_super_shortcut->client == owner &&
        g_str_equal(owner->control->bare_super_shortcut->id, id))
        return owner->control->bare_super_shortcut;
    return NULL;
}

NativeDynamicShortcut* gnoblin_control_find_runtime_dynamic_shortcut(GnoblinNativeControl* control,
                                                                     const char* owner_id,
                                                                     const char* id) {
    if (!control || !owner_id || !id)
        return NULL;
    if (control->bare_super_shortcut &&
        g_str_equal(control->bare_super_shortcut->owner_id, owner_id) &&
        g_str_equal(control->bare_super_shortcut->id, id))
        return control->bare_super_shortcut;
    GHashTableIter iter;
    gpointer value;
    g_hash_table_iter_init(&iter, control->dynamic_shortcuts);
    while (g_hash_table_iter_next(&iter, NULL, &value)) {
        NativeDynamicShortcut* shortcut = value;
        if (!shortcut->client && g_str_equal(shortcut->owner_id, owner_id) &&
            g_str_equal(shortcut->id, id))
            return shortcut;
    }
    return NULL;
}

void gnoblin_control_remove_dynamic_shortcut(GnoblinNativeControl* control,
                                             NativeDynamicShortcut* shortcut) {
    if (!control || !shortcut)
        return;
    gnoblin_control_dynamic_shortcut_end_session(control, shortcut, "unbound");
    if (control->bare_super_shortcut == shortcut) {
        control->bare_super_shortcut = NULL;
        gnoblin_control_dynamic_shortcut_free(shortcut);
        return;
    }
    if (!control->dynamic_shortcuts)
        return;
    guint action = shortcut->action;
    meta_display_ungrab_accelerator(control->display, action);
    g_hash_table_remove(control->dynamic_shortcuts, GUINT_TO_POINTER(action));
}

void gnoblin_control_clear_client_dynamic_shortcuts(Client* client) {
    if (!client || !client->control || !client->control->dynamic_shortcuts)
        return;
    GnoblinNativeControl* control = client->control;
    GHashTableIter iter;
    gpointer value;
    g_hash_table_iter_init(&iter, control->dynamic_shortcuts);
    while (g_hash_table_iter_next(&iter, NULL, &value)) {
        NativeDynamicShortcut* shortcut = value;
        if (shortcut->client == client) {
            gnoblin_control_dynamic_shortcut_end_session(control, shortcut, "owner_disconnected");
            meta_display_ungrab_accelerator(control->display, shortcut->action);
            g_hash_table_iter_remove(&iter);
        }
    }
    if (control->bare_super_shortcut && control->bare_super_shortcut->client == client) {
        NativeDynamicShortcut* shortcut = control->bare_super_shortcut;
        gnoblin_control_dynamic_shortcut_end_session(control, shortcut, "owner_disconnected");
        control->bare_super_shortcut = NULL;
        gnoblin_control_dynamic_shortcut_free(shortcut);
    }
}

void gnoblin_control_clear_runtime_dynamic_shortcuts(GnoblinNativeControl* control,
                                                     const char* reason) {
    if (!control || !control->dynamic_shortcuts)
        return;
    GHashTableIter iter;
    gpointer value;
    g_hash_table_iter_init(&iter, control->dynamic_shortcuts);
    while (g_hash_table_iter_next(&iter, NULL, &value)) {
        NativeDynamicShortcut* shortcut = value;
        if (shortcut->owner_id && g_str_has_prefix(shortcut->owner_id, "lua:")) {
            gnoblin_control_dynamic_shortcut_end_session(control, shortcut, reason);
            meta_display_ungrab_accelerator(control->display, shortcut->action);
            g_hash_table_iter_remove(&iter);
        }
    }
    if (control->bare_super_shortcut && control->bare_super_shortcut->owner_id &&
        g_str_has_prefix(control->bare_super_shortcut->owner_id, "lua:")) {
        NativeDynamicShortcut* shortcut = control->bare_super_shortcut;
        gnoblin_control_dynamic_shortcut_end_session(control, shortcut, reason);
        control->bare_super_shortcut = NULL;
        gnoblin_control_dynamic_shortcut_free(shortcut);
    }
}

void gnoblin_control_clear_configured_capture_shortcut(GnoblinNativeControl* control,
                                                       const char* reason) {
    NativeDynamicShortcut* shortcut = control ? control->bare_super_shortcut : NULL;
    if (!shortcut || !shortcut->owner_id || !g_str_has_prefix(shortcut->owner_id, "config:"))
        return;
    gnoblin_control_dynamic_shortcut_end_session(control, shortcut, reason);
    control->bare_super_shortcut = NULL;
    gnoblin_control_dynamic_shortcut_free(shortcut);
}

gboolean gnoblin_control_dynamic_shortcut_id_valid(const char* id) {
    if (!id || !*id || strlen(id) > 64)
        return FALSE;
    for (const char* cursor = id; *cursor; cursor++)
        if (!g_ascii_isalnum(*cursor) && *cursor != '_' && *cursor != '-')
            return FALSE;
    return TRUE;
}

NativeDynamicShortcut* gnoblin_control_arm_bare_super_shortcut(GnoblinNativeControl* control,
                                                               const char* id, const char* owner_id,
                                                               Client* client, guint64 session_id,
                                                               GError** error) {
    if (!control || control->stopping || !control->overlay_modifier_hook_available) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_SUPPORTED,
                            "bare Super capture is unavailable in this compositor build");
        return NULL;
    }
    if (!gnoblin_control_dynamic_shortcut_id_valid(id) || !owner_id || !*owner_id ||
        strlen(owner_id) > 128 || !session_id || control->bare_super_shortcut ||
        gnoblin_control_dynamic_shortcut_count(control, NULL) >= MAX_DYNAMIC_SHORTCUTS) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_BUSY,
                            "bare Super capture cannot be armed for this owner");
        return NULL;
    }
    for (const char* cursor = owner_id; *cursor; cursor++)
        if (g_ascii_iscntrl(*cursor)) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                                "shortcut owner ID contains a control character");
            return NULL;
        }
    if (client && gnoblin_control_find_dynamic_shortcut(client, id)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_EXISTS,
                            "shortcut ID is already registered by this connection");
        return NULL;
    }
    NativeDynamicShortcut* shortcut = g_new0(NativeDynamicShortcut, 1);
    shortcut->control = control;
    shortcut->client = client;
    shortcut->client_id = client ? client->client_id : 0;
    shortcut->id = g_strdup(id);
    shortcut->accelerator = g_strdup("Super");
    shortcut->owner_id = g_strdup(owner_id);
    shortcut->session_id = session_id;
    shortcut->trigger_release = TRUE;
    shortcut->capture_input = TRUE;
    shortcut->armed = TRUE;
    control->bare_super_shortcut = shortcut;
    return shortcut;
}

gboolean gnoblin_control_dynamic_shortcut_accelerator_valid(const char* accelerator) {
    if (!accelerator || !*accelerator || strlen(accelerator) > 128 ||
        !g_utf8_validate(accelerator, -1, NULL))
        return FALSE;
    for (const char* cursor = accelerator; *cursor; cursor++)
        if (g_ascii_iscntrl(*cursor))
            return FALSE;
    return TRUE;
}

static gboolean dynamic_shortcut_arguments_have_only(JsonObject* arguments, const char* first,
                                                     const char* second) {
    if (!arguments)
        return FALSE;
    GList* members = json_object_get_members(arguments);
    gboolean valid = TRUE;
    for (GList* item = members; item; item = item->next) {
        const char* name = item->data;
        gboolean supported = g_str_equal(name, "hold") || g_str_equal(name, "trigger") ||
                             g_str_equal(name, "mode") || g_str_equal(name, "capture_input");
        if (!g_str_equal(name, first) && (!second || !g_str_equal(name, second)) && !supported) {
            valid = FALSE;
            break;
        }
    }
    g_list_free(members);
    return valid;
}

char* gnoblin_control_dynamic_shortcut_bind(Client* client, const char* request_id,
                                            JsonObject* arguments) {
    if (!client || client->closing || !client->control || !arguments)
        return gnoblin_control_encode_response(request_id, NULL,
                                               "shortcut.bind requires a live connection");
    if (!dynamic_shortcut_arguments_have_only(arguments, "id", "accelerator"))
        return gnoblin_control_encode_response(
            request_id, NULL,
            "shortcut.bind accepts id, accelerator, hold, trigger, mode, and "
            "capture_input");
    JsonNode* id_node = json_object_get_member(arguments, "id");
    JsonNode* accelerator_node = json_object_get_member(arguments, "accelerator");
    if (!id_node || !JSON_NODE_HOLDS_VALUE(id_node) ||
        json_node_get_value_type(id_node) != G_TYPE_STRING ||
        !gnoblin_control_dynamic_shortcut_id_valid(json_node_get_string(id_node)))
        return gnoblin_control_encode_response(
            request_id, NULL,
            "shortcut.bind id must contain 1 to 64 letters, digits, underscores, or hyphens");
    if (!accelerator_node || !JSON_NODE_HOLDS_VALUE(accelerator_node) ||
        json_node_get_value_type(accelerator_node) != G_TYPE_STRING ||
        !gnoblin_control_dynamic_shortcut_accelerator_valid(json_node_get_string(accelerator_node)))
        return gnoblin_control_encode_response(
            request_id, NULL,
            "shortcut.bind accelerator must be a non-empty GTK accelerator "
            "string of at most 128 bytes");
    const char* id = json_node_get_string(id_node);
    const char* accelerator = json_node_get_string(accelerator_node);
    const char* trigger = "press";
    const char* hold = "none";
    const char* mode = "passive";
    gboolean capture_input = FALSE;
    JsonNode* member = json_object_get_member(arguments, "trigger");
    if (member) {
        if (!JSON_NODE_HOLDS_VALUE(member) || json_node_get_value_type(member) != G_TYPE_STRING ||
            (g_strcmp0(json_node_get_string(member), "press") != 0 &&
             g_strcmp0(json_node_get_string(member), "release") != 0))
            return gnoblin_control_encode_response(request_id, NULL,
                                                   "trigger must be 'press' or 'release'");
        trigger = json_node_get_string(member);
    }
    member = json_object_get_member(arguments, "hold");
    if (member) {
        if (!JSON_NODE_HOLDS_VALUE(member) || json_node_get_value_type(member) != G_TYPE_STRING ||
            (g_strcmp0(json_node_get_string(member), "none") != 0 &&
             g_strcmp0(json_node_get_string(member), "super") != 0 &&
             g_strcmp0(json_node_get_string(member), "control") != 0 &&
             g_strcmp0(json_node_get_string(member), "alt") != 0))
            return gnoblin_control_encode_response(
                request_id, NULL, "hold must be 'none', 'super', 'control', or 'alt'");
        hold = json_node_get_string(member);
    }
    member = json_object_get_member(arguments, "mode");
    if (member) {
        if (!JSON_NODE_HOLDS_VALUE(member) || json_node_get_value_type(member) != G_TYPE_STRING ||
            (g_strcmp0(json_node_get_string(member), "passive") != 0 &&
             g_strcmp0(json_node_get_string(member), "modal") != 0))
            return gnoblin_control_encode_response(request_id, NULL,
                                                   "mode must be 'passive' or 'modal'");
        mode = json_node_get_string(member);
    }
    member = json_object_get_member(arguments, "capture_input");
    if (member) {
        if (!JSON_NODE_HOLDS_VALUE(member) || json_node_get_value_type(member) != G_TYPE_BOOLEAN)
            return gnoblin_control_encode_response(request_id, NULL,
                                                   "capture_input must be a boolean");
        capture_input = json_node_get_boolean(member);
    }
    if (client->api_minor < 22 && json_object_get_size(arguments) > 2)
        return gnoblin_control_encode_response(
            request_id, NULL, "held and modal shortcut bindings require API version 1.22");
    if (g_str_equal(mode, "modal") && g_str_equal(hold, "none"))
        return gnoblin_control_encode_response(request_id, NULL,
                                               "modal mode requires a held modifier");
    if (capture_input && !g_str_equal(accelerator, "Super"))
        return gnoblin_control_encode_response(
            request_id, NULL,
            "capture_input is only supported for an explicit bare Super "
            "binding");
    if (g_str_equal(accelerator, "Super") &&
        (!capture_input || !g_str_equal(trigger, "release") || !g_str_equal(hold, "none")))
        return gnoblin_control_encode_response(
            request_id, NULL,
            "bare Super requires capture_input=true, trigger='release', and "
            "hold='none'");
    GnoblinNativeControl* control = client->control;
    if (gnoblin_control_find_dynamic_shortcut(client, id))
        return gnoblin_control_encode_response(request_id, NULL,
                                               "shortcut.bind id is already registered");
    if (gnoblin_control_dynamic_shortcut_count(control, client) >=
            MAX_DYNAMIC_SHORTCUTS_PER_CLIENT ||
        gnoblin_control_dynamic_shortcut_count(control, NULL) >= MAX_DYNAMIC_SHORTCUTS)
        return gnoblin_control_encode_response(request_id, NULL,
                                               "dynamic shortcut registration limit reached");

    NativeDynamicShortcut* shortcut = g_new0(NativeDynamicShortcut, 1);
    shortcut->control = control;
    shortcut->client = client;
    shortcut->client_id = client->client_id;
    shortcut->id = g_strdup(id);
    shortcut->accelerator = g_strdup(accelerator);
    shortcut->owner_id = g_strdup_printf("socket:%" G_GUINT64_FORMAT, client->client_id);
    shortcut->trigger_release = g_str_equal(trigger, "release");
    shortcut->modal = g_str_equal(mode, "modal");
    shortcut->capture_input = capture_input;
    shortcut->hold_mask = g_str_equal(hold, "super")     ? CLUTTER_SUPER_MASK
                          : g_str_equal(hold, "control") ? CLUTTER_CONTROL_MASK
                          : g_str_equal(hold, "alt")     ? CLUTTER_MOD1_MASK
                                                         : 0;
    shortcut->armed = TRUE;
    if (g_str_equal(accelerator, "Super")) {
        g_autoptr(GError) arm_error = NULL;
        guint64 session_id = ++control->next_shortcut_session_id;
        if (!session_id)
            session_id = ++control->next_shortcut_session_id;
        gnoblin_control_dynamic_shortcut_free(shortcut);
        g_autofree char* owner_id = g_strdup_printf("socket:%" G_GUINT64_FORMAT, client->client_id);
        shortcut = gnoblin_control_arm_bare_super_shortcut(control, id, owner_id, client,
                                                           session_id, &arm_error);
        if (!shortcut)
            return gnoblin_control_encode_response(
                request_id, NULL,
                arm_error ? arm_error->message : "bare Super capture could not be armed");
    } else {
        guint action = meta_display_grab_accelerator(control->display, accelerator, 0);
        if (action == META_KEYBINDING_ACTION_NONE) {
            gnoblin_control_dynamic_shortcut_free(shortcut);
            return gnoblin_control_encode_response(
                request_id, NULL, "shortcut.bind accelerator is invalid or already claimed");
        }
        shortcut->action = action;
        g_hash_table_insert(control->dynamic_shortcuts, GUINT_TO_POINTER(action), shortcut);
    }

    JsonObject* result_object = json_object_new();
    json_object_set_string_member(result_object, "id", id);
    json_object_set_string_member(result_object, "accelerator", accelerator);
    json_object_set_string_member(result_object, "trigger", trigger);
    json_object_set_string_member(result_object, "hold", hold);
    json_object_set_string_member(result_object, "mode", mode);
    json_object_set_boolean_member(result_object, "capture_input", capture_input);
    g_autoptr(JsonNode) result = json_node_new(JSON_NODE_OBJECT);
    json_node_take_object(result, result_object);
    return gnoblin_control_encode_response(request_id, result, NULL);
}

char* gnoblin_control_dynamic_shortcut_unbind(Client* client, const char* request_id,
                                              JsonObject* arguments) {
    if (!client || client->closing || !client->control || !arguments)
        return gnoblin_control_encode_response(request_id, NULL,
                                               "shortcut.unbind requires a live connection");
    if (!arguments || json_object_get_size(arguments) != 1 ||
        !json_object_has_member(arguments, "id"))
        return gnoblin_control_encode_response(request_id, NULL, "shortcut.unbind accepts only id");
    JsonNode* id_node = json_object_get_member(arguments, "id");
    if (!id_node || !JSON_NODE_HOLDS_VALUE(id_node) ||
        json_node_get_value_type(id_node) != G_TYPE_STRING ||
        !gnoblin_control_dynamic_shortcut_id_valid(json_node_get_string(id_node)))
        return gnoblin_control_encode_response(request_id, NULL,
                                               "shortcut.unbind requires a valid id");
    const char* id = json_node_get_string(id_node);
    NativeDynamicShortcut* shortcut = gnoblin_control_find_dynamic_shortcut(client, id);
    if (!shortcut)
        return gnoblin_control_encode_response(
            request_id, NULL, "shortcut.unbind id is not registered by this connection");
    gnoblin_control_remove_dynamic_shortcut(client->control, shortcut);
    JsonObject* result_object = json_object_new();
    json_object_set_string_member(result_object, "id", id);
    json_object_set_boolean_member(result_object, "unbound", TRUE);
    g_autoptr(JsonNode) result = json_node_new(JSON_NODE_OBJECT);
    json_node_take_object(result, result_object);
    return gnoblin_control_encode_response(request_id, result, NULL);
}

char* gnoblin_control_dynamic_shortcut_request_end_session(Client* client, const char* request_id,
                                                           JsonObject* arguments) {
    if (!client || client->closing || !client->control || !arguments)
        return gnoblin_control_encode_response(request_id, NULL,
                                               "shortcut.session.end requires a live connection");
    if (json_object_get_size(arguments) != 2 || !json_object_has_member(arguments, "id") ||
        !json_object_has_member(arguments, "session_id"))
        return gnoblin_control_encode_response(
            request_id, NULL, "shortcut.session.end accepts only id and session_id");
    JsonNode* id_node = json_object_get_member(arguments, "id");
    JsonNode* session_node = json_object_get_member(arguments, "session_id");
    if (!id_node || !JSON_NODE_HOLDS_VALUE(id_node) ||
        json_node_get_value_type(id_node) != G_TYPE_STRING ||
        !gnoblin_control_dynamic_shortcut_id_valid(json_node_get_string(id_node)) ||
        !session_node || !JSON_NODE_HOLDS_VALUE(session_node) ||
        json_node_get_value_type(session_node) != G_TYPE_INT64 ||
        json_node_get_int(session_node) <= 0)
        return gnoblin_control_encode_response(
            request_id, NULL, "shortcut.session.end requires a valid id and positive session_id");

    const char* binding_id = json_node_get_string(id_node);
    guint64 session_id = (guint64)json_node_get_int(session_node);
    NativeDynamicShortcut* shortcut = gnoblin_control_find_dynamic_shortcut(client, binding_id);
    if (!shortcut || !shortcut->active || shortcut->session_id != session_id)
        return gnoblin_control_encode_response(
            request_id, NULL, "shortcut.session.end session is no longer active for this binding");

    gnoblin_control_dynamic_shortcut_end_session(client->control, shortcut, "cancelled");
    JsonObject* result_object = json_object_new();
    json_object_set_string_member(result_object, "id", binding_id);
    json_object_set_int_member(result_object, "session_id", (gint64)session_id);
    json_object_set_boolean_member(result_object, "ended", TRUE);
    g_autoptr(JsonNode) result = json_node_new(JSON_NODE_OBJECT);
    json_node_take_object(result, result_object);
    return gnoblin_control_encode_response(request_id, result, NULL);
}
