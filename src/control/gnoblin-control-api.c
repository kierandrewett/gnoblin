#include "gnoblin-native-control-private.h"

gboolean gnoblin_native_control_dispatch_runtime_event(MetaDisplay* display, const char* event,
                                                       GVariant* payload, gboolean* claimed,
                                                       GError** error) {
    GnoblinNativeControl* control =
        g_object_get_data(G_OBJECT(display), NATIVE_CONTROL_OBJECT_DATA_KEY);
    if (!control || !control->supervised_runtime || !event || !*event || !payload ||
        !g_variant_is_of_type(payload, G_VARIANT_TYPE_VARDICT)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "invalid supervised runtime event");
        return FALSE;
    }
    if (claimed)
        *claimed = FALSE;
    g_autoptr(GVariant) sequenced_payload = NULL;
    if (g_str_equal(event, "mutter.touchpad.gesture")) {
        if (control->input_gesture_sequence >= G_MAXINT64) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NO_SPACE,
                                "touchpad gesture sequence is exhausted");
            return FALSE;
        }
        GVariantBuilder builder;
        GVariantIter iterator;
        const char* key;
        GVariant* value;
        g_variant_builder_init(&builder, G_VARIANT_TYPE_VARDICT);
        g_variant_iter_init(&iterator, payload);
        while (g_variant_iter_next(&iterator, "{&sv}", &key, &value)) {
            g_autoptr(GVariant) field = value;
            if (g_str_equal(key, "sequence"))
                continue;
            g_variant_builder_add(&builder, "{sv}", key, field);
        }
        control->input_gesture_sequence++;
        g_variant_builder_add(&builder, "{sv}", "sequence",
                              g_variant_new_int64((gint64)control->input_gesture_sequence));
        sequenced_payload = g_variant_ref_sink(g_variant_builder_end(&builder));
        payload = sequenced_payload;
    }
    if (g_str_equal(event, "mutter.touchpad.gesture") &&
        g_variant_is_of_type(payload, G_VARIANT_TYPE_VARDICT) && claimed)
        *claimed = native_config_event(display, event, NULL, payload, control);
    native_publish_mutter_socket_event(control, event, payload);
    if (!native_runtime_dispatch_event(control, event, payload)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_FAILED,
                            "could not queue supervised runtime event");
        return FALSE;
    }
    return TRUE;
}

gboolean gnoblin_native_control_is_session(MetaDisplay* display) {
    GnoblinNativeControl* control =
        display ? g_object_get_data(G_OBJECT(display), NATIVE_CONTROL_OBJECT_DATA_KEY) : NULL;
    if (!display)
        return bootstrap_runtime_cache != NULL;
    return control && control->supervised_runtime;
}

gboolean gnoblin_native_control_is_supervised(MetaDisplay* display) {
    return gnoblin_native_control_is_session(display);
}

