/* Workspace and monitor snapshots: the workspace and monitor list snapshots, the created, renamed,
 * changed, removed and activated workspace events, the monitor lifecycle events, and the signal
 * watches on the workspace and monitor managers. */
#ifndef _GNU_SOURCE
#define _GNU_SOURCE
#endif
#include "config.h"

#include "core/gnoblin-control-internal.h"

#include "meta/util.h"
#include "meta/workspace.h"

static void native_workspace_state_free(gpointer data) {
    NativeWorkspaceState* state = data;
    g_free(state->json);
    g_free(state);
}

static void native_monitor_state_free(gpointer data) {
    NativeMonitorState* state = data;
    g_free(state->json);
    g_free(state);
}

static void cache_lua_workspace_snapshot(GnoblinNativeControl* control, GVariant* native_snapshot,
                                         guint64 revision) {
    g_autoptr(JsonNode) json = gnoblin_control_json_from_variant(native_snapshot);
    if (!JSON_NODE_HOLDS_OBJECT(json)) {
        gnoblin_control_publish_runtime_snapshot(control, "workspaces", NULL, revision);
        return;
    }
    JsonObject* object = json_node_get_object(json);
    JsonNode* native_workspaces = json_object_get_member(object, "workspaces");
    if (!native_workspaces || !JSON_NODE_HOLDS_ARRAY(native_workspaces)) {
        gnoblin_control_publish_runtime_snapshot(control, "workspaces", NULL, revision);
        return;
    }

    JsonArray* workspaces = json_array_new();
    JsonArray* source = json_node_get_array(native_workspaces);
    for (guint i = 0; i < json_array_get_length(source); i++) {
        JsonNode* native_record = json_array_get_element(source, i);
        if (!JSON_NODE_HOLDS_OBJECT(native_record))
            continue;
        JsonObject* source_record = json_node_get_object(native_record);
        JsonObject* record = json_object_new();
        GList* members = json_object_get_members(source_record);
        for (GList* item = members; item; item = item->next) {
            const char* name = item->data;
            const char* target_name = g_str_equal(name, "windows") ? "window_count" : name;
            if (!g_str_equal(name, "revision"))
                json_object_set_member(record, target_name,
                                       json_node_copy(json_object_get_member(source_record, name)));
        }
        g_list_free(members);
        json_object_set_int_member(record, "revision", revision);
        JsonNode* record_node = json_node_new(JSON_NODE_OBJECT);
        json_node_take_object(record_node, record);
        json_array_add_element(workspaces, record_node);
    }
    JsonNode* lua_workspaces = json_node_new(JSON_NODE_ARRAY);
    json_node_take_array(lua_workspaces, workspaces);
    json_object_set_member(object, "workspaces", lua_workspaces);
    json_object_set_int_member(object, "revision", revision);
    g_autoptr(GVariant) snapshot = gnoblin_control_variant_from_json(json);
    if (snapshot)
        gnoblin_control_publish_runtime_snapshot(control, "workspaces", snapshot, revision);
    else
        gnoblin_control_publish_runtime_snapshot(control, "workspaces", NULL, revision);
}

static JsonNode* lua_workspace_record(JsonNode* native_record, guint64 revision) {
    JsonObject* source = json_node_get_object(native_record);
    JsonObject* record = json_object_new();
    GList* members = json_object_get_members(source);
    for (GList* item = members; item; item = item->next) {
        const char* name = item->data;
        const char* target_name = g_str_equal(name, "windows") ? "window_count" : name;
        if (!g_str_equal(name, "revision"))
            json_object_set_member(record, target_name,
                                   json_node_copy(json_object_get_member(source, name)));
    }
    g_list_free(members);
    json_object_set_int_member(record, "revision", revision);
    JsonNode* result = json_node_new(JSON_NODE_OBJECT);
    json_node_take_object(result, record);
    return result;
}

