#include "gnoblin-native-control-private.h"
#include "core/stack.h"

static void add_window_signal_identity(GVariantBuilder* payload, MetaWindow* window);

static void write_next(Client* client);

static void write_done(GObject* source, GAsyncResult* result, gpointer user_data) {
    PendingWrite* pending = user_data;
    g_autoptr(GError) error = NULL;
    gboolean written =
        g_output_stream_write_all_finish(G_OUTPUT_STREAM(source), result, NULL, &error);
    Client* client = pending->client;
    client->writing = FALSE;
    client->pending_bytes -= strlen(pending->response);
    g_free(pending->response);
    g_free(pending);
    if (!written || client->closing) {
        client_close(client);
        return;
    }
    if (client->close_after_response && g_queue_is_empty(client->outgoing)) {
        client_close(client);
        return;
    }
    write_next(client);
    if (!client->writing)
        process_buffer(client);
}

static void write_next(Client* client) {
    if (client->writing || client->closing || g_queue_is_empty(client->outgoing))
        return;
    PendingWrite* pending = g_new0(PendingWrite, 1);
    pending->client = client;
    pending->response = g_queue_pop_head(client->outgoing);
    client->writing = TRUE;
    g_output_stream_write_all_async(g_io_stream_get_output_stream(G_IO_STREAM(client->connection)),
                                    pending->response, strlen(pending->response),
                                    G_PRIORITY_DEFAULT, NULL, write_done, pending);
}

void send_response(Client* client, char* response) {
    if (client->closing) {
        g_free(response);
        return;
    }
    gsize length = strlen(response);
    if (client->pending_bytes + length > MAX_PENDING_BYTES) {
        g_free(response);
        client_close(client);
        return;
    }
    client->pending_bytes += length;
    g_queue_push_tail(client->outgoing, response);
    write_next(client);
}

static gboolean publish_windows(gpointer user_data) {
    GnoblinNativeControl* control = user_data;
    guint64 revision = control->state_revision;
    control->publish_id = 0;
    g_autoptr(GError) workspace_error = NULL;
    g_autoptr(JsonNode) workspace_json =
        gnoblin_control_workspace_snapshot_json(control, &workspace_error);
    if (!workspace_json)
        g_warning("gnoblin-native-control: cannot publish workspaces: %s",
                  workspace_error ? workspace_error->message : "workspace listing unavailable");
    g_autoptr(GError) error = NULL;
    g_autoptr(JsonNode) snapshot_json = window_snapshot_json(control, TRUE, &error);
    g_autoptr(GError) monitor_error = NULL;
    g_autoptr(JsonNode) monitor_json =
        gnoblin_control_monitor_snapshot_json(control, TRUE, &monitor_error);
    g_autoptr(GError) output_error = NULL;
    g_autoptr(JsonNode) output_json =
        gnoblin_control_output_snapshot_json(control, TRUE, &output_error);
    g_autoptr(GError) layer_error = NULL;
    g_autoptr(JsonNode) layer_json =
        gnoblin_control_layer_snapshot_json(control, TRUE, &layer_error);
    g_autoptr(GVariant) capabilities = capability_snapshot(control);
    native_publish_runtime_snapshot(control, "capabilities", capabilities, revision);
    g_autoptr(GVariant) input_devices = input_device_snapshot(control);
    native_publish_runtime_snapshot(control, "input-devices", input_devices, revision);
    gnoblin_control_publish_input_source_changes(control, revision);
    g_autoptr(JsonNode) input_devices_json = json_from_variant(input_devices);
    gboolean monitor_changed = FALSE;
    gnoblin_control_publish_input_device_changes(control, input_devices_json, revision);
    if (workspace_json)
        gnoblin_control_publish_workspace_changes(control, workspace_json, snapshot_json, revision);
    if (snapshot_json)
        publish_window_changes(control, snapshot_json, revision);
    else
        g_warning("gnoblin-native-control: cannot publish windows: %s",
                  error ? error->message : "window listing unavailable");
    if (monitor_json)
        monitor_changed = gnoblin_control_publish_monitor_changes(control, monitor_json, revision);
    else
        g_warning("gnoblin-native-control: cannot publish monitors: %s",
                  monitor_error ? monitor_error->message : "monitor listing unavailable");
    if (layer_json)
        publish_layer_changes(control, layer_json, revision);
    else
        g_warning("gnoblin-native-control: cannot publish layers: %s",
                  layer_error ? layer_error->message : "layer listing unavailable");
    g_autofree char* snapshot = NULL;
    if (snapshot_json) {
        JsonObject* object = json_node_get_object(snapshot_json);
        json_object_set_string_member(object, "event", "windows");
        json_object_set_int_member(object, "revision", revision);
        g_autofree char* encoded = json_to_string(snapshot_json, FALSE);
        snapshot = g_strconcat(encoded, "\n", NULL);
    } else {
        snapshot = encode_response("", NULL, error ? error->message : "window listing unavailable");
    }
    GList* clients = g_hash_table_get_keys(control->clients);
    for (GList* item = clients; item; item = item->next) {
        Client* client = item->data;
        if (client->track_windows)
            send_response(client, g_strdup(snapshot));
        if (client->track_monitors && monitor_json && monitor_changed) {
            JsonObject* object = json_node_get_object(monitor_json);
            json_object_set_string_member(object, "event", "monitors");
            g_autofree char* encoded = json_to_string(monitor_json, FALSE);
            send_response(client, g_strconcat(encoded, "\n", NULL));
            json_object_remove_member(object, "event");
        }
    }
    g_list_free(clients);
    return G_SOURCE_REMOVE;
}