GnoblinNativeControl* gnoblin_native_control_start(MetaContext* context, GVariant* document,
                                                   int runtime_fd, guint64 settings_revision,
                                                   GError** error) {
    g_autoptr(GVariant) initial_session_lock = NULL;
    g_autoptr(GSocketAddress) bind_address = NULL;
    g_autoptr(GList) windows = NULL;
    g_autofree char* socket_directory = NULL;
    g_autoptr(GError) snapshot_error = NULL;
    g_autoptr(JsonNode) initial_snapshot = NULL;
    g_autoptr(GError) workspace_error = NULL;
    g_autoptr(JsonNode) initial_workspaces = NULL;
    g_autoptr(GError) monitor_error = NULL;
    g_autoptr(JsonNode) initial_monitors = NULL;
    g_autoptr(JsonNode) initial_outputs = NULL;
    g_autoptr(GError) layer_error = NULL;
    g_autoptr(JsonNode) initial_layer_snapshot = NULL;
    g_autoptr(GVariant) capabilities = NULL;
    g_autoptr(GVariant) input_devices = NULL;
    g_autoptr(GVariant) input_sources = NULL;
    g_autoptr(GVariant) appearance = NULL;
    g_autoptr(JsonNode) initial_input_devices = NULL;
    if (runtime_fd < 0 || !document || !g_variant_is_of_type(document, G_VARIANT_TYPE_VARDICT)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "native control requires a validated Lua-supervisor bootstrap");
        return NULL;
    }
    gnoblin_restore_interactive_priority("compositor");
    const char* runtime = g_getenv("XDG_RUNTIME_DIR");
    if (!runtime || !*runtime) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "XDG_RUNTIME_DIR is required for native control");
        return NULL;
    }
    g_autofree char* directory = g_build_filename(runtime, "gnoblin", NULL);
    if (g_mkdir_with_parents(directory, 0700) != 0) {
        g_set_error(error, G_IO_ERROR, g_io_error_from_errno(errno), "cannot create %s: %s",
                    directory, g_strerror(errno));
        return NULL;
    }
    struct stat directory_stat;
    if (lstat(directory, &directory_stat) != 0 || !S_ISDIR(directory_stat.st_mode) ||
        directory_stat.st_uid != getuid() || (directory_stat.st_mode & 077) != 0) {
        g_set_error(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                    "%s must be a private directory owned by this user", directory);
        return NULL;
    }
    GnoblinNativeControl* control = g_new0(GnoblinNativeControl, 1);
    control->runtime_fd = -1;
    control->supervised_runtime = TRUE;
    control->session_lifecycle_supported =
        g_strcmp0(g_getenv("GNOBLIN_SESSION_LIFECYCLE_VERSION"), "1") == 0;
    control->runtime_fd = runtime_fd;
    control->runtime_generation = bootstrap_runtime_generation;
    control->runtime_cache = gnoblin_runtime_cache_new();
    control->runtime_reader = gnoblin_runtime_reader_new();
    control->runtime_writer = gnoblin_runtime_writer_new();
    if (!gnoblin_runtime_cache_replace(control->runtime_cache, document, settings_revision,
                                       error)) {
        gnoblin_runtime_cache_free(control->runtime_cache);
        gnoblin_runtime_reader_free(control->runtime_reader);
        gnoblin_runtime_writer_free(control->runtime_writer);
        g_free(control);
        return NULL;
    }
    control->clients = g_hash_table_new(g_direct_hash, g_direct_equal);
    control->ui_sessions =
        g_hash_table_new_full(g_str_hash, g_str_equal, g_free, native_ui_session_free);
    control->window_shader_files = g_hash_table_new_full(g_str_hash, g_str_equal, g_free,
                                                         gnoblin_control_window_shader_file_free);
    control->pending_runtime_requests =
        g_hash_table_new_full(g_int64_hash, g_int64_equal, g_free, pending_runtime_request_free);
    control->pending_location_authorizations = g_hash_table_new_full(
        g_int64_hash, g_int64_equal, g_free, gnoblin_control_pending_location_authorization_free);
    gnoblin_control_auth_init(control);
    gnoblin_control_prompts_init(control);
    gnoblin_control_capture_init(control);
    control->runtime_operation_ids =
        g_hash_table_new_full(g_int64_hash, g_int64_equal, g_free, NULL);
    control->runtime_cancelled_operation_ids =
        g_hash_table_new_full(g_int64_hash, g_int64_equal, g_free, NULL);
    control->runtime_event_subscriptions =
        g_hash_table_new_full(g_str_hash, g_str_equal, g_free, NULL);
    control->display_signal_handler_ids = g_array_new(FALSE, FALSE, sizeof(gulong));
    control->workspace_manager_signal_handler_ids = g_array_new(FALSE, FALSE, sizeof(gulong));
    control->workspace_signal_handler_ids = g_hash_table_new_full(
        g_direct_hash, g_direct_equal, g_object_unref, (GDestroyNotify)g_array_unref);
    control->backend_signal_handler_ids = g_array_new(FALSE, FALSE, sizeof(gulong));
    control->monitor_manager_signal_handler_ids = g_array_new(FALSE, FALSE, sizeof(gulong));
    control->cursor_tracker_signal_handler_ids = g_array_new(FALSE, FALSE, sizeof(gulong));
    control->window_signal_handler_ids =
        g_hash_table_new_full(g_direct_hash, g_direct_equal, NULL, (GDestroyNotify)g_array_unref);
    control->pending_runtime_states = g_queue_new();
    control->pending_runtime_events = g_queue_new();
    control->pending_input_events = g_queue_new();
    control->held_input_dispositions = g_hash_table_new_full(g_str_hash, g_str_equal, g_free, NULL);
    control->privacy_handles =
        g_hash_table_new_full(g_direct_hash, g_direct_equal, g_object_unref, NULL);
    control->privacy_revision = 1;
    control->dynamic_shortcuts = g_hash_table_new_full(g_direct_hash, g_direct_equal, NULL,
                                                       gnoblin_control_dynamic_shortcut_free);
    control->touchpad_router = gnoblin_touchpad_router_new();
    control->focus_contexts = g_hash_table_new_full(g_int64_hash, g_int64_equal, g_free, g_free);
    control->window_input_sources = g_hash_table_new_full(g_str_hash, g_str_equal, g_free,
                                                          gnoblin_control_window_input_source_free);
    control->menu_contexts =
        g_hash_table_new_full(g_int64_hash, g_int64_equal, g_free, native_menu_context_free);
    control->text_targets = g_hash_table_new_full(g_str_hash, g_str_equal, g_free, g_free);
    control->window_drags = g_hash_table_new_full(g_int64_hash, g_int64_equal, g_free,
                                                  gnoblin_control_window_drag_free);
    control->snap_contexts =
        g_hash_table_new_full(g_str_hash, g_str_equal, g_free, native_snap_context_free);
    control->snap_restore_frames =
        g_hash_table_new_full(g_direct_hash, g_direct_equal, NULL, g_free);
    control->focus_context_timeout_id =
        g_timeout_add_seconds(1, focus_context_expiry_tick, control);
    control->launches = g_ptr_array_new_with_free_func(native_launch_free);
    update_launch_snapshot(control);
    control->wayland_compositor = meta_context_get_wayland_compositor(context);
    if (control->wayland_compositor)
        control->session_lock_callback_id = meta_wayland_session_lock_add_state_changed_callback(
            control->wayland_compositor, native_session_lock_changed, control, NULL);
    initial_session_lock =
        native_session_lock_snapshot(control->wayland_compositor, control->session_lock_revision);
    native_publish_runtime_snapshot(control, "session-lock", initial_session_lock,
                                    control->session_lock_revision);
    control->windows = g_hash_table_new_full(g_direct_hash, g_direct_equal, g_object_unref, NULL);
    control->window_state =
        g_hash_table_new_full(g_str_hash, g_str_equal, g_free, native_window_state_free);
    control->layer_state =
        g_hash_table_new_full(g_str_hash, g_str_equal, g_free, gnoblin_control_layer_state_free);
    control->input_device_ids = g_hash_table_new_full(g_direct_hash, g_direct_equal, NULL, g_free);
    control->display = meta_context_get_display(context);
    control->overlay_modifier_hook_available = GPOINTER_TO_INT(
        g_object_get_data(G_OBJECT(control->display), OVERLAY_MODIFIER_HOOK_AVAILABLE_DATA_KEY));
    g_object_set_data(G_OBJECT(control->display), NATIVE_CONTROL_OBJECT_DATA_KEY, control);
    NativeConsoleToggleBridge* console_bridge =
        g_object_get_data(G_OBJECT(control->display), NATIVE_CONSOLE_TOGGLE_BRIDGE_DATA_KEY);
    if (console_bridge) {
        control->console_toggle_handler = console_bridge->handler;
        control->console_toggle_handler_data = console_bridge->user_data;
    }
    /* The compositor plugin registers its debug overlay before this object exists. */
    NativeDebugBridge* debug_bridge =
        g_object_get_data(G_OBJECT(control->display), NATIVE_DEBUG_BRIDGE_DATA_KEY);
    if (debug_bridge) {
        control->debug_handler = debug_bridge->handler;
        control->debug_handler_data = debug_bridge->user_data;
    }
    NativeBackgroundBridge* background_bridge =
        g_object_get_data(G_OBJECT(control->display), NATIVE_BACKGROUND_BRIDGE_DATA_KEY);
    if (background_bridge) {
        control->background_handler = background_bridge->handler;
        control->background_handler_data = background_bridge->user_data;
    }
    MetaBackend* backend = meta_context_get_backend(context);
    control->backend = backend;
    control->orientation_manager = meta_backend_get_orientation_manager(backend);
    control->cursor_tracker = meta_backend_get_cursor_tracker(backend);
    control->remote_access_controller =
        g_object_ref(meta_backend_get_remote_access_controller(backend));
    control->monitor_manager = meta_backend_get_monitor_manager(backend);
    control->input_seat = meta_backend_get_default_seat(backend);
    control->workspace_manager = meta_display_get_workspace_manager(control->display);
    control->path = g_strdup(g_getenv("GNOBLIN_COMPOSITOR_SOCKET"));
    if (!control->path)
        control->path = g_build_filename(directory, "compositor-v1.sock", NULL);
    socket_directory = g_path_get_dirname(control->path);
    if (!meta_gnoblin_initialize_native_workspaces(control->display, error))
        goto fail;
    struct stat socket_directory_stat;
    if (lstat(socket_directory, &socket_directory_stat) != 0 ||
        !S_ISDIR(socket_directory_stat.st_mode) || socket_directory_stat.st_uid != getuid() ||
        (socket_directory_stat.st_mode & 077) != 0) {
        g_set_error(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                    "%s must be a private directory owned by this user", socket_directory);
        goto fail;
    }

    struct stat existing;
    if (lstat(control->path, &existing) == 0) {
        g_autoptr(GSocketClient) probe = NULL;
        g_autoptr(GSocketAddress) address = NULL;
        g_autoptr(GError) probe_error = NULL;
        g_autoptr(GSocketConnection) active = NULL;
        if (!S_ISSOCK(existing.st_mode) || existing.st_uid != getuid()) {
            g_set_error(error, G_IO_ERROR, G_IO_ERROR_EXISTS, "refusing to replace %s",
                        control->path);
            goto fail;
        }
        probe = g_socket_client_new();
        g_socket_client_set_timeout(probe, 1);
        address = g_unix_socket_address_new(control->path);
        active = g_socket_client_connect(probe, G_SOCKET_CONNECTABLE(address), NULL, &probe_error);
        if (active || !g_error_matches(probe_error, G_IO_ERROR, G_IO_ERROR_CONNECTION_REFUSED)) {
            g_set_error(error, G_IO_ERROR, G_IO_ERROR_EXISTS,
                        "Gnoblin compositor socket is already active: %s", control->path);
            goto fail;
        }
        if (g_unlink(control->path) != 0) {
            g_set_error(error, G_IO_ERROR, g_io_error_from_errno(errno),
                        "cannot replace stale socket %s: %s", control->path, g_strerror(errno));
            goto fail;
        }
    }
    control->service = g_socket_service_new();
    bind_address = g_unix_socket_address_new(control->path);
    if (!g_socket_listener_add_address(G_SOCKET_LISTENER(control->service), bind_address,
                                       G_SOCKET_TYPE_STREAM, G_SOCKET_PROTOCOL_DEFAULT, NULL, NULL,
                                       error))
        goto fail;
    if (lstat(control->path, &existing) == 0 && S_ISSOCK(existing.st_mode)) {
        control->device = existing.st_dev;
        control->inode = existing.st_ino;
    }
    if (!control->inode || g_chmod(control->path, 0600) != 0) {
        g_set_error(error, G_IO_ERROR, g_io_error_from_errno(errno),
                    "cannot protect native control socket %s: %s", control->path,
                    g_strerror(errno));
        goto fail;
    }
    g_signal_connect(control->service, "incoming", G_CALLBACK(client_connected), control);
    g_socket_service_start(control->service);
    if (control->supervised_runtime) {
        control->runtime_read_source_id =
            g_unix_fd_add(control->runtime_fd, G_IO_IN | G_IO_HUP | G_IO_ERR | G_IO_NVAL,
                          native_runtime_fd_ready, control);
    }
    gnoblin_control_activity_publish(control, FALSE, FALSE, 120000, 0);
    start_native_policy_dbus(control); /* D-Bus absence keeps portal grants fail closed. */
    g_signal_connect(control->display, "gnoblin-native-event", G_CALLBACK(native_native_event),
                     control);
    native_settings_changed(gnoblin_runtime_cache_get_settings_revision(control->runtime_cache),
                            control);
    g_signal_connect(control->display, "window-created", G_CALLBACK(window_created), control);
    g_signal_connect(control->display, "notify::focus-window", G_CALLBACK(display_notified),
                     control);
    g_signal_connect(control->display, "restacked", G_CALLBACK(display_restacked), control);
    g_signal_connect(control->remote_access_controller, "new-handle",
                     G_CALLBACK(gnoblin_control_privacy_new_handle), control);
    g_signal_connect(control->workspace_manager, "workspace-added",
                     G_CALLBACK(workspace_manager_index_changed), control);
    g_signal_connect(control->workspace_manager, "workspace-removed",
                     G_CALLBACK(workspace_manager_index_changed), control);
    g_signal_connect(control->workspace_manager, "workspaces-reordered",
                     G_CALLBACK(workspace_manager_changed), control);
    g_signal_connect(control->workspace_manager, "active-workspace-changed",
                     G_CALLBACK(workspace_manager_changed), control);
    g_signal_connect(control->workspace_manager, "gnoblin-workspace-state-changed",
                     G_CALLBACK(workspace_manager_changed), control);
    if (control->monitor_manager)
        g_signal_connect(control->monitor_manager, "monitors-changed",
                         G_CALLBACK(monitor_manager_changed), control);
    g_signal_connect(control->display, "workareas-changed", G_CALLBACK(display_work_areas_changed),
                     control);
    if (control->monitor_manager)
        g_signal_connect(control->monitor_manager, "monitor-privacy-screen-changed",
                         G_CALLBACK(monitor_privacy_screen_changed), control);
    g_signal_connect(control->backend, "keymap-layout-group-changed",
                     G_CALLBACK(backend_keymap_layout_group_changed), control);
    g_signal_connect(control->backend, "keymap-changed", G_CALLBACK(backend_keymap_changed),
                     control);
    if (control->input_seat) {
        g_signal_connect(control->input_seat, "device-added", G_CALLBACK(input_device_added),
                         control);
        g_signal_connect(control->input_seat, "device-removed", G_CALLBACK(input_device_removed),
                         control);
    }
    GSettingsSchemaSource* schema_source = g_settings_schema_source_get_default();
    GSettingsSchema* appearance_schema =
        schema_source
            ? g_settings_schema_source_lookup(schema_source, "org.gnome.desktop.interface", TRUE)
            : NULL;
    if (appearance_schema && g_settings_schema_has_key(appearance_schema, "color-scheme")) {
        control->appearance_settings = g_settings_new_full(appearance_schema, NULL, NULL);
        g_signal_connect(control->appearance_settings, "changed::color-scheme",
                         G_CALLBACK(appearance_color_scheme_changed), control);
    } else {
        g_warning(
            "gnoblin-native-control: org.gnome.desktop.interface/color-scheme is unavailable; "
            "Lua appearance reads and change events are unavailable");
    }
    if (appearance_schema)
        g_settings_schema_unref(appearance_schema);
    control->appearance_revision = 1;
    appearance = appearance_snapshot_new(control);
    native_publish_runtime_snapshot(control, "appearance", appearance,
                                    control->appearance_revision);
    gnoblin_control_input_sources_refresh(control, document);
    gnoblin_control_input_sources_start_ibus(control);
    windows = meta_display_list_all_windows(control->display);
    for (GList* item = windows; item; item = item->next)
        track_window(control, item->data);
    gnoblin_control_input_source_focus_changed(control);
    initial_snapshot = window_snapshot_json(control, TRUE, &snapshot_error);
    if (!initial_snapshot)
        g_warning("gnoblin-native-control: cannot seed Lua window snapshot: %s",
                  snapshot_error ? snapshot_error->message : "window listing unavailable");
    initial_workspaces = gnoblin_control_workspace_snapshot_json(control, &workspace_error);
    if (!initial_workspaces)
        g_warning("gnoblin-native-control: cannot seed Lua workspace snapshot: %s",
                  workspace_error ? workspace_error->message : "workspace listing unavailable");
    else {
        control->workspace_state =
            gnoblin_control_workspace_state_from_snapshot(initial_workspaces);
        control->workspace_state_initialized = TRUE;
    }
    initial_monitors = gnoblin_control_monitor_snapshot_json(control, TRUE, &monitor_error);
    if (!initial_monitors)
        g_warning("gnoblin-native-control: cannot seed monitor snapshot: %s",
                  monitor_error ? monitor_error->message : "monitor listing unavailable");
    else {
        control->monitor_state = gnoblin_control_monitor_state_from_snapshot(initial_monitors);
        control->monitor_state_initialized = TRUE;
    }
    g_clear_error(&monitor_error);
    initial_outputs = gnoblin_control_output_snapshot_json(control, TRUE, &monitor_error);
    if (!initial_outputs)
        g_warning("gnoblin-native-control: cannot seed output snapshot: %s",
                  monitor_error ? monitor_error->message : "output listing unavailable");
    g_signal_connect(control->display, "show-osd", G_CALLBACK(native_show_osd_requested), control);
    g_signal_connect(control->display, "show-pad-osd", G_CALLBACK(native_show_pad_osd_requested),
                     control);
    initial_layer_snapshot = gnoblin_control_layer_snapshot_json(control, TRUE, &layer_error);
    if (!initial_layer_snapshot)
        g_warning("gnoblin-native-control: cannot seed Lua layer snapshot: %s",
                  layer_error ? layer_error->message : "layer listing unavailable");
    else {
        g_hash_table_unref(control->layer_state);
        control->layer_state = gnoblin_control_layer_state_from_snapshot(initial_layer_snapshot);
        control->layer_state_initialized = TRUE;
    }
    capabilities = capability_snapshot(control);
    native_publish_runtime_snapshot(control, "capabilities", capabilities, control->state_revision);
    input_devices = input_device_snapshot(control);
    native_publish_runtime_snapshot(control, "input-devices", input_devices,
                                    control->state_revision);
    input_sources = gnoblin_control_input_source_snapshot(control);
    native_publish_runtime_snapshot(control, "input-sources", input_sources,
                                    control->state_revision);
    gnoblin_control_publish_privacy_snapshot(control, FALSE);
    control->input_source_state_initialized = TRUE;
    NativeInputSource* initial_current_source = gnoblin_control_current_input_source(control);
    control->last_published_input_source =
        initial_current_source ? g_strdup(initial_current_source->id) : NULL;
    initial_input_devices = json_from_variant(input_devices);
    control->input_device_state =
        gnoblin_control_input_device_state_from_snapshot(initial_input_devices);
    control->input_device_state_initialized = TRUE;
