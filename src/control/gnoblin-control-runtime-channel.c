#include "gnoblin-native-control-private.h"

gboolean native_runtime_dispatch_event(GnoblinNativeControl* control, const char* event,
                                       GVariant* payload) {
    if (!control || control->stopping || !event || !*event || !payload ||
        !g_variant_is_of_type(payload, G_VARIANT_TYPE_VARDICT))
        return FALSE;
    GVariantBuilder event_packet;
    g_variant_builder_init(&event_packet, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&event_packet, "{sv}", "event", g_variant_new_string(event));
    gboolean trusted_binding = g_str_equal(event, "gnoblin.shortcut.binding-activated");
    gboolean pointer_binding = g_str_equal(event, "gnoblin.pointer.binding-activated");
    gboolean session_key = g_str_equal(event, "gnoblin.shortcut.session.key");
    if (g_str_equal(event, "gnoblin.shortcut.activated") || trusted_binding || pointer_binding ||
        session_key) {
        gboolean first = FALSE;
        if (trusted_binding && (!g_variant_lookup(payload, "first", "b", &first) || !first))
            return FALSE;
        g_autoptr(GVariant) context_handle =
            g_variant_lookup_value(payload, "focus_context_handle", NULL);
        g_autoptr(GVariant) context_generation =
            g_variant_lookup_value(payload, "focus_context_generation", NULL);
        g_autoptr(GVariant) context_expiry =
            g_variant_lookup_value(payload, "focus_context_expires_at_us", NULL);
        gboolean has_any_context = context_handle || context_generation || context_expiry;
        gboolean has_all_context = context_handle && context_generation && context_expiry;
        if (!has_any_context && (trusted_binding || pointer_binding || session_key)) {
            g_variant_builder_add(&event_packet, "{sv}", "payload", payload);
            g_autoptr(GVariant) packet = g_variant_ref_sink(g_variant_builder_end(&event_packet));
            return native_runtime_send_event_packet(control, event, packet);
        }
        if (!has_all_context)
            return FALSE;
        guint64 handle = 0;
        guint64 generation = 0;
        gint64 expires_at_us = 0;
        if (!g_variant_lookup(payload, "focus_context_handle", "t", &handle) || !handle ||
            !g_variant_lookup(payload, "focus_context_generation", "t", &generation) ||
            !generation ||
            !g_variant_lookup(payload, "focus_context_expires_at_us", "x", &expires_at_us) ||
            expires_at_us <= g_get_monotonic_time())
            return FALSE;
        GVariantBuilder clean_payload;
        GVariantIter fields;
        const char* field_name;
        GVariant* field_value;
        g_variant_builder_init(&clean_payload, G_VARIANT_TYPE_VARDICT);
        g_variant_iter_init(&fields, payload);
        while (g_variant_iter_next(&fields, "{&sv}", &field_name, &field_value)) {
            g_autoptr(GVariant) value = field_value;
            if (g_str_equal(field_name, "focus_context_handle") ||
                g_str_equal(field_name, "focus_context_generation") ||
                g_str_equal(field_name, "focus_context_expires_at_us"))
                continue;
            g_variant_builder_add(&clean_payload, "{sv}", field_name, value);
        }
        g_variant_builder_add(&event_packet, "{sv}", "payload",
                              g_variant_builder_end(&clean_payload));
        g_variant_builder_add(&event_packet, "{sv}", "focus_context_handle",
                              g_variant_new_uint64(handle));
        g_variant_builder_add(&event_packet, "{sv}", "focus_context_generation",
                              g_variant_new_uint64(generation));
        g_variant_builder_add(&event_packet, "{sv}", "focus_context_expires_at_us",
                              g_variant_new_int64(expires_at_us));
    } else {
        g_variant_builder_add(&event_packet, "{sv}", "payload", payload);
    }
    g_autoptr(GVariant) packet = g_variant_ref_sink(g_variant_builder_end(&event_packet));
    return native_runtime_send_event_packet(control, event, packet);
}

static GVariant* native_runtime_shortcut_operation(GnoblinNativeControl* control,
                                                   const char* method, GVariant* arguments,
                                                   gint64 operation_id, GError** error) {
    const char* id = NULL;
    if (g_str_equal(method, "shortcut.session.end")) {
        gint64 signed_session_id = 0;
        if (!g_variant_lookup(arguments, "id", "&s", &id) ||
            !gnoblin_control_dynamic_shortcut_id_valid(id) ||
            !g_variant_lookup(arguments, "session_id", "x", &signed_session_id) ||
            signed_session_id <= 0) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                                "shortcut.session.end requires a valid id and positive session_id");
            return NULL;
        }
        guint64 session_id = (guint64)signed_session_id;
        guint64 generation = native_config_generation(control);
        g_autofree char* owner_id = g_strdup_printf("lua:%" G_GUINT64_FORMAT, generation);
        NativeDynamicShortcut* shortcut =
            gnoblin_control_find_runtime_dynamic_shortcut(control, owner_id, id);
        if (!shortcut || !shortcut->active || shortcut->session_id != session_id) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_FOUND,
                                "shortcut session is no longer active for this Lua binding");
            return NULL;
        }
        dynamic_shortcut_end_session(control, shortcut, "cancelled");
        GVariantBuilder result;
        g_variant_builder_init(&result, G_VARIANT_TYPE_VARDICT);
        g_variant_builder_add(&result, "{sv}", "id", g_variant_new_string(id));
        g_variant_builder_add(&result, "{sv}", "session_id", g_variant_new_uint64(session_id));
        g_variant_builder_add(&result, "{sv}", "ended", g_variant_new_boolean(TRUE));
        return g_variant_ref_sink(g_variant_builder_end(&result));
    }
    if (g_str_equal(method, "shortcut.unbind")) {
        if (!g_variant_lookup(arguments, "id", "&s", &id) ||
            !gnoblin_control_dynamic_shortcut_id_valid(id)) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                                "shortcut.unbind requires a valid id");
            return NULL;
        }
        guint64 generation = native_config_generation(control);
        g_autofree char* owner_id = g_strdup_printf("lua:%" G_GUINT64_FORMAT, generation);
        NativeDynamicShortcut* shortcut =
            gnoblin_control_find_runtime_dynamic_shortcut(control, owner_id, id);
        if (!shortcut) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_FOUND,
                                "shortcut ID is not registered by this Lua runtime");
            return NULL;
        }
        gnoblin_control_remove_dynamic_shortcut(control, shortcut);
        GVariantBuilder result;
        g_variant_builder_init(&result, G_VARIANT_TYPE_VARDICT);
        g_variant_builder_add(&result, "{sv}", "id", g_variant_new_string(id));
        g_variant_builder_add(&result, "{sv}", "unbound", g_variant_new_boolean(TRUE));
        return g_variant_ref_sink(g_variant_builder_end(&result));
    }

    const char* accelerator = NULL;
    const char* hold = "none";
    const char* trigger = "press";
    const char* mode = "passive";
    gboolean capture_input = FALSE;
    if (!g_variant_lookup(arguments, "id", "&s", &id) ||
        !gnoblin_control_dynamic_shortcut_id_valid(id) ||
        !g_variant_lookup(arguments, "accelerator", "&s", &accelerator) ||
        !gnoblin_control_dynamic_shortcut_accelerator_valid(accelerator)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "shortcut.bind requires a valid id and accelerator");
        return NULL;
    }
    g_variant_lookup(arguments, "hold", "&s", &hold);
    g_variant_lookup(arguments, "trigger", "&s", &trigger);
    g_variant_lookup(arguments, "mode", "&s", &mode);
    g_variant_lookup(arguments, "capture_input", "b", &capture_input);
    if (!((g_str_equal(hold, "none")) || g_str_equal(hold, "super") ||
          g_str_equal(hold, "control") || g_str_equal(hold, "alt")) ||
        (!g_str_equal(trigger, "press") && !g_str_equal(trigger, "release")) ||
        (!g_str_equal(mode, "passive") && !g_str_equal(mode, "modal")) ||
        (g_str_equal(mode, "modal") && g_str_equal(hold, "none")) ||
        (capture_input && !g_str_equal(accelerator, "Super")) ||
        (g_str_equal(accelerator, "Super") &&
         (!capture_input || !g_str_equal(trigger, "release") || !g_str_equal(hold, "none")))) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "shortcut.bind has an unsupported hold, trigger, mode, or "
                            "capture_input combination");
        return NULL;
    }
    guint64 generation = native_config_generation(control);
    g_autofree char* owner_id = g_strdup_printf("lua:%" G_GUINT64_FORMAT, generation);
    if (!generation || gnoblin_control_find_runtime_dynamic_shortcut(control, owner_id, id)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_EXISTS,
                            "shortcut ID already exists in this Lua runtime");
        return NULL;
    }
    NativeDynamicShortcut* shortcut = NULL;
    if (g_str_equal(accelerator, "Super")) {
        guint64 session_id = (guint64)operation_id;
        shortcut =
            gnoblin_control_arm_bare_super_shortcut(control, id, owner_id, NULL, session_id, error);
        if (!shortcut)
            return NULL;
    } else {
        guint action = meta_display_grab_accelerator(control->display, accelerator, 0);
        if (action == META_KEYBINDING_ACTION_NONE) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_BUSY,
                                "shortcut.bind accelerator is already claimed");
            return NULL;
        }
        shortcut = g_new0(NativeDynamicShortcut, 1);
        shortcut->control = control;
        shortcut->id = g_strdup(id);
        shortcut->accelerator = g_strdup(accelerator);
        shortcut->owner_id = g_strdup(owner_id);
        shortcut->action = action;
        shortcut->trigger_release = g_str_equal(trigger, "release");
        shortcut->modal = g_str_equal(mode, "modal");
        shortcut->capture_input = capture_input;
        shortcut->hold_mask = g_str_equal(hold, "super")     ? CLUTTER_SUPER_MASK
                              : g_str_equal(hold, "control") ? CLUTTER_CONTROL_MASK
                              : g_str_equal(hold, "alt")     ? CLUTTER_MOD1_MASK
                                                             : 0;
        shortcut->armed = TRUE;
        g_hash_table_insert(control->dynamic_shortcuts, GUINT_TO_POINTER(action), shortcut);
    }
    GVariantBuilder result;
    g_variant_builder_init(&result, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&result, "{sv}", "id", g_variant_new_string(id));
    g_variant_builder_add(&result, "{sv}", "accelerator", g_variant_new_string(accelerator));
    g_variant_builder_add(&result, "{sv}", "hold", g_variant_new_string(hold));
    g_variant_builder_add(&result, "{sv}", "trigger", g_variant_new_string(trigger));
    g_variant_builder_add(&result, "{sv}", "mode", g_variant_new_string(mode));
    g_variant_builder_add(&result, "{sv}", "capture_input", g_variant_new_boolean(capture_input));
    return g_variant_ref_sink(g_variant_builder_end(&result));
}