void schedule_windows(GnoblinNativeControl* control) {
    if (!control->publish_id) {
        control->state_revision++;
        control->publish_id = g_idle_add(publish_windows, control);
    }
}

static void window_changed(MetaWindow* window, gpointer user_data) {
    gnoblin_control_refresh_window_rule_blur(window);
    gnoblin_control_apply_window_rules(user_data, window);
    schedule_windows(user_data);
}

static gboolean mutter_signal_value_supported(GType type) {
    type &= ~G_SIGNAL_TYPE_STATIC_SCOPE;
    return type == G_TYPE_BOOLEAN || type == G_TYPE_CHAR || type == G_TYPE_UCHAR ||
           type == G_TYPE_INT || type == G_TYPE_UINT || type == G_TYPE_LONG ||
           type == G_TYPE_ULONG || type == G_TYPE_INT64 || type == G_TYPE_UINT64 ||
           type == G_TYPE_FLOAT || type == G_TYPE_DOUBLE || type == G_TYPE_STRING ||
           G_TYPE_IS_ENUM(type) || G_TYPE_IS_FLAGS(type);
}

static gboolean mutter_signal_argument_supported(GType type) {
    type &= ~G_SIGNAL_TYPE_STATIC_SCOPE;
    return mutter_signal_value_supported(type) || g_type_is_a(type, META_TYPE_WINDOW);
}

