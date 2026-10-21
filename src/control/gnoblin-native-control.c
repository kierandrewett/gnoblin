#include "gnoblin-native-control-private.h"

/* Initial native snapshot is available before MetaContext creates the display,
 * so Mutter preferences/workspace setup can read it without starting Lua. */
GnoblinRuntimeCache* bootstrap_runtime_cache;

guint64 bootstrap_runtime_generation;

void native_runtime_abort(GnoblinNativeControl* control) {
    if (!control)
        return;
    control->stopping = TRUE;
    gnoblin_control_clear_pending_location_authorizations(control);
    gnoblin_control_corner_toolkit_cache_shutdown(control);
    if (control->runtime_read_source_id) {
        g_source_remove(control->runtime_read_source_id);
        control->runtime_read_source_id = 0;
    }
    if (control->runtime_write_source_id) {
        g_source_remove(control->runtime_write_source_id);
        control->runtime_write_source_id = 0;
    }
    native_runtime_fail_pending_requests(control, "Lua runtime stopped before replying");
    if (control->active_shortcut_session)
        dynamic_shortcut_end_session(control, control->active_shortcut_session, "runtime_stopped");
    gnoblin_control_clear_runtime_dynamic_shortcuts(control, "runtime_stopped");
    g_clear_pointer(&control->window_shader_files, g_hash_table_unref);
    stop_native_shortcut_capture(control, FALSE, FALSE, NULL, NULL, NULL);
    gnoblin_control_cancel_window_drags(control, "runtime_stopped");
    revoke_text_targets(control);
    revoke_focus_contexts(control);
    revoke_menu_contexts(control);
    if (control->snap_contexts)
        g_hash_table_remove_all(control->snap_contexts);
}

/* The supervised compositor may consume configuration only from the immutable
 * snapshot transferred by its Lua-owning parent. Keep legacy reads confined to
 * the unsupervised compatibility path. */
GVariant* native_config_document(GnoblinNativeControl* control) {
    if (control && control->supervised_runtime)
        return gnoblin_runtime_cache_get_document(control->runtime_cache);
    if (!control && bootstrap_runtime_cache)
        return gnoblin_runtime_cache_get_document(bootstrap_runtime_cache);
    return NULL;
}

static GnoblinLocationAccuracy native_location_accuracy(const char* nick) {
    if (g_strcmp0(nick, "country") == 0)
        return GNOBLIN_LOCATION_ACCURACY_COUNTRY;
    if (g_strcmp0(nick, "city") == 0)
        return GNOBLIN_LOCATION_ACCURACY_CITY;
    if (g_strcmp0(nick, "neighborhood") == 0)
        return GNOBLIN_LOCATION_ACCURACY_NEIGHBORHOOD;
    if (g_strcmp0(nick, "street") == 0)
        return GNOBLIN_LOCATION_ACCURACY_STREET;
    if (g_strcmp0(nick, "exact") == 0)
        return GNOBLIN_LOCATION_ACCURACY_EXACT;
    return GNOBLIN_LOCATION_ACCURACY_INHERIT;
}

void native_apply_location_policy(GnoblinNativeControl* control, GVariant* config) {
    if (!control || !control->location_agent)
        return;
    g_autoptr(GVariant) location =
        config ? g_variant_lookup_value(config, "location", G_VARIANT_TYPE_VARDICT) : NULL;
    g_autoptr(GVariant) enabled_value =
        location ? g_variant_lookup_value(location, "enabled", NULL) : NULL;
    g_autoptr(GVariant) accuracy_value =
        location ? g_variant_lookup_value(location, "max-accuracy", NULL) : NULL;
    gboolean enabled_set =
        enabled_value && g_variant_is_of_type(enabled_value, G_VARIANT_TYPE_BOOLEAN);
    gboolean enabled = enabled_set && g_variant_get_boolean(enabled_value);
    gboolean accuracy_set =
        accuracy_value && g_variant_is_of_type(accuracy_value, G_VARIANT_TYPE_STRING);
    GnoblinLocationAccuracy accuracy =
        accuracy_set ? native_location_accuracy(g_variant_get_string(accuracy_value, NULL))
                     : GNOBLIN_LOCATION_ACCURACY_INHERIT;
    if (accuracy == GNOBLIN_LOCATION_ACCURACY_INHERIT)
        accuracy_set = FALSE;
    gnoblin_location_agent_set_policy(control->location_agent, enabled_set, enabled, accuracy_set,
                                      accuracy);
}