static void dispatch_lua_workspace_event(GnoblinNativeControl* control, guint64 revision,
                                         const char* event, const char* id, JsonNode* workspace,
                                         JsonNode* last, const char* previous_id) {
    g_autoptr(JsonNode) root = json_node_new(JSON_NODE_OBJECT);
    JsonObject* object = json_object_new();
    json_node_take_object(root, object);
    json_object_set_string_member(object, "name", event);
    json_object_set_int_member(object, "revision", revision);
    json_object_set_int_member(object, "sequence", ++control->event_sequence);
    json_object_set_int_member(object, "time", g_get_monotonic_time());
    if (g_str_equal(event, "gnoblin.workspace.removed")) {
        json_object_set_string_member(object, "workspace_id", id);
        json_object_set_member(object, "last", lua_workspace_record(last, revision));
    } else {
        json_object_set_member(object, "workspace", lua_workspace_record(workspace, revision));
        if (g_str_equal(event, "gnoblin.workspace.activated") && previous_id)
            json_object_set_string_member(object, "previous_id", previous_id);
    }
    gnoblin_control_publish_socket_event(control, root);
    g_autoptr(GVariant) payload = gnoblin_control_variant_from_json(root);
    if (payload)
        gnoblin_control_dispatch_event(control, event, payload);
}

static void dispatch_lua_workspace_changed_event(GnoblinNativeControl* control, guint64 revision,
                                                 JsonNode* workspace, const char* const* changed,
                                                 guint changed_count) {
    g_autoptr(JsonNode) root = json_node_new(JSON_NODE_OBJECT);
    JsonObject* object = json_object_new();
    json_node_take_object(root, object);
    json_object_set_string_member(object, "name", "gnoblin.workspace.changed");
    json_object_set_int_member(object, "revision", revision);
    json_object_set_int_member(object, "sequence", ++control->event_sequence);
    json_object_set_int_member(object, "time", g_get_monotonic_time());
    json_object_set_member(object, "workspace", lua_workspace_record(workspace, revision));
    JsonArray* changed_array = json_array_new();
    for (guint i = 0; i < changed_count; i++)
        json_array_add_string_element(changed_array, changed[i]);
    json_object_set_array_member(object, "changed", changed_array);
    gnoblin_control_publish_socket_event(control, root);
    g_autoptr(GVariant) payload = gnoblin_control_variant_from_json(root);
    if (payload)
        gnoblin_control_dispatch_event(control, "gnoblin.workspace.changed", payload);
}

static void dispatch_lua_window_moved_event(GnoblinNativeControl* control, guint64 revision,
                                            const char* id, const char* from_id,
                                            const char* to_id) {
    g_autoptr(JsonNode) root = json_node_new(JSON_NODE_OBJECT);
    JsonObject* object = json_object_new();
    json_node_take_object(root, object);
    json_object_set_string_member(object, "name", "gnoblin.workspace.window-moved");
    json_object_set_int_member(object, "revision", revision);
    json_object_set_int_member(object, "sequence", ++control->event_sequence);
    json_object_set_int_member(object, "time", g_get_monotonic_time());
    json_object_set_string_member(object, "window_id", id);
    json_object_set_string_member(object, "from_id", from_id);
    json_object_set_string_member(object, "to_id", to_id);
    gnoblin_control_publish_socket_event(control, root);
    g_autoptr(GVariant) payload = gnoblin_control_variant_from_json(root);
    if (payload)
        gnoblin_control_dispatch_event(control, "gnoblin.workspace.window-moved", payload);
}

static gboolean workspace_active(JsonNode* node) {
    JsonNode* active = json_object_get_member(json_node_get_object(node), "active");
    return active && JSON_NODE_HOLDS_VALUE(active) && json_node_get_boolean(active);
}

static const char* workspace_id(JsonNode* node) {
    return json_object_get_string_member_with_default(json_node_get_object(node), "id", NULL);
}