#ifdef HAVE_REMOTE_DESKTOP
    control->pipewire_monitor =
        gnoblin_pipewire_monitor_new(NULL, privacy_pipewire_state_changed, control, NULL);
    if (control->pipewire_monitor)
        gnoblin_pipewire_monitor_start(control->pipewire_monitor);
#endif
    gnoblin_control_location_agent_create(control);
    if (control->location_agent) {
        g_autoptr(GVariant) initial_config = native_config_document(control);
        native_apply_location_policy(control, initial_config);
        gnoblin_location_agent_start(control->location_agent);
    }
    {
        g_autoptr(GVariant) prompts_config = native_config_document(control);
        gnoblin_control_auth_apply_config(control, prompts_config);
        gnoblin_control_prompts_apply_config(control, prompts_config);
    }
    schedule_windows(control);
    if (control->input_sources && control->input_sources->len > 0 &&
        !control->input_keymap_description) {
        NativeInputSource* first = g_ptr_array_index(control->input_sources, 0);
        if (!gnoblin_control_select_xkb_source(control, first->id, 0, "input.select_source", FALSE,
                                               error))
            goto fail;
    }
    if (!apply_native_input(control, context, document, error))
        goto fail;
    if (control->orientation_manager) {
        g_signal_connect(control->orientation_manager, "notify::orientation-locked",
                         G_CALLBACK(native_orientation_manager_notified), control);
        g_signal_connect(control->orientation_manager, "notify::has-accelerometer",
                         G_CALLBACK(native_orientation_manager_accelerometer_notified), control);
        g_signal_connect(control->orientation_manager, "orientation-changed",
                         G_CALLBACK(native_orientation_manager_orientation_changed), control);
    }
    native_orientation_lock_publish(control);
    if (!apply_native_keybindings(control, document, error))
        goto fail;
    if (!start_native_shortcuts(control, document, error))
        goto fail;
    if (control->supervised_runtime) {
        GVariantBuilder hello;
        g_variant_builder_init(&hello, G_VARIANT_TYPE_VARDICT);
        g_variant_builder_add(&hello, "{sv}", "role", g_variant_new_string("compositor"));
        g_variant_builder_add(&hello, "{sv}", "document_version", g_variant_new_uint32(1));
        if (control->session_lifecycle_supported)
            g_variant_builder_add(&hello, "{sv}", "session_lifecycle_version",
                                  g_variant_new_uint32(1));
        gnoblin_control_runtime_add_display_environment(&hello);
        g_autoptr(GVariant) payload = g_variant_ref_sink(g_variant_builder_end(&hello));
        if (!native_runtime_send(control, GNOBLIN_RUNTIME_PACKET_HELLO, 0, payload, error))
            goto fail;
        control->runtime_hello_sent = TRUE;
        if (!native_runtime_flush_state_snapshots(control, error) ||
            !native_runtime_flush_pending_events(control, error))
            goto fail;
        native_runtime_publish_status(control);
    }