gboolean native_runtime_flush_state_snapshots(GnoblinNativeControl* control, GError** error) {
    return gnoblin_control_runtime_flush_state_snapshots(control, error);
}

gboolean native_runtime_flush_pending_events(GnoblinNativeControl* control, GError** error) {
    return gnoblin_control_runtime_flush_pending_events(control, error);
}

void native_runtime_fail_pending_requests(GnoblinNativeControl* control, const char* reason) {
    if (!control || !control->pending_runtime_requests)
        return;
    g_autoptr(GPtrArray) pending_requests =
        g_ptr_array_new_with_free_func(pending_runtime_request_free);
    GHashTableIter iter;
    gpointer key;
    gpointer value;
    g_hash_table_iter_init(&iter, control->pending_runtime_requests);
    while (g_hash_table_iter_next(&iter, &key, &value)) {
        g_ptr_array_add(pending_requests, value);
        g_hash_table_iter_steal(&iter);
        g_free(key);
    }
    for (guint i = 0; i < pending_requests->len; i++) {
        PendingRuntimeRequest* pending = g_ptr_array_index(pending_requests, i);
        if (pending->discard_response)
            continue;
        if (pending->callback) {
            pending->callback(FALSE, NULL, reason ? reason : "Lua worker stopped before replying",
                              pending->callback_data);
            continue;
        }
        Client* client = pending->client;
        if (!client->closing) {
            send_response(client,
                          encode_response(pending->request_id, NULL,
                                          reason ? reason : "Lua worker stopped before replying"));
            process_buffer(client);
            if (client->pending_deferred_requests > 0)
                client->pending_deferred_requests--;
        } else if (client->pending_deferred_requests > 0) {
            client->pending_deferred_requests--;
        }
        client_maybe_free(client);
    }
    if (control->runtime_operation_ids && control->runtime_cancelled_operation_ids) {
        GHashTableIter operation_iter;
        gpointer operation_key;
        g_hash_table_iter_init(&operation_iter, control->runtime_operation_ids);
        while (g_hash_table_iter_next(&operation_iter, &operation_key, NULL)) {
            guint64* cancelled_key = g_new(guint64, 1);
            *cancelled_key = *(guint64*)operation_key;
            g_hash_table_add(control->runtime_cancelled_operation_ids, cancelled_key);
        }
        g_hash_table_remove_all(control->runtime_operation_ids);
    }
}

static gboolean native_runtime_send_worker_suspended(GnoblinNativeControl* control,
                                                     GError** error) {
    return gnoblin_control_runtime_send_worker_suspended(control, error);
}

static void native_runtime_suspend_worker(GnoblinNativeControl* control) {
    if (!control || control->runtime_worker_suspended)
        return;
    control->runtime_worker_suspended = TRUE;
    control->runtime_hello_sent = FALSE;
    native_input_cancel_pending(
        control, !(control->wayland_compositor &&
                   meta_wayland_session_lock_is_active(control->wayland_compositor)));
    /* The host discards output through the acknowledgement. Drop queued
     * fragments first so an old message cannot continue after SUSPENDED. */
    gnoblin_runtime_writer_reset(control->runtime_writer);
    gnoblin_runtime_reader_reset(control->runtime_reader);
    gnoblin_control_runtime_clear_queue(control->pending_runtime_states);
    gnoblin_control_runtime_clear_queue(control->pending_runtime_events);
    native_runtime_fail_pending_requests(control, "Lua worker restarted before replying");
    gnoblin_control_cancel_window_drags(control, "runtime_restarted");
    /* Do not carry an in-progress gesture from one Lua worker generation to
     * the next. The replacement will receive only complete new gestures. */
    gnoblin_touchpad_router_reset(control->touchpad_router);
    gnoblin_control_clear_runtime_dynamic_shortcuts(control, "runtime_restarted");
    stop_native_shortcut_capture(control, TRUE, FALSE, NULL, "unavailable",
                                 "shortcut capture cancelled because the Lua worker restarted");
    revoke_focus_contexts(control);
    revoke_menu_contexts(control);
    revoke_text_targets(control);
    if (control->snap_contexts)
        g_hash_table_remove_all(control->snap_contexts);
    g_autoptr(GError) error = NULL;
    if (!native_runtime_send_worker_suspended(control, &error)) {
        g_warning("gnoblin-native-control: could not acknowledge Lua worker suspension: %s",
                  error ? error->message : "unknown error");
        native_runtime_abort(control);
    } else
        native_runtime_publish_status(control);
}

static GHashTable* native_runtime_event_subscriptions_from_payload(GVariant* payload,
                                                                   GError** error) {
    return gnoblin_control_runtime_event_subscriptions_from_payload(payload, error);
}

