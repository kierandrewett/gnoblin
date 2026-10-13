#include "gnoblin-native-control-private.h"
#include "core/stack.h"

const char* gnoblin_control_operation_error_code(const GError* error) {
    if (!error)
        return "internal";
    if (error->domain == G_DBUS_ERROR) {
        switch (error->code) {
        case G_DBUS_ERROR_SERVICE_UNKNOWN:
        case G_DBUS_ERROR_NAME_HAS_NO_OWNER:
            return "unavailable";
        case G_DBUS_ERROR_UNKNOWN_METHOD:
        case G_DBUS_ERROR_NOT_SUPPORTED:
            return "unsupported";
        case G_DBUS_ERROR_TIMEOUT:
            return "timed_out";
        case G_DBUS_ERROR_FAILED:
            return "not_found";
        default:
            return "internal";
        }
    }
    if (error->domain != G_IO_ERROR)
        return "internal";
    switch (error->code) {
    case G_IO_ERROR_INVALID_ARGUMENT:
        return "invalid_argument";
    case G_IO_ERROR_NOT_FOUND:
        return "not_found";
    case G_IO_ERROR_EXISTS:
        return "stale";
    case G_IO_ERROR_NOT_SUPPORTED:
        return "unsupported";
    case G_IO_ERROR_BUSY:
        return "busy";
    case G_IO_ERROR_PERMISSION_DENIED:
        return "denied";
    case G_IO_ERROR_CANCELLED:
        return "cancelled";
    case G_IO_ERROR_TIMED_OUT:
        return "timed_out";
    case G_IO_ERROR_NOT_CONNECTED:
    case G_IO_ERROR_CLOSED:
        return "unavailable";
    default:
        return "internal";
    }
}