gboolean native_mutter_event_is_subscribable(GnoblinNativeControl* control, const char* name) {
    if (!control || !name || strlen(name) > 128 || !g_utf8_validate(name, -1, NULL) ||
        !g_str_has_prefix(name, "mutter."))
        return FALSE;

    if (g_str_equal(name, "mutter.touchpad.gesture") ||
        g_str_equal(name, "mutter.wayland.pointer-window-changed"))
        return TRUE;

    const char* source_start = name + strlen("mutter.");
    const char* separator = strchr(source_start, '.');
    if (!separator || separator == source_start || !separator[1])
        return FALSE;

    g_autofree char* source = g_strndup(source_start, separator - source_start);
    const char* signal = separator + 1;
    GType object_type = G_TYPE_INVALID;
    if (g_str_equal(source, "display") && control->display)
        object_type = G_OBJECT_TYPE(control->display);
    else if (g_str_equal(source, "window"))
        object_type = META_TYPE_WINDOW;
    else if (g_str_equal(source, "workspace"))
        object_type = META_TYPE_WORKSPACE;
    else if (g_str_equal(source, "workspace-manager") && control->workspace_manager)
        object_type = G_OBJECT_TYPE(control->workspace_manager);
    else if (g_str_equal(source, "backend") && control->backend)
        object_type = G_OBJECT_TYPE(control->backend);
    else if (g_str_equal(source, "monitor-manager") && control->monitor_manager)
        object_type = G_OBJECT_TYPE(control->monitor_manager);
    else if (g_str_equal(source, "cursor-tracker") && control->cursor_tracker)
        object_type = G_OBJECT_TYPE(control->cursor_tracker);
    if (object_type == G_TYPE_INVALID)
        return FALSE;

    guint signal_id = 0;
    GQuark detail = 0;
    if (!g_signal_parse_name(signal, object_type, &signal_id, &detail, FALSE))
        return FALSE;

    GSignalQuery query = {0};
    g_signal_query(signal_id, &query);
    gboolean supported = (query.return_type & ~G_SIGNAL_TYPE_STATIC_SCOPE) == G_TYPE_NONE;
    for (guint i = 0; supported && i < query.n_params; i++)
        supported = mutter_signal_argument_supported(query.param_types[i]);
    return supported;
}

static GVariant* mutter_signal_value_to_variant(const GValue* value) {
    GType type = G_VALUE_TYPE(value);
    type &= ~G_SIGNAL_TYPE_STATIC_SCOPE;
    if (type == G_TYPE_BOOLEAN)
        return g_variant_new_boolean(g_value_get_boolean(value));
    if (type == G_TYPE_CHAR)
        return g_variant_new_int64(g_value_get_schar(value));
    if (type == G_TYPE_UCHAR)
        return g_variant_new_uint64(g_value_get_uchar(value));
    if (type == G_TYPE_INT)
        return g_variant_new_int64(g_value_get_int(value));
    if (type == G_TYPE_UINT)
        return g_variant_new_uint64(g_value_get_uint(value));
    if (type == G_TYPE_LONG)
        return g_variant_new_int64(g_value_get_long(value));
    if (type == G_TYPE_ULONG)
        return g_variant_new_uint64(g_value_get_ulong(value));
    if (type == G_TYPE_INT64)
        return g_variant_new_int64(g_value_get_int64(value));
    if (type == G_TYPE_UINT64)
        return g_variant_new_uint64(g_value_get_uint64(value));
    if (type == G_TYPE_FLOAT)
        return g_variant_new_double(g_value_get_float(value));
    if (type == G_TYPE_DOUBLE)
        return g_variant_new_double(g_value_get_double(value));
    if (type == G_TYPE_STRING) {
        const char* string = g_value_get_string(value);
        if (string && !g_utf8_validate(string, -1, NULL))
            return NULL;
        return g_variant_new_string(string ? string : "");
    }
    if (G_TYPE_IS_ENUM(type))
        return g_variant_new_int64(g_value_get_enum(value));
    if (G_TYPE_IS_FLAGS(type))
        return g_variant_new_uint64(g_value_get_flags(value));
    return NULL;
}

static GVariant* mutter_signal_argument_to_variant(const GValue* value) {
    GType type = G_VALUE_TYPE(value) & ~G_SIGNAL_TYPE_STATIC_SCOPE;
    if (g_type_is_a(type, META_TYPE_WINDOW)) {
        MetaWindow* window = g_value_get_object(value);
        if (!window || !META_IS_WINDOW(window))
            return NULL;
        GVariantBuilder identity;
        g_variant_builder_init(&identity, G_VARIANT_TYPE_VARDICT);
        add_window_signal_identity(&identity, window);
        return g_variant_builder_end(&identity);
    }
    return mutter_signal_value_to_variant(value);
}