static gboolean native_runtime_republish_full_state(GnoblinNativeControl* control, GError** error) {
    guint64 revision = control->state_revision;
    g_autoptr(GError) snapshot_error = NULL;
    /* These helpers publish normalized Lua snapshots themselves. Re-publishing
     * their native JSON results here would overwrite derived Lua fields. */
    {
        g_autoptr(JsonNode) windows = window_snapshot_json(control, TRUE, &snapshot_error);
        (void)windows;
        if (!windows)
            g_warning("gnoblin-native-control: cannot restore window snapshot: %s",
                      snapshot_error ? snapshot_error->message : "window listing unavailable");
    }

    g_clear_error(&snapshot_error);
    {
        g_autoptr(JsonNode) workspaces =
            gnoblin_control_workspace_snapshot_json(control, &snapshot_error);
        (void)workspaces;
        if (!workspaces)
            g_warning("gnoblin-native-control: cannot restore workspace snapshot: %s",
                      snapshot_error ? snapshot_error->message : "workspace listing unavailable");
    }

    g_clear_error(&snapshot_error);
    g_autoptr(JsonNode) monitors =
        gnoblin_control_monitor_snapshot_json(control, TRUE, &snapshot_error);
    if (!monitors)
        g_warning("gnoblin-native-control: cannot restore monitor snapshot: %s",
                  snapshot_error ? snapshot_error->message : "monitor listing unavailable");
    g_autoptr(GVariant) monitor_value = monitors ? variant_from_json(monitors) : NULL;
    native_publish_runtime_snapshot(control, "monitors", monitor_value, revision);

    g_clear_error(&snapshot_error);
    g_autoptr(JsonNode) outputs =
        gnoblin_control_output_snapshot_json(control, TRUE, &snapshot_error);
    g_autoptr(GVariant) output_value = outputs ? variant_from_json(outputs) : NULL;
    native_publish_runtime_snapshot(control, "outputs", output_value, revision);
    native_monitor_privacy_screen_publish(control);

    g_clear_error(&snapshot_error);
    g_autoptr(JsonNode) layers =
        gnoblin_control_layer_snapshot_json(control, TRUE, &snapshot_error);
    if (!layers)
        g_warning("gnoblin-native-control: cannot restore layer snapshot: %s",
                  snapshot_error ? snapshot_error->message : "layer listing unavailable");
    g_autoptr(GVariant) layer_value = layers ? variant_from_json(layers) : NULL;
    native_publish_runtime_snapshot(control, "layers", layer_value, revision);

    g_autoptr(GVariant) capabilities = capability_snapshot(control);
    native_publish_runtime_snapshot(control, "capabilities", capabilities, revision);
    g_autoptr(GVariant) devices = input_device_snapshot(control);
    native_publish_runtime_snapshot(control, "input-devices", devices, revision);
    g_autoptr(GVariant) sources = gnoblin_control_input_source_snapshot(control);
    native_publish_runtime_snapshot(control, "input-sources", sources, revision);
    g_autoptr(GVariant) appearance = appearance_snapshot_new(control);
    native_publish_runtime_snapshot(control, "appearance", appearance,
                                    control->appearance_revision);
    g_autoptr(GVariant) orientation_lock =
        native_orientation_lock_snapshot(control, control->orientation_lock_revision);
    native_publish_runtime_snapshot(control, "input-orientation-lock", orientation_lock,
                                    control->orientation_lock_revision);
    g_autoptr(GVariant) shortcuts = native_shortcut_snapshot(control);
    native_publish_runtime_snapshot(control, "shortcuts", shortcuts, revision);
    gnoblin_control_publish_privacy_snapshot(control, FALSE);
    if (control->session_activity_snapshot)
        native_publish_runtime_snapshot(control, "session-activity",
                                        control->session_activity_snapshot,
                                        control->session_activity_revision);
    if (control->session_lifecycle_supported && control->session_lifecycle_snapshot)
        native_publish_runtime_snapshot(control, "session-lifecycle",
                                        control->session_lifecycle_snapshot,
                                        control->session_lifecycle_revision);
    g_autoptr(GVariant) lock_snapshot =
        native_session_lock_snapshot(control->wayland_compositor, control->session_lock_revision);
    native_publish_runtime_snapshot(control, "session-lock", lock_snapshot,
                                    control->session_lock_revision);
    update_launch_snapshot(control);
    return native_runtime_flush_state_snapshots(control, error);
}

static gboolean native_runtime_handle_session_lifecycle_state(GnoblinNativeControl* control,
                                                              const GnoblinRuntimePacket* packet,
                                                              GError** error) {
    const char* state = NULL;
    const char* previous_state = NULL;
    guint64 revision = 0;
    if (!control->supervised_runtime || !control->session_lifecycle_supported ||
        packet->request_id != 0 || g_variant_n_children(packet->payload) != 2 ||
        !g_variant_lookup(packet->payload, "state", "&s", &state) || !state ||
        !g_variant_lookup(packet->payload, "revision", "t", &revision) || revision == 0) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                            "guardian sent an invalid session lifecycle update");
        return FALSE;
    }
    if (control->session_lifecycle_snapshot)
        g_variant_lookup(control->session_lifecycle_snapshot, "state", "&s", &previous_state);
    if (revision == control->session_lifecycle_revision && g_strcmp0(state, previous_state) == 0) {
        /* The guardian may retry after the packet reached Mutter but its
         * bounded write attempt did not observe completion. */
    } else {
        gboolean valid_transition =
            (!previous_state && g_str_equal(state, "starting")) ||
            (g_strcmp0(previous_state, "starting") == 0 &&
             (g_str_equal(state, "running") || g_str_equal(state, "stopping"))) ||
            (g_strcmp0(previous_state, "running") == 0 && g_str_equal(state, "stopping"));
        if (!valid_transition || control->session_lifecycle_revision == G_MAXUINT64 ||
            revision != control->session_lifecycle_revision + 1) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                                "guardian sent an out-of-order session lifecycle update");
            return FALSE;
        }
        control->session_lifecycle_revision = revision;
        g_clear_pointer(&control->session_lifecycle_snapshot, g_variant_unref);
        control->session_lifecycle_snapshot = g_variant_ref(packet->payload);
        native_publish_runtime_snapshot(control, "session-lifecycle",
                                        control->session_lifecycle_snapshot, revision);
        native_publish_request_event(control, "gnoblin.session.state-changed",
                                     control->session_lifecycle_snapshot);
    }

    if (g_str_equal(state, "stopping")) {
        g_autoptr(GVariant) empty =
            g_variant_ref_sink(g_variant_new_array(G_VARIANT_TYPE("{sv}"), NULL, 0));
        return native_runtime_send(control, GNOBLIN_RUNTIME_PACKET_SESSION_STATE_PUBLISHED,
                                   revision, empty, error);
    }
    return TRUE;
}

static GVariant* native_set_orientation_lock(GnoblinNativeControl* control, GVariant* arguments,
                                             GError** error) {
    if (!arguments || !g_variant_is_of_type(arguments, G_VARIANT_TYPE_VARDICT) ||
        g_variant_n_children(arguments) != 1) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "input.set_orientation_lock requires only value");
        return NULL;
    }
    if (!control->orientation_manager) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_SUPPORTED,
                            "orientation lock is unavailable on this backend");
        return NULL;
    }

    g_autoptr(GVariant) value = g_variant_lookup_value(arguments, "value", NULL);
    if (!value) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "input.set_orientation_lock requires value");
        return NULL;
    }

    if (g_variant_is_of_type(value, G_VARIANT_TYPE_BOOLEAN)) {
        control->orientation_lock_runtime_override = TRUE;
        control->orientation_lock_notifications_suppressed = TRUE;
        meta_orientation_manager_set_orientation_locked(control->orientation_manager,
                                                        g_variant_get_boolean(value));
    } else if (g_variant_is_of_type(value, G_VARIANT_TYPE_STRING) &&
               g_str_equal(g_variant_get_string(value, NULL), "inherit")) {
        control->orientation_lock_runtime_override = FALSE;
        control->orientation_lock_notifications_suppressed = TRUE;
        if (control->orientation_lock_configured)
            meta_orientation_manager_set_orientation_locked(control->orientation_manager,
                                                            control->orientation_lock_config_value);
        else
            meta_orientation_manager_clear_orientation_lock_override(control->orientation_manager);
    } else {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "input.set_orientation_lock value must be boolean or 'inherit'");
        return NULL;
    }

    control->orientation_lock_notifications_suppressed = FALSE;
    native_orientation_lock_publish(control);
    return g_variant_ref(control->orientation_lock_snapshot);
}