void gnoblin_control_dispatch_operation_completion_full(GnoblinNativeControl* control,
                                                        gint64 request_id, const char* method,
                                                        gboolean ok, JsonNode* result,
                                                        const char* error_code, const char* message,
                                                        gboolean dispatch_lua) {
    if (request_id <= 0)
        return;

    gboolean cancelled_by_restart =
        control->runtime_cancelled_operation_ids &&
        g_hash_table_remove(control->runtime_cancelled_operation_ids, &request_id);
    if (control->runtime_worker_suspended || cancelled_by_restart) {
        ok = FALSE;
        result = NULL;
        error_code = "unavailable";
        message = "Lua worker restarted before the operation completed";
        dispatch_lua = FALSE;
    }

    const char* failure = message && *message ? message : "Operation failed";
    g_autoptr(JsonNode) root = json_node_new(JSON_NODE_OBJECT);
    JsonObject* object = json_object_new();
    json_node_take_object(root, object);
    json_object_set_string_member(object, "name", "gnoblin.operation.completed");
    json_object_set_int_member(object, "operation_id", request_id);
    json_object_set_string_member(object, "method", method);
    json_object_set_boolean_member(object, "ok", ok);
    if (ok && result) {
        json_object_set_member(object, "value", json_node_copy(result));
    } else if (!ok) {
        JsonObject* error = json_object_new();
        json_object_set_string_member(error, "code",
                                      error_code && *error_code ? error_code : "internal");
        json_object_set_string_member(error, "message", failure);
        JsonNode* error_node = json_node_new(JSON_NODE_OBJECT);
        json_node_take_object(error_node, error);
        json_object_set_member(object, "error", error_node);
    }
    json_object_set_int_member(object, "revision", control->state_revision);
    json_object_set_int_member(object, "sequence", ++control->event_sequence);
    json_object_set_int_member(object, "time", g_get_monotonic_time());
    publish_native_socket_event(control, root);
    (void)dispatch_lua;

    if (control->runtime_operation_ids &&
        g_hash_table_remove(control->runtime_operation_ids, &request_id)) {
        GVariantBuilder completion;
        g_variant_builder_init(&completion, G_VARIANT_TYPE_VARDICT);
        g_variant_builder_add(&completion, "{sv}", "operation_id", g_variant_new_int64(request_id));
        g_variant_builder_add(&completion, "{sv}", "method", g_variant_new_string(method));
        g_variant_builder_add(&completion, "{sv}", "ok", g_variant_new_boolean(ok));
        if (ok && result) {
            g_autoptr(GVariant) value = variant_from_json(result);
            if (value)
                g_variant_builder_add(&completion, "{sv}", "value", value);
        } else if (!ok) {
            GVariantBuilder error_record;
            g_variant_builder_init(&error_record, G_VARIANT_TYPE_VARDICT);
            g_variant_builder_add(
                &error_record, "{sv}", "code",
                g_variant_new_string(error_code && *error_code ? error_code : "internal"));
            g_variant_builder_add(&error_record, "{sv}", "message", g_variant_new_string(failure));
            g_variant_builder_add(&completion, "{sv}", "error",
                                  g_variant_builder_end(&error_record));
        }
        g_autoptr(GVariant) payload = g_variant_ref_sink(g_variant_builder_end(&completion));
        g_autoptr(GError) completion_error = NULL;
        if (!native_runtime_send(control, GNOBLIN_RUNTIME_PACKET_COMPLETION, (guint64)request_id,
                                 payload, &completion_error)) {
            native_runtime_abort(control);
            g_warning("gnoblin-native-control: cannot return async operation completion: %s",
                      completion_error ? completion_error->message : "unknown error");
        }
    }

    /* Keep the original socket and Lua event payload during client migration. */
    g_autoptr(JsonNode) legacy = json_node_new(JSON_NODE_OBJECT);
    JsonObject* legacy_object = json_object_new();
    json_node_take_object(legacy, legacy_object);
    json_object_set_string_member(legacy_object, "name", "gnoblin.api.operation-completed");
    json_object_set_int_member(legacy_object, "request_id", request_id);
    json_object_set_string_member(legacy_object, "method", method);
    json_object_set_boolean_member(legacy_object, "ok", ok);
    if (ok && result)
        json_object_set_member(legacy_object, "result", json_node_copy(result));
    else if (!ok)
        json_object_set_string_member(legacy_object, "error", failure);
    json_object_set_int_member(legacy_object, "revision", control->state_revision);
    json_object_set_int_member(legacy_object, "sequence", ++control->event_sequence);
    json_object_set_int_member(legacy_object, "time", g_get_monotonic_time());
    publish_native_socket_event(control, legacy);
    /* Public socket subscribers receive the legacy completion name above.
     * Lua completion callbacks are owned by the supervisor process. */
}

static void dispatch_operation_completion(GnoblinNativeControl* control, gint64 request_id,
                                          const char* method, gboolean ok, JsonNode* result,
                                          const char* error_code, const char* message) {
    gnoblin_control_dispatch_operation_completion_full(control, request_id, method, ok, result,
                                                       error_code, message, TRUE);
}

void native_control_maybe_free_stopped(GnoblinNativeControl* control) {
    /* Runtime abort can precede compositor teardown while async callbacks drain. */
    if (!control || !control->stopping || !control->teardown_complete ||
        control->pending_input_source_ops != 0 || control->pending_ibus_queries != 0 ||
        control->pending_portal_grant_ops != 0 || control->pending_thumbnail_count != 0 ||
        control->pending_activity_queries != 0)
        return;
    gnoblin_control_input_sources_release(control);
    g_clear_pointer(&control->session_activity_snapshot, g_variant_unref);
    g_free(control);
}

static void clear_pending_grant_delivery(GnoblinNativeControl* control, gint64 request_id) {
    if (!control || request_id <= 0 || !control->clients)
        return;
    g_autofree char* key = g_strdup_printf("%" G_GINT64_FORMAT, request_id);
    GHashTableIter iter;
    gpointer client_value;
    g_hash_table_iter_init(&iter, control->clients);
    while (g_hash_table_iter_next(&iter, &client_value, NULL)) {
        Client* client = client_value;
        if (client->pending_grant_operations)
            g_hash_table_remove(client->pending_grant_operations, key);
    }
}