static gboolean workspace_int_property_changed(JsonObject* previous, JsonObject* current,
                                               const char* property) {
    JsonNode* before = json_object_get_member(previous, property);
    JsonNode* after = json_object_get_member(current, property);
    if (!before || !after)
        return before != after;
    return json_node_get_int(before) != json_node_get_int(after);
}

static gboolean workspace_boolean_property_changed(JsonObject* previous, JsonObject* current,
                                                   const char* property) {
    JsonNode* before = json_object_get_member(previous, property);
    JsonNode* after = json_object_get_member(current, property);
    if (!before || !after)
        return before != after;
    return json_node_get_boolean(before) != json_node_get_boolean(after);
}

GHashTable* gnoblin_control_workspace_state_from_snapshot(JsonNode* snapshot) {
    GHashTable* state =
        g_hash_table_new_full(g_str_hash, g_str_equal, g_free, native_workspace_state_free);
    if (!JSON_NODE_HOLDS_OBJECT(snapshot))
        return state;
    JsonArray* workspaces =
        json_object_get_array_member(json_node_get_object(snapshot), "workspaces");
    for (guint i = 0; workspaces && i < json_array_get_length(workspaces); i++) {
        JsonNode* record = json_array_get_element(workspaces, i);
        if (!JSON_NODE_HOLDS_OBJECT(record))
            continue;
        const char* id = workspace_id(record);
        if (!id || !*id)
            continue;
        NativeWorkspaceState* value = g_new0(NativeWorkspaceState, 1);
        value->json = json_to_string(record, FALSE);
        g_hash_table_insert(state, g_strdup(id), value);
    }
    return state;
}