guint64 native_config_revision(GnoblinNativeControl* control) {
    if (control && control->supervised_runtime)
        return gnoblin_runtime_cache_get_settings_revision(control->runtime_cache);
    if (!control && bootstrap_runtime_cache)
        return gnoblin_runtime_cache_get_settings_revision(bootstrap_runtime_cache);
    return 0;
}

GVariant* gnoblin_native_control_get_config_document(MetaDisplay* display) {
    GnoblinNativeControl* control =
        display ? g_object_get_data(G_OBJECT(display), NATIVE_CONTROL_OBJECT_DATA_KEY) : NULL;
    return native_config_document(control);
}

guint64 gnoblin_native_control_get_config_revision(MetaDisplay* display) {
    GnoblinNativeControl* control =
        display ? g_object_get_data(G_OBJECT(display), NATIVE_CONTROL_OBJECT_DATA_KEY) : NULL;
    return native_config_revision(control);
}

gboolean gnoblin_native_control_get_config_bool(MetaDisplay* display, const char* section,
                                                const char* key, gboolean fallback) {
    GnoblinNativeControl* control =
        display ? g_object_get_data(G_OBJECT(display), NATIVE_CONTROL_OBJECT_DATA_KEY) : NULL;
    GnoblinRuntimeCache* cache =
        control && control->supervised_runtime ? control->runtime_cache : bootstrap_runtime_cache;
    return gnoblin_runtime_cache_get_bool(cache, section, key, fallback);
}

gboolean gnoblin_native_control_protocol_enabled(const char* protocol) {
    g_return_val_if_fail(protocol != NULL && *protocol != '\0', FALSE);
    if (!gnoblin_native_control_is_session(NULL))
        return FALSE;
    return gnoblin_native_control_get_config_bool(NULL, "protocols", protocol, TRUE);
}

guint64 native_config_generation(GnoblinNativeControl* control) {
    return native_config_revision(control);
}

void native_publish_runtime_snapshot(GnoblinNativeControl* control, const char* name,
                                     GVariant* snapshot, guint64 revision) {
    if (!control || control->stopping || control->runtime_worker_suspended)
        return;
    if (!control->supervised_runtime) {
        g_warning("gnoblin-native-control: refusing state snapshot without the Lua supervisor");
        return;
    }
    GVariantBuilder builder;
    g_variant_builder_init(&builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&builder, "{sv}", "state_version", g_variant_new_uint32(1));
    g_variant_builder_add(&builder, "{sv}", "name", g_variant_new_string(name));
    g_variant_builder_add(&builder, "{sv}", "revision", g_variant_new_uint64(revision));
    g_variant_builder_add(&builder, "{sv}", "available", g_variant_new_boolean(snapshot != NULL));
    if (snapshot)
        g_variant_builder_add(&builder, "{sv}", "snapshot", snapshot);
    g_autoptr(GVariant) payload = g_variant_ref_sink(g_variant_builder_end(&builder));
    if (!control->runtime_hello_sent) {
        g_queue_push_tail(control->pending_runtime_states, g_variant_ref(payload));
        return;
    }
    g_autoptr(GError) error = NULL;
    if (!native_runtime_send(control, GNOBLIN_RUNTIME_PACKET_STATE, 0, payload, &error)) {
        native_runtime_abort(control);
        g_warning("gnoblin-native-control: cannot send state snapshot to Lua supervisor: %s",
                  error ? error->message : "unknown error");
    }
}

static Client* native_client_by_id(GnoblinNativeControl* control, guint64 client_id) {
    if (!control || !control->clients || !client_id)
        return NULL;
    GHashTableIter iter;
    gpointer key;
    g_hash_table_iter_init(&iter, control->clients);
    while (g_hash_table_iter_next(&iter, &key, NULL)) {
        Client* client = key;
        if (client->client_id == client_id && !client->closing)
            return client;
    }
    return NULL;
}