/* Exported names for static helpers that gnoblin-control-grants.c calls. */
void gnoblin_control_maybe_free_stopped(GnoblinNativeControl* control) {
    native_control_maybe_free_stopped(control);
}

guint64 gnoblin_control_config_generation(GnoblinNativeControl* control) {
    return native_config_generation(control);
}

guint64 gnoblin_control_config_revision(GnoblinNativeControl* control) {
    return native_config_revision(control);
}

void gnoblin_control_send_response(Client* client, char* response) {
    send_response(client, response);
}

gboolean gnoblin_control_focus_token_random(char token[65]) {
    return focus_token_random(token);
}

void gnoblin_control_clear_pending_grant_delivery(GnoblinNativeControl* control,
                                                  gint64 request_id) {
    clear_pending_grant_delivery(control, request_id);
}

void stop_native_shortcut_capture(GnoblinNativeControl* control, gboolean complete, gboolean ok,
                                  const char* accelerator, const char* error_code,
                                  const char* message) {
    if (!control || !control->shortcut_capture_active)
        return;

    control->shortcut_capture_active = FALSE;
    control->shortcut_capture_super_pressed = FALSE;
    if (control->shortcut_capture_timeout_id) {
        g_source_remove(control->shortcut_capture_timeout_id);
        control->shortcut_capture_timeout_id = 0;
    }
    meta_display_stop_native_key_capture(control->display);
    meta_display_unregister_native_key_capture_handler(control->display, control);

    gint64 request_id = control->shortcut_capture_request_id;
    control->shortcut_capture_request_id = 0;
    if (!complete || request_id <= 0 || control->stopping)
        return;

    g_autoptr(JsonNode) result = NULL;
    if (ok && accelerator) {
        JsonObject* result_object = json_object_new();
        json_object_set_string_member(result_object, "accelerator", accelerator);
        result = json_node_new(JSON_NODE_OBJECT);
        json_node_take_object(result, result_object);
    }
    dispatch_operation_completion(control, request_id, "shortcut.capture", ok, result, error_code,
                                  message);
}

static gboolean native_shortcut_capture_timed_out(gpointer user_data) {
    GnoblinNativeControl* control = user_data;
    control->shortcut_capture_timeout_id = 0;
    stop_native_shortcut_capture(control, TRUE, FALSE, NULL, "timed_out",
                                 "shortcut capture timed out");
    return G_SOURCE_REMOVE;
}

static gboolean native_keysym_is_modifier(guint keyval) {
    switch (keyval) {
    case XKB_KEY_Shift_L:
    case XKB_KEY_Shift_R:
    case XKB_KEY_Control_L:
    case XKB_KEY_Control_R:
    case XKB_KEY_Caps_Lock:
    case XKB_KEY_Shift_Lock:
    case XKB_KEY_Meta_L:
    case XKB_KEY_Meta_R:
    case XKB_KEY_Alt_L:
    case XKB_KEY_Alt_R:
    case XKB_KEY_Super_L:
    case XKB_KEY_Super_R:
    case XKB_KEY_Hyper_L:
    case XKB_KEY_Hyper_R:
        return TRUE;
    default:
        return FALSE;
    }
}

gboolean native_shortcut_hold_family_pressed(GnoblinNativeControl* control, guint32 hold_mask) {
    if (!control)
        return FALSE;
    if ((hold_mask & CLUTTER_SUPER_MASK) &&
        (control->super_left_pressed || control->super_right_pressed))
        return TRUE;
    if ((hold_mask & CLUTTER_CONTROL_MASK) &&
        (control->control_left_pressed || control->control_right_pressed))
        return TRUE;
    if ((hold_mask & CLUTTER_MOD1_MASK) &&
        (control->alt_left_pressed || control->alt_right_pressed))
        return TRUE;
    return FALSE;
}