#ifdef HAVE_XWAYLAND
    if (control->display->x11_display)
        meta_x11_display_update_gnoblin_xsettings(control->display->x11_display);
#endif
    return control;

fail:
    gnoblin_native_control_stop(control);
    return NULL;
}

void gnoblin_native_control_stop(GnoblinNativeControl* control) {
    if (!control)
        return;
    control->stopping = TRUE;
    if (control->privacy_camera_disable_source_id) {
        g_source_remove(control->privacy_camera_disable_source_id);
        control->privacy_camera_disable_source_id = 0;
    }
    gnoblin_control_clear_pending_location_authorizations(control);
    g_clear_pointer(&control->location_agent, gnoblin_location_agent_free);
    gnoblin_control_auth_stop(control);
    gnoblin_control_prompts_stop(control);
    gnoblin_control_capture_clear(control);
    control->overlay_modifier_hook_available = FALSE;
    if (control->active_shortcut_session)
        dynamic_shortcut_end_session(control, control->active_shortcut_session,
                                     "compositor_stopped");
    gnoblin_control_corner_toolkit_cache_shutdown(control);
#ifdef HAVE_REMOTE_DESKTOP
    g_clear_pointer(&control->pipewire_monitor, gnoblin_pipewire_monitor_free);
#endif
    if (control->runtime_read_source_id) {
        g_source_remove(control->runtime_read_source_id);
        control->runtime_read_source_id = 0;
    }
    if (control->runtime_write_source_id) {
        g_source_remove(control->runtime_write_source_id);
        control->runtime_write_source_id = 0;
    }
    native_runtime_fail_pending_requests(control, "Gnoblin compositor stopped before replying");
    stop_native_policy_dbus(control);
    if (control->display)
        meta_display_set_gnoblin_shortcut_activated_handler(control->display, NULL, NULL);
    revoke_focus_contexts(control);
    revoke_text_targets(control);
    gnoblin_control_cancel_window_drags(control, "compositor_stopped");
    if (control->snap_contexts)
        g_hash_table_remove_all(control->snap_contexts);
    stop_native_shortcut_capture(control, FALSE, FALSE, NULL, NULL, NULL);
    if (control->wayland_compositor && control->session_lock_callback_id) {
        meta_wayland_session_lock_remove_state_changed_callback(control->wayland_compositor,
                                                                control->session_lock_callback_id);
        control->session_lock_callback_id = 0;
    }
    if (control->display &&
        g_object_get_data(G_OBJECT(control->display), NATIVE_CONTROL_OBJECT_DATA_KEY) == control)
        g_object_set_data(G_OBJECT(control->display), NATIVE_CONTROL_OBJECT_DATA_KEY, NULL);
    clear_display_signal_watches(control);
    if (control->service) {
        g_socket_service_stop(control->service);
        g_clear_object(&control->service);
    }
    if (control->publish_id)
        g_source_remove(control->publish_id);
    if (control->launch_tick_id)
        g_source_remove(control->launch_tick_id);
    if (control->focus_context_timeout_id) {
        g_source_remove(control->focus_context_timeout_id);
        control->focus_context_timeout_id = 0;
    }
    if (control->backend) {
        MetaCursorTracker* tracker = meta_backend_get_cursor_tracker(control->backend);
        if (tracker)
            meta_cursor_tracker_set_gnoblin_launch_cursor(tracker, FALSE);
    }
    if (control->display)
        g_signal_handlers_disconnect_by_data(control->display, control);
    if (control->dynamic_shortcuts) {
        GHashTableIter shortcut_iter;
        gpointer shortcut_value;
        g_hash_table_iter_init(&shortcut_iter, control->dynamic_shortcuts);
        while (g_hash_table_iter_next(&shortcut_iter, NULL, &shortcut_value)) {
            NativeDynamicShortcut* shortcut = shortcut_value;
            meta_display_ungrab_accelerator(control->display, shortcut->action);
        }
        g_hash_table_unref(control->dynamic_shortcuts);
        control->dynamic_shortcuts = NULL;
    }
    if (control->bare_super_shortcut) {
        gnoblin_control_dynamic_shortcut_free(control->bare_super_shortcut);
        control->bare_super_shortcut = NULL;
    }
    if (control->shortcuts) {
        for (guint i = 0; i < control->shortcuts->len; i++) {
            NativeShortcut* shortcut = g_ptr_array_index(control->shortcuts, i);
            if (!shortcut->overlay)
                meta_display_ungrab_accelerator(control->display, shortcut->action);
        }
        g_ptr_array_unref(control->shortcuts);
    }
    if (control->windows) {
        GHashTableIter window_iter;
        gpointer window;
        g_hash_table_iter_init(&window_iter, control->windows);
        while (g_hash_table_iter_next(&window_iter, &window, NULL)) {
            GArray* handler_ids = g_hash_table_lookup(control->window_signal_handler_ids, window);
            gnoblin_control_clear_object_signal_watches(G_OBJECT(window), handler_ids);
            g_signal_handlers_disconnect_by_data(window, control);
        }
        g_hash_table_unref(control->windows);
    }
    g_clear_pointer(&control->window_signal_handler_ids, g_hash_table_unref);
    if (control->workspace_manager)
        gnoblin_control_clear_workspace_manager_signal_watches(control);
    gnoblin_control_clear_all_workspace_signal_watches(control);
    clear_backend_signal_watches(control);
    gnoblin_control_clear_monitor_manager_signal_watches(control);
    clear_cursor_tracker_signal_watches(control);
    if (control->workspace_manager)
        g_signal_handlers_disconnect_by_data(control->workspace_manager, control);
    if (control->monitor_manager)
        g_signal_handlers_disconnect_by_data(control->monitor_manager, control);
    if (control->backend)
        g_signal_handlers_disconnect_by_data(control->backend, control);
    if (control->remote_access_controller)
        g_signal_handlers_disconnect_by_data(control->remote_access_controller, control);
    if (control->orientation_manager)
        g_signal_handlers_disconnect_by_data(control->orientation_manager, control);
    if (control->appearance_settings)
        g_signal_handlers_disconnect_by_data(control->appearance_settings, control);
    if (control->input_seat)
        g_signal_handlers_disconnect_by_data(control->input_seat, control);
    if (control->privacy_handles) {
        GHashTableIter privacy_iter;
        gpointer handle;
        g_hash_table_iter_init(&privacy_iter, control->privacy_handles);
        while (g_hash_table_iter_next(&privacy_iter, &handle, NULL))
            g_signal_handlers_disconnect_by_data(handle, control);
        g_hash_table_unref(control->privacy_handles);
        control->privacy_handles = NULL;
    }
    g_clear_pointer(&control->privacy_snapshot, g_variant_unref);
    g_clear_pointer(&control->session_lifecycle_snapshot, g_variant_unref);
    g_clear_pointer(&control->orientation_lock_snapshot, g_variant_unref);
    g_clear_pointer(&control->monitor_privacy_screen_snapshot, g_variant_unref);
    native_publish_runtime_snapshot(control, "privacy", NULL, control->privacy_revision);
    native_publish_runtime_snapshot(control, "monitor-privacy-screen", NULL,
                                    control->monitor_privacy_screen_revision);
    g_clear_object(&control->remote_access_controller);
    if (control->window_state)
        g_hash_table_unref(control->window_state);
    if (control->layer_state)
        g_hash_table_unref(control->layer_state);
    if (control->workspace_state)
        g_hash_table_unref(control->workspace_state);
    if (control->monitor_state)
        g_hash_table_unref(control->monitor_state);
    if (control->input_device_state)
        g_hash_table_unref(control->input_device_state);
    if (control->input_device_ids)
        g_hash_table_unref(control->input_device_ids);
    if (control->launches)
        g_ptr_array_unref(control->launches);
    if (control->focus_contexts)
        g_hash_table_unref(control->focus_contexts);
    g_clear_pointer(&control->window_input_sources, g_hash_table_unref);
    g_clear_pointer(&control->focused_input_window_id, g_free);
    if (control->menu_contexts)
        g_hash_table_unref(control->menu_contexts);
    if (control->window_drags)
        g_hash_table_unref(control->window_drags);
    if (control->snap_contexts)
        g_hash_table_unref(control->snap_contexts);
    if (control->snap_restore_frames)
        g_hash_table_unref(control->snap_restore_frames);
    if (control->text_targets)
        g_hash_table_unref(control->text_targets);
    if (control->clients) {
        GHashTableIter iter;
        gpointer value;
        g_hash_table_iter_init(&iter, control->clients);
        while (g_hash_table_iter_next(&iter, &value, NULL)) {
            Client* client = value;
            client->control = NULL;
            client_close(client);
        }
        g_hash_table_unref(control->clients);
        control->clients = NULL;
    }
    g_clear_pointer(&control->ui_sessions, g_hash_table_unref);
    struct stat current;
    if (control->path && control->inode && lstat(control->path, &current) == 0 &&
        current.st_dev == control->device && current.st_ino == control->inode)
        g_unlink(control->path);
    g_free(control->path);
    g_free(control->background_color_published);
    g_free(control->background_image_published);
    g_clear_pointer(&control->native_touchpad_gestures, g_variant_unref);
    g_clear_pointer(&control->touchpad_router, gnoblin_touchpad_router_free);
    g_clear_pointer(&control->runtime_cache, gnoblin_runtime_cache_free);
    g_clear_pointer(&control->pending_runtime_requests, g_hash_table_unref);
    g_clear_pointer(&control->pending_location_authorizations, g_hash_table_unref);
    g_clear_pointer(&control->pending_auth, g_hash_table_unref);
    g_clear_pointer(&control->pending_prompts, g_hash_table_unref);
    g_clear_pointer(&control->pending_captures, g_hash_table_unref);
    g_clear_pointer(&control->runtime_operation_ids, g_hash_table_unref);
    g_clear_pointer(&control->runtime_cancelled_operation_ids, g_hash_table_unref);
    g_clear_pointer(&control->runtime_event_subscriptions, g_hash_table_unref);
    g_clear_pointer(&control->display_signal_handler_ids, g_array_unref);
    g_clear_pointer(&control->workspace_manager_signal_handler_ids, g_array_unref);
    g_clear_pointer(&control->workspace_signal_handler_ids, g_hash_table_unref);
    g_clear_pointer(&control->backend_signal_handler_ids, g_array_unref);
    g_clear_pointer(&control->monitor_manager_signal_handler_ids, g_array_unref);
    g_clear_pointer(&control->cursor_tracker_signal_handler_ids, g_array_unref);
    g_clear_pointer(&control->runtime_reader, gnoblin_runtime_reader_free);
    g_clear_pointer(&control->runtime_writer, gnoblin_runtime_writer_free);
    if (control->pending_runtime_states) {
        g_queue_clear_full(control->pending_runtime_states, (GDestroyNotify)g_variant_unref);
        g_queue_free(control->pending_runtime_states);
        control->pending_runtime_states = NULL;
    }
    if (control->pending_runtime_events) {
        g_queue_clear_full(control->pending_runtime_events, (GDestroyNotify)g_variant_unref);
        g_queue_free(control->pending_runtime_events);
        control->pending_runtime_events = NULL;
    }
    native_input_cancel_pending(control, FALSE);
    if (control->pending_input_events) {
        g_queue_free(control->pending_input_events);
        control->pending_input_events = NULL;
    }
    g_clear_pointer(&control->held_input_dispositions, g_hash_table_unref);
    g_clear_pointer(&control->accepted_input_requests, g_hash_table_unref);
    g_clear_pointer(&control->input_policy_error, g_free);
    control->path = NULL;
    control->teardown_complete = TRUE;
    native_control_maybe_free_stopped(control);
}