MetaWindow* native_window_by_stable_id(GnoblinNativeControl* control, const char* stable_id) {
    if (!control || !stable_id || !*stable_id)
        return NULL;
    GSList* windows = meta_display_list_windows(control->display, META_LIST_DEFAULT);
    MetaWindow* match = NULL;
    for (GSList* link = windows; link; link = link->next) {
        MetaWindow* window = link->data;
        g_autofree char* id = g_strdup_printf("%u", meta_window_get_stable_sequence(window));
        if (g_str_equal(id, stable_id)) {
            match = window;
            break;
        }
    }
    g_slist_free(windows);
    return match;
}

GVariant* gnoblin_native_control_restore_or_minimize_window(MetaDisplay* display,
                                                            GVariant* arguments, GError** error) {
    GnoblinNativeControl* control =
        display ? g_object_get_data(G_OBJECT(display), NATIVE_CONTROL_OBJECT_DATA_KEY) : NULL;
    const char* id = NULL;
    if (!control || control->stopping || !control->snap_restore_frames ||
        !g_variant_is_of_type(arguments, G_VARIANT_TYPE_VARDICT) ||
        g_variant_n_children(arguments) != 1 || !g_variant_lookup(arguments, "id", "&s", &id) ||
        !id || !*id) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "window.restore_or_minimize requires only a stable window id");
        return NULL;
    }
    if (control->wayland_compositor &&
        meta_wayland_session_lock_is_active(control->wayland_compositor)) {
        g_set_error_literal(
            error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
            "window.restore_or_minimize is unavailable while the session is locked");
        return NULL;
    }
    MetaWindow* window = native_window_by_stable_id(control, id);
    if (!window || meta_window_is_skip_taskbar(window) ||
        meta_window_is_override_redirect(window)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_FOUND,
                            "window.restore_or_minimize target is no longer available");
        return NULL;
    }
    const char* action = NULL;
    if (meta_window_get_maximize_flags(window) != 0) {
        meta_window_set_unmaximize_flags(window, META_MAXIMIZE_BOTH);
        g_hash_table_remove(control->snap_restore_frames, window);
        action = "unmaximize";
    } else {
        MtkRectangle* frame = g_hash_table_lookup(control->snap_restore_frames, window);
        if (frame) {
            MtkRectangle restore_frame = *frame;
            g_hash_table_remove(control->snap_restore_frames, window);
            meta_window_move_resize_frame(window, TRUE, restore_frame.x, restore_frame.y,
                                          restore_frame.width, restore_frame.height);
            action = "restore";
        } else if (meta_window_can_minimize(window)) {
            meta_window_minimize(window);
            action = "minimize";
        } else {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_SUPPORTED,
                                "window cannot be minimized and has no saved snap frame");
            return NULL;
        }
    }
    GVariantBuilder result;
    g_variant_builder_init(&result, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&result, "{sv}", "id", g_variant_new_string(id));
    g_variant_builder_add(&result, "{sv}", "action", g_variant_new_string(action));
    return g_variant_ref_sink(g_variant_builder_end(&result));
}

static void pending_thumbnail_free(PendingThumbnail* pending) {
    if (!pending)
        return;
    g_free(pending->window_id);
    g_free(pending);
}