void native_shortcut_capture_key(const ClutterEvent* event, gpointer user_data) {
    GnoblinNativeControl* control = user_data;
    if (control->shortcut_session_capture_active && control->active_shortcut_session) {
        NativeDynamicShortcut* shortcut = control->active_shortcut_session;
        if (!event) {
            dynamic_shortcut_end_session(control, shortcut, "preempted");
            return;
        }
        ClutterEventType type = clutter_event_type(event);
        if (type != CLUTTER_KEY_PRESS && type != CLUTTER_KEY_RELEASE)
            return;
        ClutterEventFlags flags = clutter_event_get_flags(event);
        if (flags & (CLUTTER_EVENT_FLAG_SYNTHETIC | CLUTTER_EVENT_FLAG_INPUT_METHOD))
            return;
        guint keyval = clutter_event_get_key_symbol(event);
        if (shortcut == control->bare_super_shortcut && type == CLUTTER_KEY_RELEASE &&
            (keyval == XKB_KEY_Super_L || keyval == XKB_KEY_Super_R) &&
            !native_shortcut_hold_family_pressed(control, CLUTTER_SUPER_MASK)) {
            if (shortcut->trigger_event) {
                shortcut->releasing = TRUE;
                dispatch_dynamic_shortcut_activated(control, 0, shortcut->trigger_event);
                shortcut->releasing = FALSE;
                clutter_event_free(shortcut->trigger_event);
                shortcut->trigger_event = NULL;
            }
            dynamic_shortcut_end_session(control, shortcut, "released");
            return;
        }
        guint32 modifiers = clutter_event_get_state(event);
        GVariantBuilder builder;
        g_variant_builder_init(&builder, G_VARIANT_TYPE_VARDICT);
        g_variant_builder_add(&builder, "{sv}", "id", g_variant_new_string(shortcut->id));
        g_variant_builder_add(&builder, "{sv}", "session_id",
                              g_variant_new_uint64(shortcut->session_id));
        g_variant_builder_add(&builder, "{sv}", "keyval", g_variant_new_uint32(keyval));
        g_variant_builder_add(&builder, "{sv}", "keycode",
                              g_variant_new_uint32(clutter_event_get_key_code(event)));
        g_variant_builder_add(&builder, "{sv}", "modifiers", g_variant_new_uint32(modifiers));
        g_variant_builder_add(
            &builder, "{sv}", "phase",
            g_variant_new_string(type == CLUTTER_KEY_PRESS ? "press" : "release"));
        g_variant_builder_add(&builder, "{sv}", "time",
                              g_variant_new_int64(clutter_event_get_time(event)));
        guint64 focus_handle = 0;
        guint64 focus_generation = 0;
        gint64 focus_expires_at_us = 0;
        char focus_token[65] = {0};
        if (!(flags & CLUTTER_EVENT_FLAG_REPEATED) &&
            issue_focus_context_for_session_key(control, shortcut, event, &focus_handle,
                                                &focus_generation, &focus_expires_at_us,
                                                focus_token)) {
            if (shortcut->client) {
                g_variant_builder_add(&builder, "{sv}", "focus_context",
                                      g_variant_new_string(focus_token));
            } else {
                g_variant_builder_add(&builder, "{sv}", "focus_context_handle",
                                      g_variant_new_uint64(focus_handle));
                g_variant_builder_add(&builder, "{sv}", "focus_context_generation",
                                      g_variant_new_uint64(focus_generation));
                g_variant_builder_add(&builder, "{sv}", "focus_context_expires_at_us",
                                      g_variant_new_int64(focus_expires_at_us));
            }
        }
        g_autoptr(GVariant) payload = g_variant_ref_sink(g_variant_builder_end(&builder));
        dynamic_shortcut_publish_event(control, shortcut, "gnoblin.shortcut.session.key", payload);
        if (type == CLUTTER_KEY_RELEASE && shortcut->hold_mask &&
            !native_shortcut_hold_family_pressed(control, shortcut->hold_mask))
            dynamic_shortcut_end_session(control, shortcut, "released");
        return;
    }
    if (!control->shortcut_capture_active)
        return;
    if (!event) {
        stop_native_shortcut_capture(
            control, TRUE, FALSE, NULL, "busy",
            "shortcut capture cancelled because keyboard input was grabbed");
        return;
    }

    guint keyval = clutter_event_get_key_symbol(event);
    ClutterEventType type = clutter_event_type(event);
    if (type == CLUTTER_KEY_PRESS && keyval == CLUTTER_KEY_Escape) {
        stop_native_shortcut_capture(control, TRUE, FALSE, NULL, "cancelled",
                                     "shortcut capture cancelled");
        return;
    }
    if ((keyval == CLUTTER_KEY_Super_L || keyval == CLUTTER_KEY_Super_R) &&
        type == CLUTTER_KEY_PRESS) {
        control->shortcut_capture_super_pressed = TRUE;
        return;
    }
    if ((keyval == CLUTTER_KEY_Super_L || keyval == CLUTTER_KEY_Super_R) &&
        type == CLUTTER_KEY_RELEASE && control->shortcut_capture_super_pressed) {
        control->shortcut_capture_super_pressed = FALSE;
        if (!(clutter_event_get_state(event) & (CLUTTER_MODIFIER_MASK & ~CLUTTER_SUPER_MASK)))
            stop_native_shortcut_capture(control, TRUE, TRUE, "Super", NULL, NULL);
        return;
    }
    if (type != CLUTTER_KEY_PRESS || native_keysym_is_modifier(keyval))
        return;

    control->shortcut_capture_super_pressed = FALSE;
    g_autofree char* accelerator = meta_accelerator_name(clutter_event_get_state(event), keyval);
    if (!accelerator || !*accelerator)
        return;

    stop_native_shortcut_capture(control, TRUE, TRUE, accelerator, NULL, NULL);
}