void gnoblin_control_publish_workspace_changes(GnoblinNativeControl* control, JsonNode* snapshot,
                                               JsonNode* window_snapshot, guint64 revision) {
    GHashTable* current = gnoblin_control_workspace_state_from_snapshot(snapshot);
    gboolean dispatch_events = control->workspace_state_initialized;
    const char* previous_active_id = NULL;
    g_autofree char* previous_active_storage = NULL;

    if (dispatch_events) {
        GHashTableIter iter;
        gpointer key;
        gpointer value;
        g_hash_table_iter_init(&iter, control->workspace_state);
        while (g_hash_table_iter_next(&iter, &key, &value)) {
            NativeWorkspaceState* previous = value;
            g_autoptr(JsonParser) parser = json_parser_new();
            if (json_parser_load_from_data(parser, previous->json, -1, NULL) &&
                workspace_active(json_parser_get_root(parser))) {
                previous_active_storage = g_strdup(key);
                previous_active_id = previous_active_storage;
                break;
            }
        }

        g_hash_table_iter_init(&iter, current);
        while (g_hash_table_iter_next(&iter, &key, &value)) {
            NativeWorkspaceState* state = value;
            NativeWorkspaceState* previous = g_hash_table_lookup(control->workspace_state, key);
            g_autoptr(JsonParser) parser = json_parser_new();
            if (!json_parser_load_from_data(parser, state->json, -1, NULL))
                continue;
            JsonNode* record = json_parser_get_root(parser);
            if (!previous) {
                dispatch_lua_workspace_event(control, revision, "gnoblin.workspace.created", key,
                                             record, NULL, NULL);
            } else {
                g_autoptr(JsonParser) previous_parser = json_parser_new();
                if (json_parser_load_from_data(previous_parser, previous->json, -1, NULL)) {
                    JsonObject* previous_record =
                        json_node_get_object(json_parser_get_root(previous_parser));
                    JsonObject* current_record = json_node_get_object(record);
                    if (g_strcmp0(
                            json_object_get_string_member_with_default(previous_record, "name", ""),
                            json_object_get_string_member_with_default(current_record, "name",
                                                                       "")) != 0)
                        dispatch_lua_workspace_event(control, revision, "gnoblin.workspace.renamed",
                                                     key, record, NULL, NULL);

                    const char* changed[3];
                    guint changed_count = 0;
                    if (workspace_int_property_changed(previous_record, current_record, "number"))
                        changed[changed_count++] = "number";
                    if (workspace_int_property_changed(previous_record, current_record, "windows"))
                        changed[changed_count++] = "window_count";
                    if (workspace_boolean_property_changed(previous_record, current_record,
                                                           "persistent"))
                        changed[changed_count++] = "persistent";
                    if (changed_count)
                        dispatch_lua_workspace_changed_event(control, revision, record, changed,
                                                             changed_count);
                }
            }
            if (workspace_active(record) &&
                (!previous_active_id || !g_str_equal(previous_active_id, key)))
                dispatch_lua_workspace_event(control, revision, "gnoblin.workspace.activated", key,
                                             record, NULL, previous_active_id);
        }

        g_hash_table_iter_init(&iter, control->workspace_state);
        while (g_hash_table_iter_next(&iter, &key, &value)) {
            if (g_hash_table_contains(current, key))
                continue;
            NativeWorkspaceState* previous = value;
            g_autoptr(JsonParser) parser = json_parser_new();
            if (json_parser_load_from_data(parser, previous->json, -1, NULL))
                dispatch_lua_workspace_event(control, revision, "gnoblin.workspace.removed", key,
                                             NULL, json_parser_get_root(parser), NULL);
        }

        JsonArray* windows =
            JSON_NODE_HOLDS_OBJECT(window_snapshot)
                ? json_object_get_array_member(json_node_get_object(window_snapshot), "windows")
                : NULL;
        for (guint i = 0; windows && i < json_array_get_length(windows); i++) {
            JsonNode* window = json_array_get_element(windows, i);
            if (!JSON_NODE_HOLDS_OBJECT(window))
                continue;
            JsonObject* record = json_node_get_object(window);
            const char* id = json_object_get_string_member_with_default(record, "id", NULL);
            const char* to_id =
                json_object_get_string_member_with_default(record, "workspaceId", NULL);
            NativeWindowState* previous =
                id ? g_hash_table_lookup(control->window_state, id) : NULL;
            if (!previous || !to_id || !*to_id)
                continue;
            g_autoptr(JsonParser) parser = json_parser_new();
            if (!json_parser_load_from_data(parser, previous->json, -1, NULL))
                continue;
            const char* from_id = json_object_get_string_member_with_default(
                json_node_get_object(json_parser_get_root(parser)), "workspaceId", NULL);
            if (from_id && *from_id && !g_str_equal(from_id, to_id))
                dispatch_lua_window_moved_event(control, revision, id, from_id, to_id);
        }
    }

    if (control->workspace_state)
        g_hash_table_unref(control->workspace_state);
    control->workspace_state = current;
    control->workspace_state_initialized = TRUE;
}

JsonNode* gnoblin_control_workspace_snapshot_json(GnoblinNativeControl* control, GError** error) {
    GVariantBuilder empty;
    g_variant_builder_init(&empty, G_VARIANT_TYPE_VARDICT);
    g_autoptr(GVariant) arguments = g_variant_ref_sink(g_variant_builder_end(&empty));
    g_autoptr(GVariant) result =
        meta_gnoblin_dispatch_native_api(control->display, "workspace.list", arguments, error);
    if (!result) {
        gnoblin_control_publish_runtime_snapshot(control, "workspaces", NULL,
                                                 control->state_revision);
        return NULL;
    }
    cache_lua_workspace_snapshot(control, result, control->state_revision);
    g_autoptr(JsonNode) json = gnoblin_control_json_from_variant(result);
    return g_steal_pointer(&json);
}