static gboolean native_thumbnail_idle(gpointer user_data) {
    PendingThumbnail* pending = user_data;
    GnoblinNativeControl* control = pending->control;
    pending->source_id = 0;
    Client* owner = native_client_by_id(control, pending->client_id);
    if (owner && owner->pending_thumbnails)
        owner->pending_thumbnails--;
    if (control->pending_thumbnail_count)
        control->pending_thumbnail_count--;

    g_autoptr(GError) error = NULL;
    g_autoptr(GBytes) png = NULL;
    g_autoptr(JsonNode) result = NULL;
    int actual_width = 0;
    int actual_height = 0;
    if (control->stopping || (pending->client_id && !owner)) {
        g_set_error_literal(&error, G_IO_ERROR, G_IO_ERROR_CANCELLED,
                            "thumbnail requester disconnected");
    } else if (control->wayland_compositor &&
               meta_wayland_session_lock_is_active(control->wayland_compositor)) {
        g_set_error_literal(&error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "window thumbnails are unavailable while the session is locked");
    } else {
        MetaWindow* window = native_window_by_stable_id(control, pending->window_id);
        if (!window) {
            g_set_error_literal(&error, G_IO_ERROR, G_IO_ERROR_NOT_FOUND,
                                "thumbnail window no longer exists");
        } else {
            MetaWindowActor* actor = meta_window_actor_from_window(window);
            if (!actor) {
                g_set_error_literal(&error, G_IO_ERROR, G_IO_ERROR_NOT_FOUND,
                                    "thumbnail window has no compositor actor");
            } else {
                png = meta_window_actor_paint_to_png(actor, pending->width, pending->height,
                                                     &actual_width, &actual_height, &error);
                /* The window must still resolve to the same stable sequence after capture. */
                if (png && !native_window_by_stable_id(control, pending->window_id)) {
                    g_clear_pointer(&png, g_bytes_unref);
                    g_set_error_literal(&error, G_IO_ERROR, G_IO_ERROR_NOT_FOUND,
                                        "thumbnail window closed during capture");
                }
            }
        }
    }

    if (png && !(control->wayland_compositor &&
                 meta_wayland_session_lock_is_active(control->wayland_compositor))) {
        gsize length = 0;
        const guchar* bytes = g_bytes_get_data(png, &length);
        if (length > 512 * 1024 || actual_width < 1 || actual_width > 480 || actual_height < 1 ||
            actual_height > 320) {
            g_set_error_literal(&error, G_IO_ERROR, G_IO_ERROR_NO_SPACE,
                                "thumbnail output exceeded the requested bounds");
        } else {
            g_autofree char* encoded = g_base64_encode(bytes, length);
            JsonObject* object = json_object_new();
            json_object_set_string_member(object, "window_id", pending->window_id);
            json_object_set_int_member(object, "width", actual_width);
            json_object_set_int_member(object, "height", actual_height);
            json_object_set_string_member(object, "data", encoded);
            result = json_node_new(JSON_NODE_OBJECT);
            json_node_take_object(result, object);
        }
    } else if (png) {
        g_clear_pointer(&png, g_bytes_unref);
        g_set_error_literal(&error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "window thumbnails are unavailable while the session is locked");
    }

    if (!control->stopping)
        gnoblin_control_dispatch_operation_completion_full(
            control, pending->operation_id, "window.thumbnail", result != NULL, result,
            gnoblin_control_operation_error_code(error), error ? error->message : NULL, FALSE);
    pending_thumbnail_free(pending);
    native_control_maybe_free_stopped(control);
    return G_SOURCE_REMOVE;
}

gboolean native_runtime_begin_thumbnail(GnoblinNativeControl* control, gint64 operation_id,
                                        GVariant* arguments, guint64 client_id, GError** error) {
    const char* window_id = NULL;
    gint64 width = 0;
    gint64 height = 0;
    if (!g_variant_lookup(arguments, "id", "&s", &window_id) || !window_id || !*window_id ||
        !g_variant_lookup(arguments, "width", "x", &width) ||
        !g_variant_lookup(arguments, "height", "x", &height) || width < 1 || width > 480 ||
        height < 1 || height > 320) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "window.thumbnail requires an id and dimensions up to 480x320");
        return FALSE;
    }
    Client* owner = native_client_by_id(control, client_id);
    if (client_id && !owner) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_CANCELLED,
                            "thumbnail requester disconnected");
        return FALSE;
    }
    if ((owner && owner->pending_thumbnails >= MAX_PENDING_THUMBNAILS_PER_CLIENT) ||
        control->pending_thumbnail_count >= MAX_PENDING_THUMBNAILS) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_BUSY, "thumbnail capture limit reached");
        return FALSE;
    }
    if (control->wayland_compositor &&
        meta_wayland_session_lock_is_active(control->wayland_compositor)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "window thumbnails are unavailable while the session is locked");
        return FALSE;
    }
    if (!native_window_by_stable_id(control, window_id)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_FOUND,
                            "thumbnail window no longer exists");
        return FALSE;
    }

    if (owner)
        owner->pending_thumbnails++;
    control->pending_thumbnail_count++;
    guint64* operation_key = g_new(guint64, 1);
    *operation_key = (guint64)operation_id;
    g_hash_table_add(control->runtime_operation_ids, operation_key);

    PendingThumbnail* pending = g_new0(PendingThumbnail, 1);
    pending->control = control;
    pending->client_id = client_id;
    pending->operation_id = operation_id;
    pending->window_id = g_strdup(window_id);
    pending->width = (guint)width;
    pending->height = (guint)height;
    pending->source_id = g_idle_add_full(G_PRIORITY_LOW, native_thumbnail_idle, pending, NULL);
    return TRUE;
}