gboolean gnoblin_native_control_overlay_modifier_pressed(MetaDisplay* display,
                                                         const ClutterEvent* event) {
    GnoblinNativeControl* control =
        display ? g_object_get_data(G_OBJECT(display), NATIVE_CONTROL_OBJECT_DATA_KEY) : NULL;
    NativeDynamicShortcut* shortcut = control ? control->bare_super_shortcut : NULL;
    if (!control || control->stopping || !control->overlay_modifier_hook_available || !shortcut ||
        !shortcut->armed || !shortcut->capture_input || shortcut->active || !event ||
        clutter_event_type(event) != CLUTTER_KEY_PRESS ||
        (clutter_event_get_flags(event) &
         (CLUTTER_EVENT_FLAG_SYNTHETIC | CLUTTER_EVENT_FLAG_INPUT_METHOD |
          CLUTTER_EVENT_FLAG_REPEATED)))
        return FALSE;
    guint keyval = clutter_event_get_key_symbol(event);
    if (keyval != XKB_KEY_Super_L && keyval != XKB_KEY_Super_R)
        return FALSE;
    if (clutter_event_get_state(event) & (CLUTTER_MODIFIER_MASK & ~CLUTTER_SUPER_MASK))
        return FALSE;
    if (control->shortcut_capture_active || control->active_shortcut_session ||
        !meta_display_can_start_native_key_capture(display) ||
        !meta_display_register_native_key_capture_handler(display, native_shortcut_capture_key,
                                                          control, NULL))
        return FALSE;

    if (!shortcut->session_id) {
        shortcut->session_id = ++control->next_shortcut_session_id;
        if (!shortcut->session_id)
            shortcut->session_id = ++control->next_shortcut_session_id;
    }
    shortcut->active = TRUE;
    shortcut->activation_dispatched = FALSE;
    shortcut->held_mask = CLUTTER_SUPER_MASK;
    shortcut->trigger_event = clutter_event_copy(event);
    shortcut->timeout_id = g_timeout_add_seconds(10, native_shortcut_session_timeout, shortcut);
    control->active_shortcut_session = shortcut;
    control->shortcut_session_capture_active = TRUE;
    if (!meta_display_start_native_key_capture(display)) {
        control->shortcut_session_capture_active = FALSE;
        control->active_shortcut_session = NULL;
        shortcut->active = FALSE;
        if (shortcut->timeout_id) {
            g_source_remove(shortcut->timeout_id);
            shortcut->timeout_id = 0;
        }
        clutter_event_free(shortcut->trigger_event);
        shortcut->trigger_event = NULL;
        meta_display_unregister_native_key_capture_handler(display, control);
        return FALSE;
    }
    return TRUE;
}