static void native_mutter_signal_watch_free(gpointer data, GClosure* closure) {
    (void)closure;
    NativeMutterSignalWatch* watch = data;
    g_free(watch->event);
    g_free(watch->source);
    g_free(watch->signal);
    g_free(watch);
}

static void add_window_signal_identity(GVariantBuilder* payload, MetaWindow* window) {
    const char* app_id = meta_window_get_gtk_application_id(window);
    const char* title = meta_window_get_title(window);
    g_variant_builder_add(payload, "{sv}", "window_id",
                          g_variant_new_uint32(meta_window_get_stable_sequence(window)));
    g_variant_builder_add(payload, "{sv}", "app_id", g_variant_new_string(app_id ? app_id : ""));
    g_variant_builder_add(payload, "{sv}", "window_title",
                          g_variant_new_string(title ? title : ""));
}

static void native_mutter_signal_marshal(GClosure* closure, GValue* return_value,
                                         guint n_param_values, const GValue* param_values,
                                         gpointer invocation_hint, gpointer marshal_data) {
    (void)return_value;
    (void)invocation_hint;
    (void)marshal_data;
    NativeMutterSignalWatch* watch = closure->data;
    GSignalQuery query = {0};
    g_signal_query(watch->signal_id, &query);
    if (!watch->control || n_param_values != query.n_params + 1)
        return;
    GVariantBuilder payload;
    g_variant_builder_init(&payload, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&payload, "{sv}", "source", g_variant_new_string(watch->source));
    g_variant_builder_add(&payload, "{sv}", "signal", g_variant_new_string(watch->signal));
    if (g_str_equal(watch->source, "window") && G_VALUE_HOLDS_OBJECT(&param_values[0])) {
        GObject* object = g_value_get_object(&param_values[0]);
        if (object && META_IS_WINDOW(object))
            add_window_signal_identity(&payload, META_WINDOW(object));
    }
    if (g_str_equal(watch->source, "workspace") && G_VALUE_HOLDS_OBJECT(&param_values[0])) {
        GObject* object = g_value_get_object(&param_values[0]);
        if (object && META_IS_WORKSPACE(object))
            g_variant_builder_add(
                &payload, "{sv}", "workspace_index",
                g_variant_new_int32(meta_workspace_index(META_WORKSPACE(object))));
    }
    for (guint i = 0; i < query.n_params; i++) {
        GType type = query.param_types[i] & ~G_SIGNAL_TYPE_STATIC_SCOPE;
        GVariant* raw_argument = mutter_signal_argument_to_variant(&param_values[i + 1]);
        if (!raw_argument) {
            g_variant_builder_clear(&payload);
            return;
        }
        g_autoptr(GVariant) argument = g_variant_ref_sink(raw_argument);
        g_autofree char* name = g_strdup_printf("arg%u", i);
        g_autofree char* type_name = g_strdup_printf("arg%u_type", i);
        g_variant_builder_add(&payload, "{sv}", name, argument);
        g_variant_builder_add(&payload, "{sv}", type_name, g_variant_new_string(g_type_name(type)));
    }
    g_autoptr(GVariant) event_payload = g_variant_ref_sink(g_variant_builder_end(&payload));
    g_autoptr(GError) error = NULL;
    if (!gnoblin_native_control_dispatch_runtime_event(watch->control->display, watch->event,
                                                       event_payload, NULL, &error))
        g_warning("gnoblin: could not dispatch Mutter event %s: %s", watch->event,
                  error ? error->message : "unknown error");
}

void gnoblin_control_clear_object_signal_watches(GObject* object, GArray* handler_ids) {
    if (!object || !handler_ids)
        return;
    for (guint i = 0; i < handler_ids->len; i++) {
        gulong handler_id = g_array_index(handler_ids, gulong, i);
        if (g_signal_handler_is_connected(object, handler_id))
            g_signal_handler_disconnect(object, handler_id);
    }
    g_array_set_size(handler_ids, 0);
}