static GVariant* native_set_monitor_privacy_screen(GnoblinNativeControl* control,
                                                   GVariant* arguments, GError** error) {
    if (!arguments || !g_variant_is_of_type(arguments, G_VARIANT_TYPE_VARDICT) ||
        g_variant_n_children(arguments) != 1) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "monitors.set_privacy_screen requires only value");
        return NULL;
    }

    g_autoptr(GVariant) value = g_variant_lookup_value(arguments, "value", NULL);
    if (value && g_variant_is_of_type(value, G_VARIANT_TYPE_BOOLEAN)) {
        control->monitor_privacy_screen_runtime_override = TRUE;
        control->monitor_privacy_screen_runtime_value = g_variant_get_boolean(value);
    } else if (value && g_variant_is_of_type(value, G_VARIANT_TYPE_STRING) &&
               g_str_equal(g_variant_get_string(value, NULL), "inherit")) {
        control->monitor_privacy_screen_runtime_override = FALSE;
        control->monitor_privacy_screen_runtime_value = FALSE;
    } else {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "monitors.set_privacy_screen value must be boolean or 'inherit'");
        return NULL;
    }

    MetaSettings* settings = meta_backend_get_settings(control->backend);
    meta_settings_apply_gnoblin_privacy_screen_runtime_override(
        settings, control->monitor_privacy_screen_runtime_override,
        control->monitor_privacy_screen_runtime_value);
    native_monitor_privacy_screen_publish(control);
    return control->monitor_privacy_screen_snapshot
               ? g_variant_ref(control->monitor_privacy_screen_snapshot)
               : native_monitor_privacy_screen_snapshot(control,
                                                        control->monitor_privacy_screen_revision);
}