void gnoblin_native_control_set_overlay_modifier_hook_available(MetaDisplay* display,
                                                                gboolean available) {
    if (!display)
        return;
    g_object_set_data(G_OBJECT(display), OVERLAY_MODIFIER_HOOK_AVAILABLE_DATA_KEY,
                      GINT_TO_POINTER(available));
    GnoblinNativeControl* control =
        g_object_get_data(G_OBJECT(display), NATIVE_CONTROL_OBJECT_DATA_KEY);
    if (!control)
        return;
    control->overlay_modifier_hook_available = available;
    if (!available && control->bare_super_shortcut) {
        NativeDynamicShortcut* shortcut = control->bare_super_shortcut;
        dynamic_shortcut_end_session(control, shortcut, "preempted");
        control->bare_super_shortcut = NULL;
        gnoblin_control_dynamic_shortcut_free(shortcut);
    }
}

void gnoblin_native_control_cancel_shortcut_session(MetaDisplay* display, guint64 session_id,
                                                    const char* reason) {
    GnoblinNativeControl* control =
        display ? g_object_get_data(G_OBJECT(display), NATIVE_CONTROL_OBJECT_DATA_KEY) : NULL;
    if (!control || !control->active_shortcut_session || !session_id ||
        control->active_shortcut_session->session_id != session_id)
        return;
    dynamic_shortcut_end_session(control, control->active_shortcut_session,
                                 reason ? reason : "cancelled");
}