JsonNode* gnoblin_control_monitor_snapshot_json(GnoblinNativeControl* control,
                                                gboolean update_lua_snapshot, GError** error) {
    GVariantBuilder empty;
    g_variant_builder_init(&empty, G_VARIANT_TYPE_VARDICT);
    g_autoptr(GVariant) arguments = g_variant_ref_sink(g_variant_builder_end(&empty));
    g_autoptr(GVariant) result =
        meta_gnoblin_dispatch_native_api(control->display, "monitor.list", arguments, error);
    if (!result) {
        if (update_lua_snapshot)
            gnoblin_control_publish_runtime_snapshot(control, "monitors", NULL,
                                                     control->state_revision);
        return NULL;
    }

    g_autoptr(JsonNode) json = gnoblin_control_json_from_variant(result);
    if (!JSON_NODE_HOLDS_OBJECT(json)) {
        if (update_lua_snapshot)
            gnoblin_control_publish_runtime_snapshot(control, "monitors", NULL,
                                                     control->state_revision);
        return NULL;
    }
    JsonArray* monitors = json_object_get_array_member(json_node_get_object(json), "monitors");
    if (!monitors) {
        if (update_lua_snapshot)
            gnoblin_control_publish_runtime_snapshot(control, "monitors", NULL,
                                                     control->state_revision);
        return NULL;
    }
    MetaWorkspaceManager* workspace_manager = meta_display_get_workspace_manager(control->display);
    MetaWorkspace* active_workspace =
        workspace_manager ? meta_workspace_manager_get_active_workspace(workspace_manager) : NULL;
    for (guint i = 0; i < json_array_get_length(monitors); i++) {
        JsonNode* record = json_array_get_element(monitors, i);
        if (!JSON_NODE_HOLDS_OBJECT(record))
            continue;
        JsonObject* monitor = json_node_get_object(record);
        json_object_set_int_member(monitor, "revision", control->state_revision);
        /* The work area is the monitor minus the space that panels reserve with exclusive zones.
         * It is what a window can use, so a shell or a person can see why a window stops where it does. */
        int monitor_index = (int)json_object_get_int_member_with_default(monitor, "index", -1);
        MtkRectangle work_area;
        if (active_workspace && monitor_index >= 0) {
            meta_workspace_get_work_area_for_monitor(active_workspace, monitor_index, &work_area);
            JsonObject* area = json_object_new();
            json_object_set_int_member(area, "x", work_area.x);
            json_object_set_int_member(area, "y", work_area.y);
            json_object_set_int_member(area, "width", work_area.width);
            json_object_set_int_member(area, "height", work_area.height);
            json_object_set_object_member(monitor, "work_area", area);
        }
    }
    json_object_set_int_member(json_node_get_object(json), "revision", control->state_revision);
    if (update_lua_snapshot) {
        g_autoptr(GVariant) snapshot = gnoblin_control_variant_from_json(json);
        gnoblin_control_publish_runtime_snapshot(control, "monitors", snapshot,
                                                 control->state_revision);
    }
    return g_steal_pointer(&json);
}

GHashTable* gnoblin_control_monitor_state_from_snapshot(JsonNode* snapshot) {
    GHashTable* state =
        g_hash_table_new_full(g_str_hash, g_str_equal, g_free, native_monitor_state_free);
    if (!JSON_NODE_HOLDS_OBJECT(snapshot))
        return state;
    JsonArray* monitors = json_object_get_array_member(json_node_get_object(snapshot), "monitors");
    for (guint i = 0; monitors && i < json_array_get_length(monitors); i++) {
        JsonNode* record = json_array_get_element(monitors, i);
        if (!JSON_NODE_HOLDS_OBJECT(record))
            continue;
        const char* id =
            json_object_get_string_member_with_default(json_node_get_object(record), "id", NULL);
        if (!id || !*id)
            continue;
        NativeMonitorState* value = g_new0(NativeMonitorState, 1);
        value->json = json_to_string(record, FALSE);
        g_hash_table_insert(state, g_strdup(id), value);
    }
    return state;
}