const NativeCapability native_capabilities[] = {
    {"layer-list", "List layer-shell surfaces and their placement."},
    {"layer-lifecycle-events", "Receive layer-surface creation, state-change, and removal events."},
    {"monitor-list", "List active logical monitors and their geometry."},
    {"window-list", "Read managed windows and their compositor state."},
    {"window-actions", "Request supported window actions."},
    {"typed-window-operations", "Set window state, geometry, workspace, or monitor."},
    {"window-interactive-grabs",
     "Start a keyboard move or resize with a trusted shortcut context."},
    {"window-thumbnails", "Capture bounded compositor-rendered window previews."},
    {"window-drag-snap-offers",
     "Receive pointer-drag state and submit capability-bound snap regions."},
    {"session-activity", "Read session idle/active state and receive activity changes."},
    {"workspace-management", "List and manage native workspaces."},
    {"window-change-events", "Receive changes to managed window properties."},
    {"window-lifecycle-events", "Receive window creation, focus, attention, and close events."},
    {"shell-request-events", "Receive window-menu and compositor OSD requests for shell UI."},
    {"workspace-lifecycle-events", "Receive workspace creation and state-change events."},
    {"monitor-lifecycle-events", "Receive active monitor addition, change, and removal events."},
    {"input-device-list", "Read detected input devices and their reported capabilities."},
    {"input-device-lifecycle-events", "Receive input-device addition and removal events."},
    {"input-source-list", "List configured XKB input sources and read the confirmed source."},
    {"input-source-selection", "Select configured XKB input sources."},
    {"input-source-lifecycle-events", "Receive changes to the active XKB input source."},
    {"launch-feedback", "Track launch feedback against newly mapped or newly focused windows."},
    {"shortcut-capture", "Capture one normalized keyboard accelerator."},
    {"generic-event-subscriptions", "Subscribe a socket connection to selected native events."},
    {"input-gesture-events", "Receive stable touchpad gesture events."},
    {"shortcut-focus-contexts",
     "Receive a one-use focus grant for a trusted configured command shortcut."},
    {"dynamic-shortcuts", "Register connection-owned momentary global shortcut bindings."},
    {"animation-preview", "Inspect configured animations and control compositor previews."},
    {"microphone-monitor", "Monitor microphone activity through PipeWire."},
    {"camera-monitor", "Monitor camera activity through PipeWire."},
    {"location-agent", "Monitor GeoClue activity and broker location authorization to Lua."},
    {"auth-agent", "Act as the polkit agent and broker password prompts to Lua."},
    {"prompt-broker", "Serve keyring and GPG passphrase prompts and broker them to Lua."},
    {"ui-sessions", "Exchange bounded state and commands between shell UI processes."},
};

const gsize native_capabilities_count = G_N_ELEMENTS(native_capabilities);

const char* native_socket_events[] = {
    "windows",
    "gnoblin.layer.created",
    "gnoblin.layer.changed",
    "gnoblin.layer.removed",
    "gnoblin.window.created",
    "gnoblin.window.changed",
    "gnoblin.window.focused",
    "gnoblin.window.unfocused",
    "gnoblin.window.attention-changed",
    "gnoblin.window.activation-denied",
    "gnoblin.window.closed",
    "gnoblin.window.drag.started",
    "gnoblin.window.drag.updated",
    "gnoblin.window.drag.ended",
    "gnoblin.window.menu-requested",
    "gnoblin.osd.requested",
    "gnoblin.pointer.locate-requested",
    "gnoblin.input.pad-help-requested",
    "gnoblin.focus.policy-changed",
    "gnoblin.config.reloaded",
    "gnoblin.config.reload-failed",
    "gnoblin.capability.changed",
    "gnoblin.appearance.color-scheme-changed",
    "gnoblin.privacy.changed",
    "gnoblin.location.authorization-requested",
    "gnoblin.auth.requested",
    "gnoblin.auth.prompt",
    "gnoblin.auth.message",
    "gnoblin.auth.finished",
    "gnoblin.prompt.requested",
    "gnoblin.prompt.finished",
    "gnoblin.animation.started",
    "gnoblin.animation.finished",
    "gnoblin.workspace.created",
    "gnoblin.workspace.renamed",
    "gnoblin.workspace.changed",
    "gnoblin.workspace.removed",
    "gnoblin.workspace.activated",
    "gnoblin.workspace.window-moved",
    "monitors",
    "gnoblin.monitor.added",
    "gnoblin.monitor.changed",
    "gnoblin.monitor.removed",
    "gnoblin.input.device-added",
    "gnoblin.input.device-removed",
    "gnoblin.input.sources-changed",
    "gnoblin.input.source-changed",
    "gnoblin.input.orientation-lock-changed",
    "gnoblin.monitor.privacy-screen-changed",
    "gnoblin.input.gesture",
    "gnoblin.launch.changed",
    "gnoblin.session.lock-requested",
    "gnoblin.session.lock-state-changed",
    "gnoblin.session.activity-changed",
    "gnoblin.session.state-changed",
    "gnoblin.runtime.status-changed",
    "gnoblin.shortcut.activated",
    "gnoblin.shortcut.binding-activated",
    "gnoblin.shortcut.binding-deactivated",
    "gnoblin.shortcut.session.activated",
    "gnoblin.shortcut.session.key",
    "gnoblin.shortcut.session.ended",
    "gnoblin.api.operation-completed",
    "gnoblin.operation.completed",
    NULL,
};