gboolean gnoblin_native_control_begin_shortcut_capture(MetaDisplay* display, GVariant* arguments,
                                                       gint64 request_id, GError** error) {
    GnoblinNativeControl* control =
        display ? g_object_get_data(G_OBJECT(display), NATIVE_CONTROL_OBJECT_DATA_KEY) : NULL;
    if (!control || control->stopping || request_id <= 0 || !arguments ||
        !g_variant_is_of_type(arguments, G_VARIANT_TYPE_VARDICT)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_SUPPORTED,
                            "native shortcut capture is unavailable");
        return FALSE;
    }
    if (control->shortcut_capture_active) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_BUSY,
                            "another shortcut capture is active");
        return FALSE;
    }
    g_autoptr(JsonNode) json = json_from_variant(arguments);
    if (!JSON_NODE_HOLDS_OBJECT(json)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "shortcut.capture accepts an optional integer timeout");
        return FALSE;
    }
    JsonObject* capture_options = json_node_get_object(json);
    if (json_object_get_size(capture_options) > 1 ||
        (json_object_get_size(capture_options) == 1 &&
         !json_object_has_member(capture_options, "timeout"))) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "shortcut.capture accepts only timeout");
        return FALSE;
    }
    JsonNode* timeout_node = json_object_get_member(capture_options, "timeout");
    if (timeout_node && (!JSON_NODE_HOLDS_VALUE(timeout_node) ||
                         (json_node_get_value_type(timeout_node) != G_TYPE_INT &&
                          json_node_get_value_type(timeout_node) != G_TYPE_INT64))) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "shortcut.capture timeout must be an integer");
        return FALSE;
    }
    gint64 timeout_seconds = 30;
    if (timeout_node)
        timeout_seconds = json_node_get_int(timeout_node);
    if (timeout_seconds < 1 || timeout_seconds > 60) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "shortcut capture timeout must be from 1 to 60 seconds");
        return FALSE;
    }
    if (!control->wayland_compositor ||
        meta_wayland_session_lock_is_active(control->wayland_compositor)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "shortcut capture is unavailable while the session is locked");
        return FALSE;
    }
    if (!meta_display_can_start_native_key_capture(display)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_BUSY,
                            "keyboard input is already grabbed or captured");
        return FALSE;
    }
    if (!meta_display_register_native_key_capture_handler(display, native_shortcut_capture_key,
                                                          control, NULL)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_BUSY,
                            "another input capture owner is registered");
        return FALSE;
    }
    control->shortcut_capture_request_id = request_id;
    control->shortcut_capture_active = TRUE;
    if (!meta_display_start_native_key_capture(display)) {
        control->shortcut_capture_active = FALSE;
        control->shortcut_capture_request_id = 0;
        meta_display_unregister_native_key_capture_handler(display, control);
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_BUSY,
                            "another input capture owner is active");
        return FALSE;
    }
    control->shortcut_capture_timeout_id =
        g_timeout_add_seconds((guint)timeout_seconds, native_shortcut_capture_timed_out, control);
    return TRUE;
}

static const char* native_session_lock_state_name(MetaWaylandSessionLockState state) {
    switch (state) {
    case META_WAYLAND_SESSION_LOCK_UNLOCKED:
        return "unlocked";
    case META_WAYLAND_SESSION_LOCK_COVERING:
        return "covering";
    case META_WAYLAND_SESSION_LOCK_LOCKED:
        return "locked";
    case META_WAYLAND_SESSION_LOCK_FAILSAFE:
    default:
        return "failsafe";
    }
}

GVariant* native_session_lock_snapshot(MetaWaylandCompositor* compositor, guint64 revision) {
    gboolean available = compositor && meta_wayland_session_lock_get_capability(compositor) != 0;
    GVariantBuilder builder;
    g_variant_builder_init(&builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&builder, "{sv}", "lock_available", g_variant_new_boolean(available));
    g_variant_builder_add(&builder, "{sv}", "revision", g_variant_new_uint64(revision));
    if (available)
        g_variant_builder_add(&builder, "{sv}", "lock_state",
                              g_variant_new_string(native_session_lock_state_name(
                                  meta_wayland_session_lock_get_state(compositor))));
    return g_variant_ref_sink(g_variant_builder_end(&builder));
}