char* gnoblin_control_monitor_id_for_index(JsonNode* snapshot, int monitor_index) {
    if (monitor_index < 0 || !JSON_NODE_HOLDS_OBJECT(snapshot))
        return NULL;
    JsonArray* monitors = json_object_get_array_member(json_node_get_object(snapshot), "monitors");
    const char* matched_id = NULL;
    for (guint i = 0; monitors && i < json_array_get_length(monitors); i++) {
        JsonNode* record = json_array_get_element(monitors, i);
        if (!JSON_NODE_HOLDS_OBJECT(record))
            continue;
        JsonObject* monitor = json_node_get_object(record);
        if (json_object_get_int_member_with_default(monitor, "index", -1) != monitor_index)
            continue;
        const char* id = json_object_get_string_member_with_default(monitor, "id", NULL);
        if (!id || !*id || matched_id)
            return NULL;
        matched_id = id;
    }
    return g_strdup(matched_id);
}

static void dispatch_lua_monitor_event(GnoblinNativeControl* control, guint64 revision,
                                       const char* event, const char* id, JsonNode* monitor,
                                       JsonNode* last, JsonArray* changed) {
    g_autoptr(JsonNode) root = json_node_new(JSON_NODE_OBJECT);
    JsonObject* object = json_object_new();
    json_node_take_object(root, object);
    json_object_set_string_member(object, "name", event);
    json_object_set_int_member(object, "revision", revision);
    json_object_set_int_member(object, "sequence", ++control->event_sequence);
    json_object_set_int_member(object, "time", g_get_monotonic_time());
    if (g_str_equal(event, "gnoblin.monitor.removed")) {
        json_object_set_string_member(object, "monitor_id", id);
        json_object_set_member(object, "last", json_node_copy(last));
    } else {
        json_object_set_member(object, "monitor", json_node_copy(monitor));
        if (changed)
            json_object_set_array_member(object, "changed", json_array_ref(changed));
    }
    gnoblin_control_publish_socket_event(control, root);
    g_autoptr(GVariant) payload = gnoblin_control_variant_from_json(root);
    if (payload)
        gnoblin_control_dispatch_event(control, event, payload);
}

gboolean gnoblin_control_publish_monitor_changes(GnoblinNativeControl* control, JsonNode* snapshot,
                                                 guint64 revision) {
    GHashTable* current = gnoblin_control_monitor_state_from_snapshot(snapshot);
    gboolean changed_state = !control->monitor_state_initialized;
    if (control->monitor_state_initialized) {
        static const char* properties[] = {
            "id",      "index", "x",    "y",     "width",  "height",       "primary",   "scale",
            "enabled", "name",  "make", "model", "serial", "refresh_rate", "transform",
            "work_area",
        };
        GHashTableIter iter;
        gpointer key;
        gpointer value;
        g_hash_table_iter_init(&iter, current);
        while (g_hash_table_iter_next(&iter, &key, &value)) {
            NativeMonitorState* state = value;
            NativeMonitorState* previous = g_hash_table_lookup(control->monitor_state, key);
            g_autoptr(JsonParser) parser = json_parser_new();
            if (!json_parser_load_from_data(parser, state->json, -1, NULL))
                continue;
            JsonNode* record = json_parser_get_root(parser);
            if (!previous) {
                changed_state = TRUE;
                dispatch_lua_monitor_event(control, revision, "gnoblin.monitor.added", key, record,
                                           NULL, NULL);
                continue;
            }
            g_autoptr(JsonParser) previous_parser = json_parser_new();
            if (!json_parser_load_from_data(previous_parser, previous->json, -1, NULL))
                continue;
            JsonObject* old_record = json_node_get_object(json_parser_get_root(previous_parser));
            JsonObject* new_record = json_node_get_object(record);
            JsonArray* changed = json_array_new();
            for (guint i = 0; i < G_N_ELEMENTS(properties); i++) {
                if (gnoblin_control_monitor_property_changed(old_record, new_record, properties[i]))
                    json_array_add_string_element(changed, properties[i]);
            }
            if (json_array_get_length(changed)) {
                changed_state = TRUE;
                dispatch_lua_monitor_event(control, revision, "gnoblin.monitor.changed", key,
                                           record, NULL, changed);
            }
            json_array_unref(changed);
        }

        g_hash_table_iter_init(&iter, control->monitor_state);
        while (g_hash_table_iter_next(&iter, &key, &value)) {
            if (g_hash_table_contains(current, key))
                continue;
            changed_state = TRUE;
            NativeMonitorState* previous = value;
            g_autoptr(JsonParser) parser = json_parser_new();
            if (json_parser_load_from_data(parser, previous->json, -1, NULL))
                dispatch_lua_monitor_event(control, revision, "gnoblin.monitor.removed", key, NULL,
                                           json_parser_get_root(parser), NULL);
        }
    }
    if (control->monitor_state)
        g_hash_table_unref(control->monitor_state);
    control->monitor_state = current;
    control->monitor_state_initialized = TRUE;
    return changed_state;
}