void gnoblin_control_refresh_object_signal_watches(GnoblinNativeControl* control, GObject* object,
                                                   const char* source, GArray* handler_ids) {
    gnoblin_control_clear_object_signal_watches(object, handler_ids);
    if (!control || !object || !source || !handler_ids)
        return;
    g_autofree char* prefix = g_strdup_printf("mutter.%s.", source);
    GHashTableIter iter;
    gpointer key;
    g_autoptr(GHashTable) connected = g_hash_table_new_full(g_str_hash, g_str_equal, g_free, NULL);
    g_autoptr(GHashTable) subscriptions =
        g_hash_table_new_full(g_str_hash, g_str_equal, g_free, NULL);
    if (control->runtime_event_subscriptions) {
        GHashTableIter runtime_iter;
        gpointer runtime_key;
        g_hash_table_iter_init(&runtime_iter, control->runtime_event_subscriptions);
        while (g_hash_table_iter_next(&runtime_iter, &runtime_key, NULL)) {
            const char* event = runtime_key;
            if (g_str_equal(event, "*") || g_str_has_prefix(event, "mutter."))
                g_hash_table_add(subscriptions, g_strdup(event));
        }
    }
    if (control->clients) {
        GHashTableIter client_iter;
        gpointer client_value;
        g_hash_table_iter_init(&client_iter, control->clients);
        while (g_hash_table_iter_next(&client_iter, NULL, &client_value)) {
            Client* client = client_value;
            if (!client->event_subscriptions)
                continue;
            GHashTableIter event_iter;
            gpointer event_key;
            g_hash_table_iter_init(&event_iter, client->event_subscriptions);
            while (g_hash_table_iter_next(&event_iter, &event_key, NULL)) {
                const char* event = event_key;
                if (g_str_has_prefix(event, "mutter."))
                    g_hash_table_add(subscriptions, g_strdup(event));
            }
        }
    }
    g_hash_table_iter_init(&iter, subscriptions);
    while (g_hash_table_iter_next(&iter, &key, NULL)) {
        const char* event = key;
        if (g_str_equal(event, "*")) {
            if (g_str_equal(source, "display"))
                event = "mutter.display.restacked";
            else if (g_str_equal(source, "window"))
                event = "mutter.window.position-changed";
            else if (g_str_equal(source, "workspace-manager"))
                event = "mutter.workspace-manager.active-workspace-changed";
            else if (g_str_equal(source, "backend"))
                event = "mutter.backend.keymap-changed";
            else if (g_str_equal(source, "monitor-manager"))
                event = "mutter.monitor-manager.monitors-changed";
            else if (g_str_equal(source, "cursor-tracker"))
                event = "mutter.cursor-tracker.cursor-changed";
            else
                continue;
        }
        if (!g_str_has_prefix(event, prefix) || !event[strlen(prefix)])
            continue;
        if (g_hash_table_contains(connected, event))
            continue;
        g_hash_table_add(connected, g_strdup(event));
        const char* signal = event + strlen(prefix);
        guint signal_id = 0;
        GQuark detail = 0;
        if (!g_signal_parse_name(signal, G_OBJECT_TYPE(object), &signal_id, &detail, FALSE)) {
            g_warning("gnoblin: cannot subscribe to unknown Mutter event %s", event);
            continue;
        }
        GSignalQuery query = {0};
        g_signal_query(signal_id, &query);
        GType return_type = query.return_type & ~G_SIGNAL_TYPE_STATIC_SCOPE;
        gboolean supported = return_type == G_TYPE_NONE;
        for (guint i = 0; supported && i < query.n_params; i++)
            supported = mutter_signal_argument_supported(query.param_types[i]);
        if (!supported) {
            g_warning("gnoblin: Mutter event %s has a signal signature that Lua cannot represent",
                      event);
            continue;
        }
        NativeMutterSignalWatch* watch = g_new0(NativeMutterSignalWatch, 1);
        watch->control = control;
        watch->event = g_strdup(event);
        watch->source = g_strdup(source);
        watch->signal = g_strdup(signal);
        watch->signal_id = signal_id;
        GClosure* closure = g_closure_new_simple(sizeof(GClosure), watch);
        g_closure_set_marshal(closure, native_mutter_signal_marshal);
        g_closure_add_finalize_notifier(closure, watch, native_mutter_signal_watch_free);
        gulong handler_id =
            g_signal_connect_closure_by_id(object, signal_id, detail, closure, TRUE);
        g_array_append_val(handler_ids, handler_id);
    }
}