gboolean start_native_session_bus(GnoblinNativeControl* control) {
    g_autoptr(GError) error = NULL;
    control->session_bus = g_bus_get_sync(G_BUS_TYPE_SESSION, NULL, &error);
    if (!control->session_bus) {
        g_warning("gnoblin-native-control: session D-Bus unavailable: %s", error->message);
        return FALSE;
    }
    gnoblin_control_activity_watch(control);
    return TRUE;
}

void stop_native_session_bus(GnoblinNativeControl* control) {
    if (control->ibus_retry_source_id) {
        g_source_remove(control->ibus_retry_source_id);
        control->ibus_retry_source_id = 0;
    }
    if (control->ibus_bus && control->ibus_signal_subscription_id) {
        g_dbus_connection_signal_unsubscribe(control->ibus_bus,
                                             control->ibus_signal_subscription_id);
        control->ibus_signal_subscription_id = 0;
    }
    if (control->ibus_bus && control->ibus_owner_subscription_id) {
        g_dbus_connection_signal_unsubscribe(control->ibus_bus,
                                             control->ibus_owner_subscription_id);
        control->ibus_owner_subscription_id = 0;
    }
    if (control->ibus_bus)
        g_signal_handlers_disconnect_by_data(control->ibus_bus, control);
    g_clear_object(&control->ibus_bus);
    if (!control->session_bus)
        return;
    if (control->session_activity_subscription_id) {
        g_dbus_connection_signal_unsubscribe(control->session_bus,
                                             control->session_activity_subscription_id);
        control->session_activity_subscription_id = 0;
    }
    if (control->session_activity_owner_subscription_id) {
        g_dbus_connection_signal_unsubscribe(control->session_bus,
                                             control->session_activity_owner_subscription_id);
        control->session_activity_owner_subscription_id = 0;
    }
    g_clear_object(&control->session_bus);
    g_clear_pointer(&control->session_activity_snapshot, g_variant_unref);
}

void native_shortcut_free(gpointer data) {
    NativeShortcut* shortcut = data;
    g_free(shortcut->name);
    g_free(shortcut->binding);
    g_strfreev(shortcut->bindings);
    g_strfreev(shortcut->argv);
    g_free(shortcut->action_id);
    g_free(shortcut);
}