static gboolean native_runtime_handle_operation(GnoblinNativeControl* control,
                                                GnoblinRuntimePacket* packet, GError** error) {
    if (!control || control->stopping) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_CLOSED,
                            "Gnoblin compositor control is stopping");
        return FALSE;
    }
    gint64 operation_id = 0;
    guint64 client_id = 0;
    const char* method = NULL;
    g_autoptr(GVariant) arguments =
        g_variant_lookup_value(packet->payload, "arguments", G_VARIANT_TYPE_VARDICT);
    if (packet->request_id > G_MAXINT64 ||
        !g_variant_lookup(packet->payload, "request_id", "x", &operation_id) || operation_id <= 0 ||
        (guint64)operation_id != packet->request_id ||
        !g_variant_lookup(packet->payload, "method", "&s", &method) || !method || !*method ||
        !arguments) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                            "supervisor sent a malformed native operation");
        return FALSE;
    }
    if ((guint64)operation_id <= control->last_runtime_operation_id) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                            "supervisor reused a native operation ID");
        return FALSE;
    }
    control->last_runtime_operation_id = (guint64)operation_id;
    g_autoptr(GError) operation_error = NULL;
    g_autoptr(GVariant) result = NULL;
    gboolean pending = FALSE;
    if (g_str_equal(method, "window.thumbnail") || g_str_equal(method, "location.authorize_app") ||
        g_str_has_prefix(method, "auth.") || g_str_has_prefix(method, "prompt."))
        g_variant_lookup(packet->payload, "client_id", "t", &client_id);
    guint64 input_request_id = 0;
    g_autoptr(GVariant) input_scope =
        g_variant_lookup_value(packet->payload, "input_request_id", NULL);
    if (input_scope &&
        (!g_variant_lookup(packet->payload, "input_request_id", "t", &input_request_id) ||
         !native_input_operation_allowed(control, input_request_id))) {
        g_set_error_literal(&operation_error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "input operation belongs to a retired decision");
    }
    if (operation_error) {
        /* Report a normal operation failure; late input is not a protocol fault. */
    } else if (g_str_equal(method, "command.run")) {
        guint64 operation_generation = 0;
        if (!g_variant_lookup(packet->payload, "runtime_generation", "t", &operation_generation) ||
            !operation_generation || operation_generation != control->runtime_generation) {
            g_set_error_literal(&operation_error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                                "command.run belongs to a stale runtime generation");
        } else if (control->runtime_worker_suspended || control->runtime_recovery_failed) {
            g_set_error_literal(&operation_error, G_IO_ERROR, G_IO_ERROR_NOT_CONNECTED,
                                "Lua runtime is unavailable for command execution");
        } else if (control->wayland_compositor &&
                   meta_wayland_session_lock_is_active(control->wayland_compositor)) {
            g_set_error_literal(&operation_error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                                "command.run is unavailable while the session is locked");
        } else {
            g_auto(GStrv) argv = gnoblin_control_command_run_argv(arguments, &operation_error);
            if (argv && gnoblin_control_queue_command("command", argv[0], argv, &operation_error)) {
                GVariantBuilder accepted;
                g_variant_builder_init(&accepted, G_VARIANT_TYPE_VARDICT);
                g_variant_builder_add(&accepted, "{sv}", "accepted", g_variant_new_boolean(TRUE));
                result = g_variant_ref_sink(g_variant_builder_end(&accepted));
            }
        }
    } else if (g_str_equal(method, "command.capture")) {
        guint64 operation_generation = 0;
        if (!g_variant_lookup(packet->payload, "runtime_generation", "t", &operation_generation) ||
            !operation_generation || operation_generation != control->runtime_generation) {
            g_set_error_literal(&operation_error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                                "command.capture belongs to a stale runtime generation");
        } else if (control->runtime_worker_suspended || control->runtime_recovery_failed) {
            g_set_error_literal(&operation_error, G_IO_ERROR, G_IO_ERROR_NOT_CONNECTED,
                                "Lua runtime is unavailable for command execution");
        } else if (control->wayland_compositor &&
                   meta_wayland_session_lock_is_active(control->wayland_compositor)) {
            g_set_error_literal(&operation_error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                                "command.capture is unavailable while the session is locked");
        }
    } else if (g_str_equal(method, "window.snap") || g_str_equal(method, "window.snap_context")) {
        guint64 operation_generation = 0;
        if (!g_variant_lookup(packet->payload, "runtime_generation", "t", &operation_generation) ||
            !operation_generation || operation_generation != control->runtime_generation) {
            g_set_error_literal(&operation_error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                                "snap operation belongs to a stale runtime generation");
        }
    }
    if (operation_error) {
        /* Return a normal operation failure; do not tear down the private channel. */
    } else if (g_str_equal(method, "command.run")) {
        /* The command was accepted by the asynchronous executor above. */
    } else if (g_str_equal(method, "command.capture")) {
        guint64* operation_key = g_new(guint64, 1);
        *operation_key = (guint64)operation_id;
        g_hash_table_add(control->runtime_operation_ids, operation_key);
        if (gnoblin_control_capture_begin(control, operation_id, arguments, &operation_error))
            return TRUE;
        g_hash_table_remove(control->runtime_operation_ids, &operation_key[0]);
    } else if (g_str_equal(method, "window.snap.offer")) {
        guint64 owner_generation = 0;
        if (!g_variant_lookup(packet->payload, "runtime_generation", "t", &owner_generation) ||
            !gnoblin_control_runtime_window_snap_offer(control, arguments, owner_generation,
                                                       &operation_error)) {
            if (!operation_error)
                g_set_error_literal(&operation_error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                                    "window.snap.offer requires the active runtime generation");
        } else {
            GVariantBuilder accepted;
            g_variant_builder_init(&accepted, G_VARIANT_TYPE_VARDICT);
            g_variant_builder_add(&accepted, "{sv}", "accepted", g_variant_new_boolean(TRUE));
            result = g_variant_ref_sink(g_variant_builder_end(&accepted));
        }
    } else if (g_str_equal(method, "window.restore_or_minimize")) {
        result = gnoblin_native_control_restore_or_minimize_window(control->display, arguments,
                                                                   &operation_error);
    } else if (g_str_equal(method, "window.snap")) {
        result = gnoblin_native_control_commit_snap_context(control->display, arguments,
                                                            &operation_error);
    } else if (g_str_equal(method, "window.focus") || g_str_equal(method, "window.snap_context") ||
               g_str_equal(method, "window.begin_move") ||
               g_str_equal(method, "window.begin_resize") ||
               g_str_equal(method, "input.text_target")) {
        guint64 menu_handle = 0;
        guint64 menu_generation = 0;
        gboolean has_menu_handle =
            g_variant_lookup(arguments, "_menu_context_handle", "t", &menu_handle);
        gboolean has_menu_generation =
            g_variant_lookup(arguments, "_menu_context_generation", "t", &menu_generation);
        if ((g_str_equal(method, "window.begin_move") ||
             g_str_equal(method, "window.begin_resize")) &&
            (has_menu_handle || has_menu_generation)) {
            GVariantBuilder clean;
            GVariantIter iter;
            const char* key;
            GVariant* value;
            g_variant_builder_init(&clean, G_VARIANT_TYPE_VARDICT);
            g_variant_iter_init(&iter, arguments);
            while (g_variant_iter_next(&iter, "{&sv}", &key, &value)) {
                g_autoptr(GVariant) field = value;
                if (g_str_equal(key, "_menu_context_handle") ||
                    g_str_equal(key, "_menu_context_generation"))
                    continue;
                g_variant_builder_add(&clean, "{sv}", key, field);
            }
            g_autoptr(GVariant) menu_arguments = g_variant_ref_sink(g_variant_builder_end(&clean));
            if (!has_menu_handle || !menu_handle || !has_menu_generation || !menu_generation) {
                g_set_error_literal(&operation_error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                                    "operation has malformed WM menu capability metadata");
            } else {
                result = gnoblin_native_control_begin_menu_window_grab(
                    control->display, method, menu_arguments, menu_handle, menu_generation, 0,
                    &operation_error);
            }
        } else {
            guint64 handle = 0;
            guint64 generation = 0;
            gint64 expires_at_us = 0;
            if (!g_variant_lookup(packet->payload, "focus_context_handle", "t", &handle) ||
                !handle ||
                !g_variant_lookup(packet->payload, "focus_context_generation", "t", &generation) ||
                !generation ||
                !g_variant_lookup(packet->payload, "focus_context_expires_at_us", "x",
                                  &expires_at_us) ||
                expires_at_us <= g_get_monotonic_time()) {
                g_set_error_literal(&operation_error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                                    "operation requires a live shortcut focus context");
            } else if (g_str_equal(method, "input.text_target")) {
                result = gnoblin_native_control_create_text_target(
                    control->display, handle, generation, expires_at_us, &operation_error);
            } else if (g_str_equal(method, "window.snap_context")) {
                result = gnoblin_native_control_create_snap_context(
                    control->display, handle, generation, expires_at_us, &operation_error);
            } else if (g_str_equal(method, "window.focus")) {
                result = gnoblin_native_control_focus_window(control->display, arguments, handle,
                                                             generation, &operation_error);
            } else {
                result = gnoblin_native_control_begin_window_grab(
                    control->display, method, arguments, handle, generation, &operation_error);
            }
        }
    } else if (g_str_equal(method, "console.toggle")) {
        if (g_variant_n_children(arguments) != 0) {
            g_set_error_literal(&operation_error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                                "console.toggle takes no arguments");
        } else if (gnoblin_native_control_toggle_console(control->display, &operation_error)) {
            GVariantBuilder toggled;
            g_variant_builder_init(&toggled, G_VARIANT_TYPE_VARDICT);
            g_variant_builder_add(&toggled, "{sv}", "toggled", g_variant_new_boolean(TRUE));
            result = g_variant_ref_sink(g_variant_builder_end(&toggled));
        }
    } else if (g_str_equal(method, "privacy.stop_sharing") ||
               g_str_equal(method, "privacy.stop_recording")) {
        if (g_variant_n_children(arguments) != 0) {
            g_set_error_literal(&operation_error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                                "privacy stop methods take no arguments");
        } else {
            result = gnoblin_control_stop_privacy_sessions(
                control, g_str_equal(method, "privacy.stop_recording"), &operation_error);
        }
    } else if (g_str_equal(method, "location.authorize_app")) {
        result =
            native_location_authorize_operation(control, arguments, client_id, &operation_error);
    } else if (g_str_has_prefix(method, "auth.")) {
        result =
            gnoblin_control_auth_operation(control, method, arguments, client_id, &operation_error);
    } else if (g_str_has_prefix(method, "prompt.")) {
        result = gnoblin_control_prompts_operation(control, method, arguments, client_id,
                                                   &operation_error);
    } else if (g_str_equal(method, "input.set_orientation_lock")) {
        result = native_set_orientation_lock(control, arguments, &operation_error);
    } else if (g_str_equal(method, "monitors.set_privacy_screen")) {
        result = native_set_monitor_privacy_screen(control, arguments, &operation_error);
    } else if (g_str_equal(method, "session.lock")) {
        result = gnoblin_native_control_request_session_lock(control->display, arguments,
                                                             &operation_error);
    } else if (g_str_equal(method, "session.logout")) {
        if (g_variant_n_children(arguments) != 0) {
            g_set_error_literal(&operation_error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                                "session.logout takes no arguments");
        } else {
            GVariantBuilder logout_result;
            g_variant_builder_init(&logout_result, G_VARIANT_TYPE_VARDICT);
            g_variant_builder_add(&logout_result, "{sv}", "accepted", g_variant_new_boolean(TRUE));
            result = g_variant_ref_sink(g_variant_builder_end(&logout_result));
        }
    } else if (g_str_equal(method, "session.suspend") || g_str_equal(method, "session.hibernate") ||
               g_str_equal(method, "session.reboot") || g_str_equal(method, "session.poweroff")) {
        if (g_variant_n_children(arguments) != 0) {
            g_set_error(&operation_error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                        "%s takes no arguments", method);
        } else {
            GVariantBuilder accepted;
            g_variant_builder_init(&accepted, G_VARIANT_TYPE_VARDICT);
            g_variant_builder_add(&accepted, "{sv}", "accepted", g_variant_new_boolean(TRUE));
            result = g_variant_ref_sink(g_variant_builder_end(&accepted));
        }
    } else if (g_str_equal(method, "config.restore_default")) {
        if (g_variant_n_children(arguments) != 0) {
            g_set_error_literal(&operation_error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                                "config.restore_default takes no arguments");
        } else {
            GVariantBuilder accepted;
            g_variant_builder_init(&accepted, G_VARIANT_TYPE_VARDICT);
            g_variant_builder_add(&accepted, "{sv}", "accepted", g_variant_new_boolean(TRUE));
            result = g_variant_ref_sink(g_variant_builder_end(&accepted));
        }
    } else if (g_str_equal(method, "input.insert_text")) {
        result = gnoblin_native_control_insert_text(control->display, arguments, &operation_error);
    } else if (g_str_equal(method, "shortcut.session.arm")) {
        const char* binding_id = NULL;
        const char* owner_id = NULL;
        const char* trigger = NULL;
        guint64 session_id = 0;
        guint32 modifiers = 0;
        gboolean capture_input = FALSE;
        if (!g_variant_lookup(arguments, "binding_id", "&s", &binding_id) ||
            !g_variant_lookup(arguments, "owner_id", "&s", &owner_id) ||
            !g_variant_lookup(arguments, "session_id", "t", &session_id) || !session_id ||
            !g_variant_lookup(arguments, "trigger", "&s", &trigger) ||
            !g_str_equal(trigger, "release") ||
            !g_variant_lookup(arguments, "modifiers", "u", &modifiers) || modifiers != 0 ||
            !g_variant_lookup(arguments, "capture_input", "b", &capture_input) || !capture_input) {
            g_set_error_literal(&operation_error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                                "shortcut.session.arm requires a bare Super release binding");
        } else {
            NativeDynamicShortcut* armed = gnoblin_control_arm_bare_super_shortcut(
                control, binding_id, owner_id, NULL, session_id, &operation_error);
            if (armed) {
                GVariantBuilder armed_result;
                g_variant_builder_init(&armed_result, G_VARIANT_TYPE_VARDICT);
                g_variant_builder_add(&armed_result, "{sv}", "armed", g_variant_new_boolean(TRUE));
                g_variant_builder_add(&armed_result, "{sv}", "session_id",
                                      g_variant_new_uint64(session_id));
                result = g_variant_ref_sink(g_variant_builder_end(&armed_result));
            }
        }
    } else if (g_str_equal(method, "shortcut.bind") || g_str_equal(method, "shortcut.unbind") ||
               g_str_equal(method, "shortcut.session.end")) {
        result = native_runtime_shortcut_operation(control, method, arguments, operation_id,
                                                   &operation_error);
    } else if (g_str_has_prefix(method, "launch.")) {
        result = gnoblin_native_control_dispatch_launch(control->display, method, arguments,
                                                        &operation_error);
    } else if (g_str_equal(method, "input.select") || g_str_equal(method, "input.select_source") ||
               g_str_equal(method, "shortcut.capture")) {
        guint64* operation_key = g_new(guint64, 1);
        *operation_key = (guint64)operation_id;
        g_hash_table_add(control->runtime_operation_ids, operation_key);
        if (g_str_equal(method, "input.select") || g_str_equal(method, "input.select_source")) {
            pending = gnoblin_native_control_select_input_source(
                control->display, arguments, operation_id, method, &operation_error);
        } else {
            pending = gnoblin_native_control_begin_shortcut_capture(control->display, arguments,
                                                                    operation_id, &operation_error);
        }
        if (pending)
            return TRUE;
        g_hash_table_remove(control->runtime_operation_ids, &operation_key[0]);
    } else if (g_str_equal(method, "window.thumbnail")) {
        pending = native_runtime_begin_thumbnail(control, operation_id, arguments, client_id,
                                                 &operation_error);
        if (pending)
            return TRUE;
    } else if (g_str_equal(method, "window.set_maximized")) {
        gboolean enabled = FALSE;
        const char* window_id = NULL;
        if (g_variant_lookup(arguments, "enabled", "b", &enabled) && enabled &&
            g_variant_lookup(arguments, "id", "&s", &window_id) && window_id) {
            MetaWindow* window = native_window_by_stable_id(control, window_id);
            if (window)
                result = meta_gnoblin_dispatch_native_api(control->display, method, arguments,
                                                          &operation_error);
            else
                g_set_error_literal(&operation_error, G_IO_ERROR, G_IO_ERROR_NOT_FOUND,
                                    "window.set_maximized target is unavailable");
            if (result)
                g_hash_table_remove(control->snap_restore_frames, window);
        } else {
            result = meta_gnoblin_dispatch_native_api(control->display, method, arguments,
                                                      &operation_error);
        }
    } else {
        result =
            meta_gnoblin_dispatch_native_api(control->display, method, arguments, &operation_error);
    }
    GVariantBuilder completion;
    g_variant_builder_init(&completion, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&completion, "{sv}", "operation_id", g_variant_new_int64(operation_id));
    g_variant_builder_add(&completion, "{sv}", "method", g_variant_new_string(method));
    g_variant_builder_add(&completion, "{sv}", "ok", g_variant_new_boolean(result != NULL));
    if (result)
        g_variant_builder_add(&completion, "{sv}", "value", result);
    else {
        GVariantBuilder error_record;
        g_variant_builder_init(&error_record, G_VARIANT_TYPE_VARDICT);
        g_variant_builder_add(
            &error_record, "{sv}", "code",
            g_variant_new_string(gnoblin_control_operation_error_code(operation_error)));
        g_variant_builder_add(&error_record, "{sv}", "message",
                              g_variant_new_string(operation_error ? operation_error->message
                                                                   : "native operation failed"));
        g_variant_builder_add(&completion, "{sv}", "error", g_variant_builder_end(&error_record));
    }
    g_autoptr(GVariant) payload = g_variant_ref_sink(g_variant_builder_end(&completion));
    if (!native_runtime_send(control, GNOBLIN_RUNTIME_PACKET_COMPLETION, (guint64)operation_id,
                             payload, error))
        return FALSE;

    g_autoptr(JsonNode) json_result = result ? json_from_variant(result) : NULL;
    gnoblin_control_dispatch_operation_completion_full(
        control, operation_id, method, result != NULL, json_result,
        gnoblin_control_operation_error_code(operation_error),
        operation_error ? operation_error->message : NULL, FALSE);
    return TRUE;
}