void clear_display_signal_watches(GnoblinNativeControl* control) {
    if (control && control->display)
        gnoblin_control_clear_object_signal_watches(G_OBJECT(control->display),
                                                    control->display_signal_handler_ids);
}

void refresh_display_signal_watches(GnoblinNativeControl* control) {
    if (control && control->display)
        gnoblin_control_refresh_object_signal_watches(
            control, G_OBJECT(control->display), "display", control->display_signal_handler_ids);
}

static gboolean workspace_signal_subscribed(GnoblinNativeControl* control) {
    if (!control)
        return FALSE;

    if (control->runtime_event_subscriptions) {
        GHashTableIter iter;
        gpointer key;
        g_hash_table_iter_init(&iter, control->runtime_event_subscriptions);
        while (g_hash_table_iter_next(&iter, &key, NULL)) {
            const char* event = key;
            if (g_str_equal(event, "*") || g_str_has_prefix(event, "mutter.workspace."))
                return TRUE;
        }
    }

    if (control->clients) {
        GHashTableIter clients;
        gpointer value;
        g_hash_table_iter_init(&clients, control->clients);
        while (g_hash_table_iter_next(&clients, NULL, &value)) {
            Client* client = value;
            if (!client->event_subscriptions)
                continue;
            GHashTableIter events;
            gpointer key;
            g_hash_table_iter_init(&events, client->event_subscriptions);
            while (g_hash_table_iter_next(&events, &key, NULL)) {
                const char* event = key;
                if (g_str_has_prefix(event, "mutter.workspace."))
                    return TRUE;
            }
        }
    }
    return FALSE;
}

gboolean gnoblin_control_workspace_signal_subscribed(GnoblinNativeControl* control) {
    return workspace_signal_subscribed(control);
}

void clear_backend_signal_watches(GnoblinNativeControl* control) {
    if (control && control->backend)
        gnoblin_control_clear_object_signal_watches(G_OBJECT(control->backend),
                                                    control->backend_signal_handler_ids);
}

void refresh_backend_signal_watches(GnoblinNativeControl* control) {
    if (control && control->backend)
        gnoblin_control_refresh_object_signal_watches(
            control, G_OBJECT(control->backend), "backend", control->backend_signal_handler_ids);
}

void clear_cursor_tracker_signal_watches(GnoblinNativeControl* control) {
    if (control && control->cursor_tracker)
        gnoblin_control_clear_object_signal_watches(G_OBJECT(control->cursor_tracker),
                                                    control->cursor_tracker_signal_handler_ids);
}

void refresh_cursor_tracker_signal_watches(GnoblinNativeControl* control) {
    if (control && control->cursor_tracker)
        gnoblin_control_refresh_object_signal_watches(control, G_OBJECT(control->cursor_tracker),
                                                      "cursor-tracker",
                                                      control->cursor_tracker_signal_handler_ids);
}

static void refresh_window_signal_watches(GnoblinNativeControl* control, MetaWindow* window) {
    if (!control || !control->window_signal_handler_ids || !window)
        return;
    GArray* handler_ids = g_hash_table_lookup(control->window_signal_handler_ids, window);
    if (handler_ids)
        gnoblin_control_refresh_object_signal_watches(control, G_OBJECT(window), "window",
                                                      handler_ids);
}