GVariant* native_shortcut_snapshot(GnoblinNativeControl* control) {
    GVariantBuilder records;
    g_variant_builder_init(&records, G_VARIANT_TYPE("av"));
    for (guint i = 0; control->shortcuts && i < control->shortcuts->len; i++) {
        NativeShortcut* shortcut = g_ptr_array_index(control->shortcuts, i);
        GVariantBuilder record;
        g_variant_builder_init(&record, G_VARIANT_TYPE_VARDICT);
        g_variant_builder_add(&record, "{sv}", "name", g_variant_new_string(shortcut->name));
        if (shortcut->binding_count > 1)
            g_variant_builder_add(&record, "{sv}", "binding",
                                  g_variant_new_strv((const char* const*)shortcut->bindings, -1));
        else if (shortcut->binding)
            g_variant_builder_add(&record, "{sv}", "binding",
                                  g_variant_new_string(shortcut->binding));
        else
            g_variant_builder_add(&record, "{sv}", "binding",
                                  g_variant_new_strv((const char* const*)shortcut->bindings, -1));
        g_variant_builder_add(&record, "{sv}", "enabled", g_variant_new_boolean(shortcut->enabled));
        g_variant_builder_add(&record, "{sv}", "trigger",
                              g_variant_new_string(shortcut->release ? "release" : "press"));
        if (shortcut->action_id) {
            g_variant_builder_add(&record, "{sv}", "action",
                                  g_variant_new_string(shortcut->action_id));
        } else {
            g_variant_builder_add(&record, "{sv}", "command",
                                  g_variant_new_strv((const char* const*)shortcut->argv, -1));
        }
        g_variant_builder_add(&record, "{sv}", "revision",
                              g_variant_new_int64(control->state_revision));
        g_variant_builder_add_value(&records,
                                    g_variant_new_variant(g_variant_builder_end(&record)));
    }
    GVariantBuilder snapshot;
    g_variant_builder_init(&snapshot, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&snapshot, "{sv}", "shortcuts", g_variant_builder_end(&records));
    g_variant_builder_add(&snapshot, "{sv}", "revision",
                          g_variant_new_int64(control->state_revision));
    return g_variant_ref_sink(g_variant_builder_end(&snapshot));
}

void native_window_state_free(gpointer data) {
    NativeWindowState* state = data;
    g_free(state->json);
    g_free(state->comparable_json);
    g_free(state);
}

void native_snap_context_free(gpointer data) {
    NativeSnapContext* context = data;
    g_free(context->window_id);
    g_free(context->monitor_id);
    g_free(context);
}

char** native_command_argv(GVariant* command) {
    if (!command || !g_variant_is_of_type(command, G_VARIANT_TYPE("av")) ||
        g_variant_n_children(command) == 0)
        return NULL;
    char** argv = g_new0(char*, g_variant_n_children(command) + 1);
    for (gsize item = 0; item < g_variant_n_children(command); item++) {
        g_autoptr(GVariant) wrapped = g_variant_get_child_value(command, item);
        g_autoptr(GVariant) argument = g_variant_get_variant(wrapped);
        if (!g_variant_is_of_type(argument, G_VARIANT_TYPE_STRING)) {
            g_strfreev(argv);
            return NULL;
        }
        argv[item] = g_variant_dup_string(argument, NULL);
    }
    if (!*argv[0]) {
        g_strfreev(argv);
        return NULL;
    }
    return argv;
}

void native_shortcut_activated(MetaDisplay* display, guint action, gpointer device, guint timestamp,
                               gpointer user_data) {
    GnoblinNativeControl* control = user_data;
    for (guint i = 0; i < control->shortcuts->len; i++) {
        NativeShortcut* shortcut = g_ptr_array_index(control->shortcuts, i);
        if (!shortcut->overlay && shortcut->action == action && !shortcut->release) {
            /* Clutter event milliseconds share the monotonic clock and
             * wrap at 32 bits. Unsigned subtraction preserves that wrap. */
            guint32 age_ms = (guint32)(g_get_monotonic_time() / 1000) - timestamp;
            if (age_ms < 10000)
                g_message("gnoblin-shortcut: %s input-to-dispatch %u ms", shortcut->name, age_ms);
            gnoblin_control_launch_command("shortcut", shortcut->name, shortcut->argv);
        }
    }
}

void capture_focus_identity(GnoblinNativeControl* control, NativeFocusContext* context,
                            const ClutterEvent* event) {
    context->modifiers = event ? clutter_event_get_state(event) : 0;
    ClutterKeymap* keymap =
        control && control->input_seat ? clutter_seat_get_keymap(control->input_seat) : NULL;
    if (keymap) {
        xkb_mod_mask_t depressed = 0, latched = 0, locked = 0;
        clutter_keymap_get_modifier_state(keymap, &depressed, &latched, &locked);
        context->allowed_modifier_mask = depressed | latched | locked;
    }
    MetaWaylandSeat* seat =
        control && control->wayland_compositor ? control->wayland_compositor->seat : NULL;
    if (!seat)
        return;
    meta_wayland_seat_get_gnoblin_focus_identity(seat, &context->surface_id, &context->client_id,
                                                 &context->focus_epoch);
}