gboolean native_runtime_fd_ready(gint fd, GIOCondition condition, gpointer user_data) {
    GnoblinNativeControl* control = user_data;
    if (control->stopping) {
        control->runtime_read_source_id = 0;
        return G_SOURCE_REMOVE;
    }
    if (condition & (G_IO_HUP | G_IO_ERR | G_IO_NVAL)) {
        control->runtime_read_source_id = 0;
        if (!control->stopping)
            g_warning("gnoblin-native-control: supervisor closed the private runtime channel");
        native_runtime_abort(control);
        return G_SOURCE_REMOVE;
    }
    for (;;) {
        GnoblinRuntimePacket packet = {0};
        gboolean available = FALSE;
        g_autoptr(GError) error = NULL;
        if (!gnoblin_runtime_reader_receive(control->runtime_reader, fd, &packet, &available,
                                            &error)) {
            g_warning("gnoblin-native-control: private runtime read failed: %s", error->message);
            native_runtime_abort(control);
            control->runtime_read_source_id = 0;
            return G_SOURCE_REMOVE;
        }
        if (!available)
            return G_SOURCE_CONTINUE;
        gboolean handled = FALSE;
        if (packet.type == GNOBLIN_RUNTIME_PACKET_SESSION_STATE_CHANGED) {
            handled = native_runtime_handle_session_lifecycle_state(control, &packet, &error);
        } else if (packet.type == GNOBLIN_RUNTIME_PACKET_RECOVERY_FAILED) {
            if (!control->supervised_runtime || packet.request_id != 0 ||
                g_variant_n_children(packet.payload) != 0) {
                g_set_error_literal(&error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                                    "supervisor sent an invalid recovery failure signal");
            } else {
                gboolean status_changed = !control->runtime_recovery_failed;
                control->runtime_recovery_failed = TRUE;
                if (status_changed)
                    native_runtime_publish_status(control);
                handled = TRUE;
            }
        } else if (control->runtime_recovery_failed &&
                   packet.type != GNOBLIN_RUNTIME_PACKET_WORKER_RESUME) {
            /* A terminal notification means no runtime worker will service
             * further packets. Discard frames queued before recovery ended. */
            gnoblin_runtime_packet_clear(&packet);
            continue;
        } else if (control->runtime_worker_suspended &&
                   packet.type == GNOBLIN_RUNTIME_PACKET_WORKER_DISCONNECTED) {
            /* A replacement can die before Mutter receives its RESUME. The
             * host cannot tell whether RESUME reached us, so acknowledge a
             * repeated DISCONNECTED while already suspended. */
            if (packet.request_id != 0 || g_variant_n_children(packet.payload) != 0) {
                g_set_error_literal(&error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                                    "supervisor sent an invalid worker disconnect signal");
            } else {
                handled = native_runtime_send_worker_suspended(control, &error);
            }
        } else if (control->runtime_worker_suspended &&
                   packet.type != GNOBLIN_RUNTIME_PACKET_WORKER_RESUME) {
            /* Discard stale frames left by the worker that was just reaped. */
            gnoblin_runtime_packet_clear(&packet);
            continue;
        } else if (packet.type == GNOBLIN_RUNTIME_PACKET_WORKER_DISCONNECTED) {
            if (packet.request_id != 0 || g_variant_n_children(packet.payload) != 0) {
                g_set_error_literal(&error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                                    "supervisor sent an invalid worker disconnect signal");
            } else {
                native_runtime_suspend_worker(control);
                handled = !control->stopping;
            }
        } else if (packet.type == GNOBLIN_RUNTIME_PACKET_WORKER_RESUME) {
            if (!control->runtime_worker_suspended) {
                g_set_error_literal(&error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                                    "worker sent RESUME while no worker was suspended");
            } else if (packet.request_id != 0 ||
                       !gnoblin_control_runtime_resume_snapshot_matches(control, packet.payload)) {
                handled = gnoblin_control_runtime_reject_resume(
                    control,
                    "replacement worker configuration does not match Mutter's accepted state",
                    &error);
            } else {
                /* Also discard any gesture that began while the worker was
                 * absent, so it cannot cross into the resumed generation. */
                gnoblin_touchpad_router_reset(control->touchpad_router);
                control->runtime_recovery_failed = FALSE;
                control->runtime_worker_suspended = FALSE;
                GVariantBuilder hello;
                g_variant_builder_init(&hello, G_VARIANT_TYPE_VARDICT);
                g_variant_builder_add(&hello, "{sv}", "role", g_variant_new_string("compositor"));
                g_variant_builder_add(&hello, "{sv}", "document_version", g_variant_new_uint32(1));
                if (control->session_lifecycle_supported)
                    g_variant_builder_add(&hello, "{sv}", "session_lifecycle_version",
                                          g_variant_new_uint32(1));
                gnoblin_control_runtime_add_display_environment(&hello);
                g_autoptr(GVariant) hello_payload =
                    g_variant_ref_sink(g_variant_builder_end(&hello));
                handled = native_runtime_send(control, GNOBLIN_RUNTIME_PACKET_HELLO, 0,
                                              hello_payload, &error);
                if (handled) {
                    control->runtime_hello_sent = TRUE;
                    handled = native_runtime_republish_full_state(control, &error);
                    if (handled)
                        native_runtime_publish_status(control);
                }
            }
        } else if (packet.type == GNOBLIN_RUNTIME_PACKET_INPUT_DECISION) {
            handled = native_input_handle_decision(control, &packet, &error);
        } else if (packet.type == GNOBLIN_RUNTIME_PACKET_OPERATION) {
            handled = native_runtime_handle_operation(control, &packet, &error);
        } else if (packet.type == GNOBLIN_RUNTIME_PACKET_API_RESPONSE && packet.request_id > 0) {
            PendingRuntimeRequest* pending =
                g_hash_table_lookup(control->pending_runtime_requests, &packet.request_id);
            gboolean ok = FALSE;
            const char* message = NULL;
            g_autoptr(GVariant) result = g_variant_lookup_value(packet.payload, "result", NULL);
            if (!pending || !g_variant_lookup(packet.payload, "ok", "b", &ok) || (ok && !result)) {
                g_set_error_literal(&error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                                    "supervisor sent an invalid API response");
            } else if (pending->discard_response) {
                /* The UI owner went away, but the supervisor still owes this
                 * response. Keep the request tracked until it arrives. */
                g_hash_table_remove(control->pending_runtime_requests, &packet.request_id);
                handled = TRUE;
            } else {
                gpointer pending_key = NULL;
                gpointer pending_value = NULL;
                if (!g_hash_table_lookup_extended(control->pending_runtime_requests,
                                                  &packet.request_id, &pending_key,
                                                  &pending_value)) {
                    g_set_error_literal(&error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                                        "runtime API response lost its pending request");
                    goto api_response_done;
                }
                g_hash_table_steal(control->pending_runtime_requests, &packet.request_id);
                g_free(pending_key);
                pending = pending_value;
                if (ok && pending->legacy_window_action) {
                    if (!pending->legacy_window_id ||
                        !g_variant_is_of_type(result, G_VARIANT_TYPE_INT64) ||
                        g_variant_get_int64(result) <= 0) {
                        ok = FALSE;
                        message = "Lua returned an invalid window action operation";
                    } else {
                        GVariantBuilder legacy_result;
                        g_variant_builder_init(&legacy_result, G_VARIANT_TYPE_VARDICT);
                        g_variant_builder_add(&legacy_result, "{sv}", "ok",
                                              g_variant_new_boolean(TRUE));
                        g_variant_builder_add(&legacy_result, "{sv}", "pending",
                                              g_variant_new_boolean(TRUE));
                        g_variant_builder_add(&legacy_result, "{sv}", "window",
                                              g_variant_new_string(pending->legacy_window_id));
                        g_variant_builder_add(&legacy_result, "{sv}", "action",
                                              g_variant_new_string(pending->legacy_window_action));
                        g_clear_pointer(&result, g_variant_unref);
                        result = g_variant_ref_sink(g_variant_builder_end(&legacy_result));
                    }
                }
                if (!ok && !message)
                    g_variant_lookup(packet.payload, "error", "&s", &message);
                if (pending->config_reload)
                    gnoblin_control_post_notice(
                        ok ? GNOBLIN_NATIVE_NOTICE_CONFIG_RELOADED
                           : GNOBLIN_NATIVE_NOTICE_CONFIG_FAILED,
                        ok ? NULL : (message ? message : "runtime API request failed"));
                if (pending->callback) {
                    /* The packet and result are owned by the runtime reader.
                     * The callback is synchronous on the compositor main
                     * loop, so borrowing the result is safe until it returns. */
                    pending->callback(
                        ok, result, ok ? NULL : (message ? message : "runtime API request failed"),
                        pending->callback_data);
                } else {
                    Client* client = pending->client;
                    if (!client->closing) {
                        g_autoptr(JsonNode) json = result ? json_from_variant(result) : NULL;
                        send_response(
                            client,
                            encode_response(
                                pending->request_id, json,
                                ok ? NULL : (message ? message : "runtime API request failed")));
                        client->pending_deferred_requests--;
                        process_buffer(client);
                    } else if (client->pending_deferred_requests > 0) {
                        client->pending_deferred_requests--;
                    }
                    client_maybe_free(client);
                }
                pending_runtime_request_free(pending);
                handled = TRUE;
            }
        api_response_done:;
        } else if (packet.type == GNOBLIN_RUNTIME_PACKET_CONFIG) {
            GVariant* document =
                g_variant_lookup_value(packet.payload, "document", G_VARIANT_TYPE_VARDICT);
            guint32 version = 0;
            guint64 revision = 0;
            guint64 runtime_generation = 0;
            guint64 previous_revision =
                gnoblin_runtime_cache_get_settings_revision(control->runtime_cache);
            guint64 previous_runtime_generation = control->runtime_generation;
            gboolean runtime_generation_changed = FALSE;
            g_autoptr(GVariant) previous_document =
                gnoblin_runtime_cache_get_document(control->runtime_cache);
            g_autoptr(GVariant) previous_bootstrap_document =
                bootstrap_runtime_cache
                    ? gnoblin_runtime_cache_get_document(bootstrap_runtime_cache)
                    : NULL;
            guint64 previous_bootstrap_revision =
                bootstrap_runtime_cache
                    ? gnoblin_runtime_cache_get_settings_revision(bootstrap_runtime_cache)
                    : 0;
            g_autoptr(GError) config_error = NULL;
            g_autoptr(GHashTable) next_event_subscriptions =
                native_runtime_event_subscriptions_from_payload(packet.payload, &config_error);
            gboolean fields_valid =
                document && next_event_subscriptions &&
                g_variant_lookup(packet.payload, "document_version", "u", &version) &&
                version == 1 &&
                g_variant_lookup(packet.payload, "settings_revision", "t", &revision) &&
                g_variant_lookup(packet.payload, "runtime_generation", "t", &runtime_generation) &&
                runtime_generation > 0;
            gboolean candidate_valid =
                fields_valid && revision >= previous_revision &&
                runtime_generation >= control->runtime_generation &&
                (packet.request_id == 0 ? runtime_generation == control->runtime_generation
                                        : runtime_generation > control->runtime_generation);
            gboolean rollback_failed = FALSE;
            if (candidate_valid) {
                g_autoptr(GVariant) old_workspaces =
                    previous_document
                        ? g_variant_lookup_value(previous_document, "workspaces", NULL)
                        : NULL;
                g_autoptr(GVariant) new_workspaces =
                    g_variant_lookup_value(document, "workspaces", NULL);
                gboolean workspaces_changed =
                    (!old_workspaces != !new_workspaces) ||
                    (old_workspaces && new_workspaces &&
                     !runtime_config_values_equal(old_workspaces, new_workspaces));
                g_autoptr(GVariant) old_input_sources =
                    previous_document
                        ? g_variant_lookup_value(previous_document, "input-sources", NULL)
                        : NULL;
                g_autoptr(GVariant) new_input_sources =
                    g_variant_lookup_value(document, "input-sources", NULL);
                gboolean input_sources_changed =
                    (!old_input_sources != !new_input_sources) ||
                    (old_input_sources && new_input_sources &&
                     !runtime_config_values_equal(old_input_sources, new_input_sources));
                if (revision == previous_revision && previous_document &&
                    !runtime_config_values_equal(previous_document, document)) {
                    g_set_error_literal(
                        &config_error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                        "supervisor changed configuration without advancing its revision");
                } else {
                    handled = gnoblin_runtime_cache_replace(control->runtime_cache, document,
                                                            revision, &config_error);
                }
                if (handled && revision > previous_revision) {
                    MetaContext* context = meta_backend_get_context(control->backend);
                    gboolean supported =
                        gnoblin_control_runtime_reload_documents_valid(previous_document, document);
                    gboolean apply_ok = supported;
                    if (!supported)
                        g_set_error_literal(
                            &config_error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                            "configuration transaction has no valid active snapshot");
                    /* Protocol advertisement reads the bootstrap cache, so update it before the
                     * apply steps run. Otherwise each reload applies the previous protocols. */
                    else if (bootstrap_runtime_cache &&
                             !gnoblin_runtime_cache_replace(bootstrap_runtime_cache, document,
                                                            revision, &config_error))
                        apply_ok = FALSE;
                    else if (!apply_native_input(control, context, document, &config_error))
                        apply_ok = FALSE;
                    else if (workspaces_changed && !meta_gnoblin_initialize_native_workspaces(
                                                       control->display, &config_error))
                        apply_ok = FALSE;
                    else if (!apply_native_keybindings(control, document, &config_error))
                        apply_ok = FALSE;
                    else if (!replace_native_shortcuts(control, document, &config_error))
                        apply_ok = FALSE;
                    else {
                        g_autoptr(GVariant) renderers = g_variant_lookup_value(
                            document, "frame-renderers", G_VARIANT_TYPE_VARDICT);
                        if (!meta_gnoblin_frame_renderers_configure(renderers, FALSE,
                                                                    &config_error))
                            apply_ok = FALSE;
                        else if (!meta_gnoblin_apply_protocol_configuration(
                                     control->wayland_compositor, &config_error))
                            apply_ok = FALSE;
                    }
                    if (!apply_ok) {
                        gnoblin_runtime_cache_replace(control->runtime_cache, previous_document,
                                                      previous_revision, NULL);
                        if (bootstrap_runtime_cache && previous_bootstrap_document)
                            gnoblin_runtime_cache_replace(bootstrap_runtime_cache,
                                                          previous_bootstrap_document,
                                                          previous_bootstrap_revision, NULL);
                        if (supported && previous_document) {
                            g_autoptr(GError) rollback_error = NULL;
                            if (!apply_native_input(control, context, previous_document,
                                                    &rollback_error))
                                rollback_failed = TRUE;
                            if (!apply_native_keybindings(control, previous_document,
                                                          &rollback_error))
                                rollback_failed = TRUE;
                            if (!replace_native_shortcuts(control, previous_document,
                                                          &rollback_error))
                                rollback_failed = TRUE;
                            g_autoptr(GVariant) previous_renderers = g_variant_lookup_value(
                                previous_document, "frame-renderers", G_VARIANT_TYPE_VARDICT);
                            if (!meta_gnoblin_frame_renderers_configure(previous_renderers, FALSE,
                                                                        &rollback_error))
                                rollback_failed = TRUE;
                            if (!meta_gnoblin_apply_protocol_configuration(
                                    control->wayland_compositor, &rollback_error))
                                rollback_failed = TRUE;
                            if (workspaces_changed && !meta_gnoblin_initialize_native_workspaces(
                                                          control->display, &rollback_error))
                                rollback_failed = TRUE;
                            native_settings_changed(previous_revision, control);
                        }
                        handled = FALSE;
                    }
                }
                if (handled && runtime_generation > previous_runtime_generation) {
                    native_input_cancel_pending(control, !(control->wayland_compositor &&
                                                           meta_wayland_session_lock_is_active(
                                                               control->wayland_compositor)));
                    /* A reload may replace only the Lua worker, leaving the
                     * settings revision unchanged. Do not retain a keyboard
                     * capture whose owner belongs to the old generation. */
                    if (control->active_shortcut_session)
                        dynamic_shortcut_end_session(control, control->active_shortcut_session,
                                                     "config_reloaded");
                    stop_native_shortcut_capture(
                        control, TRUE, FALSE, NULL, "reloaded",
                        "shortcut capture cancelled because configuration reloaded");
                    control->input_policy_bypassed = FALSE;
                    g_clear_pointer(&control->input_policy_error, g_free);
                    gnoblin_control_cancel_window_drags(control, "config_reloaded");
                    revoke_focus_contexts(control);
                }
                if (handled && revision > previous_revision) {
                    native_settings_changed(revision, control);
#ifdef HAVE_XWAYLAND
                    if (gnoblin_control_xwayland_server_flags_changed(previous_document, document))
                        meta_xwayland_restart_for_config(
                            &control->wayland_compositor->xwayland_manager);
#endif
                    if (input_sources_changed) {
                        gnoblin_control_publish_input_source_changes(control,
                                                                     control->state_revision);
                        gnoblin_control_input_source_restore_focused(control);
                        /* The startup selection runs against the bootstrap config, which has no
                         * input sources, so the list from the first accepted config arrives with
                         * nothing current. The same happens when a reload drops the current source.
                         * Make the first configured source current. */
                        if (control->input_sources && control->input_sources->len > 0 &&
                            !gnoblin_control_current_input_source(control) &&
                            control->pending_input_source_ops == 0) {
                            NativeInputSource* first = g_ptr_array_index(control->input_sources, 0);
                            g_autoptr(GError) select_error = NULL;
                            if (!gnoblin_control_select_xkb_source(control, first->id, 0,
                                                                   "config.input_sources", FALSE,
                                                                   &select_error))
                                g_warning("gnoblin-native-control: could not select the first "
                                          "input source %s: %s",
                                          first->id,
                                          select_error ? select_error->message : "unknown error");
                        }
                        schedule_windows(control);
                    }
                }
                if (handled) {
                    control->runtime_generation = runtime_generation;
                    runtime_generation_changed = runtime_generation > previous_runtime_generation;
                }
            } else if (fields_valid) {
                g_set_error_literal(&config_error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                                    "configuration revision or runtime generation is stale");
            } else if (!config_error) {
                g_set_error_literal(&config_error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                                    "supervisor sent an invalid configuration snapshot");
            }
            if (handled) {
                gboolean events_changed =
                    !control->runtime_event_subscriptions || !next_event_subscriptions ||
                    !gnoblin_control_runtime_event_subscriptions_equal(
                        control->runtime_event_subscriptions, next_event_subscriptions);
                if (events_changed) {
                    g_clear_pointer(&control->runtime_event_subscriptions, g_hash_table_unref);
                    control->runtime_event_subscriptions =
                        g_steal_pointer(&next_event_subscriptions);
                    refresh_display_signal_watches(control);
                    refresh_all_window_signal_watches(control);
                    gnoblin_control_refresh_all_workspace_signal_watches(control);
                    gnoblin_control_refresh_workspace_manager_signal_watches(control);
                    refresh_backend_signal_watches(control);
                    gnoblin_control_refresh_monitor_manager_signal_watches(control);
                    refresh_cursor_tracker_signal_watches(control);
                }
            }
            g_clear_pointer(&document, g_variant_unref);
            if (packet.request_id > 0) {
                const char* message = config_error ? config_error->message : NULL;
                handled = gnoblin_control_runtime_send_config_result(
                    control, packet.request_id, handled, revision, runtime_generation, message,
                    &error);
            } else if (config_error) {
                g_propagate_error(&error, g_steal_pointer(&config_error));
            }
            if (handled && !error && runtime_generation_changed)
                native_runtime_publish_status(control);
            if (rollback_failed && !error)
                g_set_error_literal(
                    &error, G_IO_ERROR, G_IO_ERROR_FAILED,
                    "Mutter could not restore the previous configuration after rejection");
        } else if (packet.type == GNOBLIN_RUNTIME_PACKET_SHUTDOWN) {
            handled = TRUE;
            native_runtime_abort(control);
        } else {
            g_set_error_literal(&error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                                "unexpected packet from Gnoblin supervisor");
        }
        gnoblin_runtime_packet_clear(&packet);
        if (error || !handled) {
            g_warning("gnoblin-native-control: private runtime packet rejected: %s",
                      error ? error->message : "packet was not handled");
            native_runtime_abort(control);
            control->runtime_read_source_id = 0;
            return G_SOURCE_REMOVE;
        }
        if (control->stopping) {
            control->runtime_read_source_id = 0;
            return G_SOURCE_REMOVE;
        }
    }
}