void refresh_all_window_signal_watches(GnoblinNativeControl* control) {
    if (!control || !control->window_signal_handler_ids)
        return;
    GHashTableIter iter;
    gpointer window;
    gpointer handler_ids;
    g_hash_table_iter_init(&iter, control->window_signal_handler_ids);
    while (g_hash_table_iter_next(&iter, &window, &handler_ids))
        gnoblin_control_refresh_object_signal_watches(control, G_OBJECT(window), "window",
                                                      handler_ids);
}

void native_refresh_all_mutter_signal_watches(GnoblinNativeControl* control) {
    if (!control)
        return;
    refresh_display_signal_watches(control);
    refresh_all_window_signal_watches(control);
    gnoblin_control_refresh_all_workspace_signal_watches(control);
    gnoblin_control_refresh_workspace_manager_signal_watches(control);
    refresh_backend_signal_watches(control);
    gnoblin_control_refresh_monitor_manager_signal_watches(control);
    refresh_cursor_tracker_signal_watches(control);
}

static void window_workspace_changed(MetaWindow* window, gpointer user_data) {
    gnoblin_control_apply_window_rules(user_data, window);
    schedule_windows(user_data);
}

static void window_notified(GObject* window, GParamSpec* property, gpointer user_data) {
    gnoblin_control_place_window_by_rules(user_data, META_WINDOW(window));
    gnoblin_control_apply_window_rules(user_data, META_WINDOW(window));
    schedule_windows(user_data);
}

static void window_unmanaged(MetaWindow* window, gpointer user_data) {
    GnoblinNativeControl* control = user_data;
    g_autofree char* window_id = native_window_id(window);
    if (control->window_input_sources)
        g_hash_table_remove(control->window_input_sources, window_id);
    GArray* handler_ids = g_hash_table_lookup(control->window_signal_handler_ids, window);
    gnoblin_control_clear_object_signal_watches(G_OBJECT(window), handler_ids);
    g_hash_table_remove(control->window_signal_handler_ids, window);
    g_signal_handlers_disconnect_by_data(window, control);
    g_hash_table_remove(control->snap_restore_frames, window);
    g_hash_table_remove(control->windows, window);
    schedule_windows(control);
}

void track_window(GnoblinNativeControl* control, MetaWindow* window) {
    if (g_hash_table_contains(control->windows, window))
        return;
    g_hash_table_add(control->windows, g_object_ref(window));
    GArray* handler_ids = g_array_new(FALSE, FALSE, sizeof(gulong));
    g_hash_table_insert(control->window_signal_handler_ids, window, handler_ids);
    g_signal_connect(window, "notify", G_CALLBACK(window_notified), control);
    g_signal_connect(window, "position-changed", G_CALLBACK(window_changed), control);
    g_signal_connect(window, "size-changed", G_CALLBACK(window_changed), control);
    g_signal_connect(window, "workspace-changed", G_CALLBACK(window_workspace_changed), control);
    g_signal_connect(window, "unmanaged", G_CALLBACK(window_unmanaged), control);
    refresh_window_signal_watches(control, window);
    gnoblin_control_apply_window_rules(control, window);
    schedule_windows(control);
}

void gnoblin_native_control_track_layer_window(MetaDisplay* display, MetaWindow* window) {
    GnoblinNativeControl* control =
        display ? g_object_get_data(G_OBJECT(display), NATIVE_CONTROL_OBJECT_DATA_KEY) : NULL;
    if (!control || control->stopping || !window)
        return;
    track_window(control, window);
}