void native_session_lock_changed(MetaWaylandCompositor* compositor,
                                 MetaWaylandSessionLockState state, gpointer user_data) {
    GnoblinNativeControl* control = user_data;
    if (!control || control->stopping)
        return;
    if (state != META_WAYLAND_SESSION_LOCK_UNLOCKED) {
        native_input_cancel_pending(control, FALSE);
        gnoblin_native_control_revoke_focus_contexts(control->display);
        gnoblin_control_cancel_window_drags(control, "locked");
        g_hash_table_remove_all(control->snap_contexts);
        if (control->active_shortcut_session)
            dynamic_shortcut_end_session(control, control->active_shortcut_session, "locked");
        stop_native_shortcut_capture(user_data, TRUE, FALSE, NULL, "denied",
                                     "shortcut capture cancelled because the session locked");
    }
    /* Companion constraints never participate in the lock screen stack. */
    meta_stack_update_transient(control->display->stack);
    const char* state_name = native_session_lock_state_name(state);

    guint64 sequence = ++control->event_sequence;
    control->session_lock_revision = sequence;
    gint64 time = g_get_monotonic_time();
    GVariantBuilder payload_builder;
    g_variant_builder_init(&payload_builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&payload_builder, "{sv}", "state", g_variant_new_string(state_name));
    g_variant_builder_add(&payload_builder, "{sv}", "revision", g_variant_new_uint64(sequence));
    g_variant_builder_add(&payload_builder, "{sv}", "sequence", g_variant_new_int64(sequence));
    g_variant_builder_add(&payload_builder, "{sv}", "time", g_variant_new_int64(time));
    g_autoptr(GVariant) payload = g_variant_ref_sink(g_variant_builder_end(&payload_builder));
    g_autoptr(GVariant) status_snapshot =
        native_session_lock_snapshot(compositor, control->session_lock_revision);
    native_publish_runtime_snapshot(control, "session-lock", status_snapshot, sequence);
    g_autoptr(JsonNode) root = json_node_new(JSON_NODE_OBJECT);
    JsonObject* object = json_object_new();
    json_node_take_object(root, object);
    json_object_set_string_member(object, "name", "gnoblin.session.lock-state-changed");
    json_object_set_string_member(object, "state", state_name);
    json_object_set_int_member(object, "revision", sequence);
    json_object_set_int_member(object, "sequence", sequence);
    json_object_set_int_member(object, "time", time);
    publish_native_socket_event(control, root);
    native_runtime_dispatch_event(control, "gnoblin.session.lock-state-changed", payload);
}

GVariant* gnoblin_native_control_request_session_lock(MetaDisplay* display, GVariant* arguments,
                                                      GError** error) {
    GnoblinNativeControl* control =
        display ? g_object_get_data(G_OBJECT(display), NATIVE_CONTROL_OBJECT_DATA_KEY) : NULL;
    const char* const event_name = "gnoblin.session.lock-requested";
    guint listeners = 0;
    GHashTableIter iterator;
    gpointer value;

    if (!control || control->stopping || !arguments ||
        !g_variant_is_of_type(arguments, G_VARIANT_TYPE_VARDICT) ||
        g_variant_n_children(arguments) != 0) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "session.lock does not accept arguments");
        return NULL;
    }
    if (!control->wayland_compositor ||
        meta_wayland_session_lock_get_capability(control->wayland_compositor) == 0) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_SUPPORTED,
                            "the compositor session-lock protocol is unavailable");
        return NULL;
    }
    if (meta_wayland_session_lock_get_state(control->wayland_compositor) !=
        META_WAYLAND_SESSION_LOCK_UNLOCKED) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_BUSY,
                            "the session is already locked or changing lock state");
        return NULL;
    }

    g_hash_table_iter_init(&iterator, control->clients);
    while (g_hash_table_iter_next(&iterator, NULL, &value)) {
        Client* client = value;
        if (!client->closing && client->api_minor >= 21 && client->event_api_minor >= 21 &&
            client->event_subscriptions &&
            g_hash_table_contains(client->event_subscriptions, event_name))
            listeners++;
    }
    if (listeners == 0) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_CONNECTED,
                            "no session-lock client is subscribed to lock requests");
        return NULL;
    }

    guint64 sequence = ++control->event_sequence;
    gint64 time = g_get_monotonic_time();
    g_autoptr(JsonNode) event = json_node_new(JSON_NODE_OBJECT);
    JsonObject* object = json_object_new();
    json_node_take_object(event, object);
    json_object_set_string_member(object, "name", event_name);
    json_object_set_int_member(object, "sequence", sequence);
    json_object_set_int_member(object, "time", time);
    publish_native_socket_event(control, event);

    GVariantBuilder result;
    g_variant_builder_init(&result, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&result, "{sv}", "dispatched", g_variant_new_boolean(TRUE));
    g_variant_builder_add(&result, "{sv}", "subscribers", g_variant_new_uint32(listeners));
    return g_variant_ref_sink(g_variant_builder_end(&result));
}