void gnoblin_control_clear_workspace_manager_signal_watches(GnoblinNativeControl* control) {
    if (control && control->workspace_manager)
        gnoblin_control_clear_object_signal_watches(G_OBJECT(control->workspace_manager),
                                                    control->workspace_manager_signal_handler_ids);
}

void gnoblin_control_refresh_workspace_manager_signal_watches(GnoblinNativeControl* control) {
    if (control && control->workspace_manager)
        gnoblin_control_refresh_object_signal_watches(
            control, G_OBJECT(control->workspace_manager), "workspace-manager",
            control->workspace_manager_signal_handler_ids);
}

void gnoblin_control_clear_all_workspace_signal_watches(GnoblinNativeControl* control) {
    if (!control || !control->workspace_signal_handler_ids)
        return;
    GHashTableIter iter;
    gpointer workspace;
    gpointer handler_ids;
    g_hash_table_iter_init(&iter, control->workspace_signal_handler_ids);
    while (g_hash_table_iter_next(&iter, &workspace, &handler_ids))
        gnoblin_control_clear_object_signal_watches(G_OBJECT(workspace), handler_ids);
    g_hash_table_remove_all(control->workspace_signal_handler_ids);
}

void gnoblin_control_refresh_all_workspace_signal_watches(GnoblinNativeControl* control) {
    if (!control || !control->workspace_manager || !control->workspace_signal_handler_ids)
        return;
    gnoblin_control_clear_all_workspace_signal_watches(control);
    if (!gnoblin_control_workspace_signal_subscribed(control))
        return;
    int count = meta_workspace_manager_get_n_workspaces(control->workspace_manager);
    for (int index = 0; index < count; index++) {
        MetaWorkspace* workspace =
            meta_workspace_manager_get_workspace_by_index(control->workspace_manager, index);
        if (!workspace)
            continue;
        GArray* handler_ids = g_array_new(FALSE, FALSE, sizeof(gulong));
        g_hash_table_insert(control->workspace_signal_handler_ids, g_object_ref(workspace),
                            handler_ids);
        gnoblin_control_refresh_object_signal_watches(control, G_OBJECT(workspace), "workspace",
                                                      handler_ids);
    }
}

void gnoblin_control_clear_monitor_manager_signal_watches(GnoblinNativeControl* control) {
    if (control && control->monitor_manager)
        gnoblin_control_clear_object_signal_watches(G_OBJECT(control->monitor_manager),
                                                    control->monitor_manager_signal_handler_ids);
}

void gnoblin_control_refresh_monitor_manager_signal_watches(GnoblinNativeControl* control) {
    if (control && control->monitor_manager)
        gnoblin_control_refresh_object_signal_watches(control, G_OBJECT(control->monitor_manager),
                                                      "monitor-manager",
                                                      control->monitor_manager_signal_handler_ids);
}