void gnoblin_native_control_layer_changed(MetaDisplay* display, MetaWindow* window) {
    GnoblinNativeControl* control =
        display ? g_object_get_data(G_OBJECT(display), NATIVE_CONTROL_OBJECT_DATA_KEY) : NULL;
    if (!control || control->stopping)
        return;
    MetaWaylandSurface* surface = meta_window_get_wayland_surface(window);
    gboolean mapped = surface && meta_wayland_surface_get_buffer(surface);
    gpointer monitor = GINT_TO_POINTER(meta_window_get_monitor(window) + 2);
    gboolean was_mapped = g_object_get_data(G_OBJECT(window), "gnoblin-ui-layer-mapped") != NULL;
    gpointer previous_monitor = g_object_get_data(G_OBJECT(window), "gnoblin-ui-layer-monitor");
    g_object_set_data(G_OBJECT(window), "gnoblin-ui-layer-mapped", GINT_TO_POINTER(mapped));
    g_object_set_data(G_OBJECT(window), "gnoblin-ui-layer-monitor", monitor);
    if (mapped != was_mapped || (mapped && monitor != previous_monitor))
        meta_stack_update_transient(display->stack);
    schedule_windows(control);
}

void window_created(MetaDisplay* display, MetaWindow* window, gpointer user_data) {
    track_window(user_data, window);
    gnoblin_control_place_window_by_rules(user_data, window);
}

void display_notified(GObject* display, GParamSpec* property, gpointer user_data) {
    if (g_str_equal(property->name, "focus-window")) {
        gnoblin_control_input_source_focus_changed(user_data);
        gnoblin_control_apply_all_window_rules(user_data);
    }
    schedule_windows(user_data);
}

void display_restacked(MetaDisplay* display, gpointer user_data) {
    schedule_windows(user_data);
}

void workspace_manager_changed(MetaWorkspaceManager* manager, gpointer user_data) {
    schedule_windows(user_data);
}

void monitor_manager_changed(MetaMonitorManager* manager, gpointer user_data) {
    schedule_windows(user_data);
    native_monitor_privacy_screen_publish(user_data);
}

void display_work_areas_changed(MetaDisplay* display, gpointer user_data) {
    (void)display;
    /* A panel with an exclusive zone changes the work area without changing the monitors. */
    schedule_windows(user_data);
}

void monitor_privacy_screen_changed(MetaMonitorManager* manager,
                                    MetaLogicalMonitor* logical_monitor, gboolean enabled,
                                    gpointer user_data) {
    (void)manager;
    (void)logical_monitor;
    (void)enabled;
    native_monitor_privacy_screen_publish(user_data);
}

void backend_keymap_changed(MetaBackend* backend, gpointer user_data) {
    (void)backend;
    schedule_windows(user_data);
}

void backend_keymap_layout_group_changed(MetaBackend* backend, guint group, gpointer user_data) {
    (void)backend;
    (void)group;
    schedule_windows(user_data);
}

void input_device_added(ClutterSeat* seat, ClutterInputDevice* device, gpointer user_data) {
    (void)seat;
    (void)device;
    schedule_windows(user_data);
}

void input_device_removed(ClutterSeat* seat, ClutterInputDevice* device, gpointer user_data) {
    (void)seat;
    GnoblinNativeControl* control = user_data;
    if (control->pending_input_events && !g_queue_is_empty(control->pending_input_events))
        native_input_cancel_pending(
            control, !(control->wayland_compositor &&
                       meta_wayland_session_lock_is_active(control->wayland_compositor)));
    const char* removed_id = g_hash_table_lookup(control->input_device_ids, device);
    if (removed_id && control->held_input_dispositions) {
        g_autofree char* part = g_strdup_printf(":%s:", removed_id);
        GHashTableIter held;
        gpointer key;
        g_hash_table_iter_init(&held, control->held_input_dispositions);
        while (g_hash_table_iter_next(&held, &key, NULL))
            if (strstr(key, part))
                g_hash_table_iter_remove(&held);
    }
    g_hash_table_remove(control->input_device_ids, device);
    schedule_windows(control);
}
