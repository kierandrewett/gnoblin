#include "gnoblin-native-control-private.h"

GVariant* capability_snapshot_record(GnoblinNativeControl* control,
                                     const NativeCapability* native_capability) {
    GVariantBuilder capability;
    gboolean available = TRUE;
    const char* unavailable_reason = NULL;

    if (g_str_equal(native_capability->id, "microphone-monitor")) {
#ifdef HAVE_REMOTE_DESKTOP
        available = control->privacy_microphone_available;
        if (!available)
            unavailable_reason = "pipewire_unavailable";
#else
        available = FALSE;
        unavailable_reason = "remote_desktop_disabled";
#endif
    } else if (g_str_equal(native_capability->id, "camera-monitor")) {
#ifdef HAVE_REMOTE_DESKTOP
        available = control->privacy_camera_available;
        if (!available)
            unavailable_reason = "pipewire_unavailable";
#else
        available = FALSE;
        unavailable_reason = "remote_desktop_disabled";
#endif
    } else if (g_str_equal(native_capability->id, "location-agent")) {
        available = control->privacy_location_available;
        if (!available)
            unavailable_reason = "geoclue_unavailable";
    } else if (g_str_equal(native_capability->id, "auth-agent")) {
        available = gnoblin_control_auth_available(control);
        unavailable_reason = gnoblin_control_auth_unavailable_reason(control);
    } else if (g_str_equal(native_capability->id, "prompt-broker")) {
        available = gnoblin_control_prompts_available(control);
        unavailable_reason = gnoblin_control_prompts_unavailable_reason(control);
    }

    g_variant_builder_init(&capability, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&capability, "{sv}", "id", g_variant_new_string(native_capability->id));
    g_variant_builder_add(&capability, "{sv}", "description",
                          g_variant_new_string(native_capability->description));
    g_variant_builder_add(&capability, "{sv}", "available", g_variant_new_boolean(available));
    if (unavailable_reason)
        g_variant_builder_add(&capability, "{sv}", "reason",
                              g_variant_new_string(unavailable_reason));
    g_variant_builder_add(&capability, "{sv}", "revision",
                          g_variant_new_int64(control->state_revision));
    return g_variant_ref_sink(g_variant_builder_end(&capability));
}

GVariant* capability_snapshot(GnoblinNativeControl* control) {
    GVariantBuilder capabilities;
    GVariantBuilder snapshot;
    g_variant_builder_init(&capabilities, G_VARIANT_TYPE("av"));
    for (guint i = 0; i < native_capabilities_count; i++) {
        g_autoptr(GVariant) capability =
            capability_snapshot_record(control, &native_capabilities[i]);
        g_variant_builder_add_value(&capabilities, g_variant_new_variant(capability));
    }
    g_variant_builder_init(&snapshot, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&snapshot, "{sv}", "capabilities", g_variant_builder_end(&capabilities));
    g_variant_builder_add(&snapshot, "{sv}", "revision",
                          g_variant_new_int64(control->state_revision));
    return g_variant_ref_sink(g_variant_builder_end(&snapshot));
}

const NativeCapability* native_capability_by_id(const char* id) {
    for (guint i = 0; i < native_capabilities_count; i++) {
        if (g_str_equal(native_capabilities[i].id, id))
            return &native_capabilities[i];
    }
    return NULL;
}

static const char* input_device_type_name(ClutterInputDeviceType type) {
    switch (type) {
    case CLUTTER_POINTER_DEVICE:
        return "pointer";
    case CLUTTER_KEYBOARD_DEVICE:
        return "keyboard";
    case CLUTTER_EXTENSION_DEVICE:
        return "extension";
    case CLUTTER_JOYSTICK_DEVICE:
        return "joystick";
    case CLUTTER_TABLET_DEVICE:
        return "tablet";
    case CLUTTER_TOUCHPAD_DEVICE:
        return "touchpad";
    case CLUTTER_TOUCHSCREEN_DEVICE:
        return "touchscreen";
    case CLUTTER_PEN_DEVICE:
        return "pen";
    case CLUTTER_ERASER_DEVICE:
        return "eraser";
    case CLUTTER_CURSOR_DEVICE:
        return "cursor";
    case CLUTTER_PAD_DEVICE:
        return "pad";
    case CLUTTER_N_DEVICE_TYPES:
        break;
    }
    return "unknown";
}

static void add_input_device_capability(GVariantBuilder* capabilities,
                                        ClutterInputCapabilities available,
                                        ClutterInputCapabilities capability, const char* name) {
    if (available & capability)
        g_variant_builder_add(capabilities, "s", name);
}

static const char* native_input_device_id(GnoblinNativeControl* control,
                                          ClutterInputDevice* device) {
    const char* id = g_hash_table_lookup(control->input_device_ids, device);
    if (!id) {
        char* owned_id =
            g_strdup_printf("input:%" G_GUINT64_FORMAT, ++control->next_input_device_id);
        g_hash_table_insert(control->input_device_ids, device, owned_id);
        id = owned_id;
    }
    return id;
}

GVariant* input_device_record(GnoblinNativeControl* control, ClutterInputDevice* device) {
    GVariantBuilder record;
    GVariantBuilder capabilities;
    ClutterInputCapabilities available = clutter_input_device_get_capabilities(device);
    const char* name = clutter_input_device_get_device_name(device);
    ClutterSeat* seat = clutter_input_device_get_seat(device);
    guint vendor_id = clutter_input_device_get_vendor_id(device);
    guint product_id = clutter_input_device_get_product_id(device);

    g_variant_builder_init(&record, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&record, "{sv}", "id",
                          g_variant_new_string(native_input_device_id(control, device)));
    g_variant_builder_add(&record, "{sv}", "name", g_variant_new_string(name ? name : ""));
    g_variant_builder_add(
        &record, "{sv}", "device_type",
        g_variant_new_string(input_device_type_name(clutter_input_device_get_device_type(device))));
    if (seat && clutter_seat_get_name(seat))
        g_variant_builder_add(&record, "{sv}", "seat",
                              g_variant_new_string(clutter_seat_get_name(seat)));
    if (vendor_id)
        g_variant_builder_add(&record, "{sv}", "vendor_id", g_variant_new_uint32(vendor_id));
    if (product_id)
        g_variant_builder_add(&record, "{sv}", "product_id", g_variant_new_uint32(product_id));

    g_variant_builder_init(&capabilities, G_VARIANT_TYPE("as"));
    add_input_device_capability(&capabilities, available, CLUTTER_INPUT_CAPABILITY_POINTER,
                                "pointer");
    add_input_device_capability(&capabilities, available, CLUTTER_INPUT_CAPABILITY_KEYBOARD,
                                "keyboard");
    add_input_device_capability(&capabilities, available, CLUTTER_INPUT_CAPABILITY_TOUCHPAD,
                                "touchpad");
    add_input_device_capability(&capabilities, available, CLUTTER_INPUT_CAPABILITY_TOUCH, "touch");
    add_input_device_capability(&capabilities, available, CLUTTER_INPUT_CAPABILITY_TABLET_TOOL,
                                "tablet_tool");
    add_input_device_capability(&capabilities, available, CLUTTER_INPUT_CAPABILITY_TABLET_PAD,
                                "tablet_pad");
    add_input_device_capability(&capabilities, available, CLUTTER_INPUT_CAPABILITY_TRACKBALL,
                                "trackball");
    add_input_device_capability(&capabilities, available, CLUTTER_INPUT_CAPABILITY_TRACKPOINT,
                                "trackpoint");
    g_variant_builder_add(&record, "{sv}", "capabilities", g_variant_builder_end(&capabilities));
    g_variant_builder_add(&record, "{sv}", "revision",
                          g_variant_new_int64(control->state_revision));
    return g_variant_ref_sink(g_variant_builder_end(&record));
}

static const char* native_input_event_type(const ClutterEvent* event) {
    switch (clutter_event_type(event)) {
    case CLUTTER_KEY_PRESS:
    case CLUTTER_KEY_RELEASE:
        return "key";
    case CLUTTER_BUTTON_PRESS:
    case CLUTTER_BUTTON_RELEASE:
        return "button";
    case CLUTTER_MOTION:
        return "motion";
    case CLUTTER_SCROLL:
        return "scroll";
    case CLUTTER_TOUCH_BEGIN:
    case CLUTTER_TOUCH_UPDATE:
    case CLUTTER_TOUCH_END:
    case CLUTTER_TOUCH_CANCEL:
        return "touch";
    case CLUTTER_PAD_BUTTON_PRESS:
    case CLUTTER_PAD_BUTTON_RELEASE:
    case CLUTTER_PAD_RING:
    case CLUTTER_PAD_STRIP:
    case CLUTTER_PAD_DIAL:
        return "tablet";
    case CLUTTER_TOUCHPAD_SWIPE:
    case CLUTTER_TOUCHPAD_PINCH:
    case CLUTTER_TOUCHPAD_HOLD:
        return "gesture";
    default:
        return NULL;
    }
}

static const char* native_input_event_phase(const ClutterEvent* event) {
    switch (clutter_event_type(event)) {
    case CLUTTER_KEY_PRESS:
    case CLUTTER_BUTTON_PRESS:
    case CLUTTER_PAD_BUTTON_PRESS:
        return "press";
    case CLUTTER_KEY_RELEASE:
    case CLUTTER_BUTTON_RELEASE:
    case CLUTTER_PAD_BUTTON_RELEASE:
        return "release";
    case CLUTTER_TOUCH_BEGIN:
        return "begin";
    case CLUTTER_TOUCH_UPDATE:
    case CLUTTER_MOTION:
    case CLUTTER_SCROLL:
    case CLUTTER_PAD_RING:
    case CLUTTER_PAD_STRIP:
    case CLUTTER_PAD_DIAL:
        return "update";
    case CLUTTER_TOUCH_END:
        return "end";
    case CLUTTER_TOUCH_CANCEL:
        return "cancel";
    case CLUTTER_TOUCHPAD_SWIPE:
    case CLUTTER_TOUCHPAD_PINCH:
    case CLUTTER_TOUCHPAD_HOLD:
        switch (clutter_event_get_gesture_phase(event)) {
        case CLUTTER_TOUCHPAD_GESTURE_PHASE_BEGIN:
            return "begin";
        case CLUTTER_TOUCHPAD_GESTURE_PHASE_UPDATE:
            return "update";
        case CLUTTER_TOUCHPAD_GESTURE_PHASE_END:
            return "end";
        case CLUTTER_TOUCHPAD_GESTURE_PHASE_CANCEL:
            return "cancel";
        default:
            return NULL;
        }
    default:
        return NULL;
    }
}

static gboolean native_input_selector_uint32(GVariant* match, const char* key, guint32* value,
                                             gboolean* present) {
    g_autoptr(GVariant) candidate = g_variant_lookup_value(match, key, NULL);
    *present = candidate != NULL;
    if (!candidate)
        return TRUE;
    if (g_variant_is_of_type(candidate, G_VARIANT_TYPE_UINT32)) {
        *value = g_variant_get_uint32(candidate);
        return TRUE;
    }
    if (g_variant_is_of_type(candidate, G_VARIANT_TYPE_INT64)) {
        gint64 signed_value = g_variant_get_int64(candidate);
        if (signed_value >= 0 && signed_value <= G_MAXUINT32) {
            *value = (guint32)signed_value;
            return TRUE;
        }
    }
    return FALSE;
}

static gboolean native_input_match(GnoblinNativeControl* control, GVariant* match, const char* type,
                                   const char* device_id, const char* phase,
                                   const ClutterEvent* event) {
    const char *wanted_type = NULL, *wanted_device = NULL, *wanted_phase = NULL;
    guint32 value = 0;
    gboolean present = FALSE;
    if (!match || !g_variant_is_of_type(match, G_VARIANT_TYPE_VARDICT) ||
        !g_variant_lookup(match, "type", "&s", &wanted_type) || !g_str_equal(wanted_type, type))
        return FALSE;
    if (g_variant_lookup(match, "device-id", "&s", &wanted_device) &&
        !g_str_equal(wanted_device, device_id))
        return FALSE;
    if (g_variant_lookup(match, "phase", "&s", &wanted_phase) &&
        g_strcmp0(wanted_phase, phase) != 0)
        return FALSE;
    if (!native_input_selector_uint32(match, "modifiers", &value, &present) ||
        (present && value != clutter_event_get_state(event)))
        return FALSE;
    if (!native_input_selector_uint32(match, "keycode", &value, &present) ||
        (present && value != clutter_event_get_key_code(event)))
        return FALSE;
    if (!native_input_selector_uint32(match, "button", &value, &present) ||
        (present && value != clutter_event_get_button(event)))
        return FALSE;
    const char* accelerator = NULL;
    if (g_variant_lookup(match, "accelerator", "&s", &accelerator) &&
        (!accelerator || !control ||
         !meta_keybindings_gnoblin_event_matches_accelerator(control->display, event, accelerator)))
        return FALSE;
    return TRUE;
}

static void native_input_clear_requests(GnoblinNativeControl* control) {
    if (control->accepted_input_requests)
        g_hash_table_remove_all(control->accepted_input_requests);
}

static void native_input_accept_request(GnoblinNativeControl* control, guint64 id,
                                        guint64 generation) {
    if (!control->accepted_input_requests)
        control->accepted_input_requests =
            g_hash_table_new_full(g_int64_hash, g_int64_equal, g_free, g_free);
    gint64 now = g_get_monotonic_time();
    GHashTableIter iter;
    gpointer value;
    g_hash_table_iter_init(&iter, control->accepted_input_requests);
    while (g_hash_table_iter_next(&iter, NULL, &value)) {
        NativeAcceptedInputRequest* accepted = value;
        if (accepted->expires_at_us <= now || accepted->generation != generation)
            g_hash_table_iter_remove(&iter);
    }
    /* Decisions carry at most 64 operations. Keep their authority bounded
     * independently of the event queue and retire it on lifecycle changes. */
    if (g_hash_table_size(control->accepted_input_requests) >= 256)
        native_input_clear_requests(control);
    guint64* key = g_new(guint64, 1);
    *key = id;
    NativeAcceptedInputRequest* accepted = g_new0(NativeAcceptedInputRequest, 1);
    accepted->generation = generation;
    accepted->expires_at_us = now + 100 * G_TIME_SPAN_MILLISECOND;
    g_hash_table_replace(control->accepted_input_requests, key, accepted);
}

gboolean native_input_operation_allowed(GnoblinNativeControl* control, guint64 id) {
    NativeAcceptedInputRequest* accepted =
        control->accepted_input_requests
            ? g_hash_table_lookup(control->accepted_input_requests, &id)
            : NULL;
    return id && accepted && accepted->generation == native_config_generation(control) &&
           accepted->expires_at_us > g_get_monotonic_time() && !control->input_policy_bypassed &&
           !control->runtime_worker_suspended && !control->runtime_recovery_failed &&
           !(control->wayland_compositor &&
             meta_wayland_session_lock_is_active(control->wayland_compositor));
}

static char* native_input_held_key(GnoblinNativeControl* control, const ClutterEvent* event) {
    ClutterInputDevice* device = clutter_event_get_source_device(event);
    const char* id = device ? native_input_device_id(control, device) : "";
    ClutterEventType type = clutter_event_type(event);
    if (type == CLUTTER_KEY_PRESS || type == CLUTTER_KEY_RELEASE)
        return g_strdup_printf("key:%s:%u", id, clutter_event_get_key_code(event));
    if (type == CLUTTER_BUTTON_PRESS || type == CLUTTER_BUTTON_RELEASE ||
        type == CLUTTER_PAD_BUTTON_PRESS || type == CLUTTER_PAD_BUTTON_RELEASE)
        return g_strdup_printf("button:%s:%u", id, clutter_event_get_button(event));
    if (type == CLUTTER_TOUCH_BEGIN || type == CLUTTER_TOUCH_UPDATE || type == CLUTTER_TOUCH_END ||
        type == CLUTTER_TOUCH_CANCEL) {
        ClutterEventSequence* sequence = clutter_event_get_event_sequence(event);
        return sequence
                   ? g_strdup_printf("touch:%s:%d", id, clutter_event_sequence_get_slot(sequence))
                   : NULL;
    }
    if (type == CLUTTER_TOUCHPAD_SWIPE || type == CLUTTER_TOUCHPAD_PINCH ||
        type == CLUTTER_TOUCHPAD_HOLD)
        return g_strdup_printf("gesture:%s:%u", id, (guint)type);
    return NULL;
}

static gboolean native_input_is_terminal(const ClutterEvent* event) {
    ClutterEventType type = clutter_event_type(event);
    if (type == CLUTTER_KEY_RELEASE || type == CLUTTER_BUTTON_RELEASE ||
        type == CLUTTER_PAD_BUTTON_RELEASE || type == CLUTTER_TOUCH_END ||
        type == CLUTTER_TOUCH_CANCEL)
        return TRUE;
    if (type == CLUTTER_TOUCHPAD_SWIPE || type == CLUTTER_TOUCHPAD_PINCH ||
        type == CLUTTER_TOUCHPAD_HOLD) {
        ClutterTouchpadGesturePhase phase = clutter_event_get_gesture_phase(event);
        return phase == CLUTTER_TOUCHPAD_GESTURE_PHASE_END ||
               phase == CLUTTER_TOUCHPAD_GESTURE_PHASE_CANCEL;
    }
    return FALSE;
}

static gboolean native_input_is_stream_begin(const ClutterEvent* event) {
    ClutterEventType type = clutter_event_type(event);
    if (type == CLUTTER_KEY_PRESS || type == CLUTTER_BUTTON_PRESS ||
        type == CLUTTER_PAD_BUTTON_PRESS || type == CLUTTER_TOUCH_BEGIN)
        return TRUE;
    return (type == CLUTTER_TOUCHPAD_SWIPE || type == CLUTTER_TOUCHPAD_PINCH ||
            type == CLUTTER_TOUCHPAD_HOLD) &&
           clutter_event_get_gesture_phase(event) == CLUTTER_TOUCHPAD_GESTURE_PHASE_BEGIN;
}

/* Stage input filters run before Clutter has chosen an event actor.  Client
 * surface actors are intentionally non-reactive, so a reactive pick cannot
 * identify them.  An all-actor pick preserves the topmost stage boundary;
 * walking up then accepts only the window owning that actor. */
static MetaWindowActor* native_input_window_actor_from_stage_actor(ClutterActor* actor) {
    for (; actor; actor = clutter_actor_get_parent(actor))
        if (META_IS_WINDOW_ACTOR(actor))
            return META_WINDOW_ACTOR(actor);
    return NULL;
}

static void native_input_issue_focus_context(GnoblinNativeControl* control,
                                             NativePendingInputEvent* pending) {
    if (!control || !pending || !control->focus_contexts ||
        (clutter_event_get_flags(pending->event) &
         (CLUTTER_EVENT_FLAG_SYNTHETIC | CLUTTER_EVENT_FLAG_INPUT_METHOD)) ||
        g_hash_table_size(control->focus_contexts) >= MAX_FOCUS_CONTEXTS)
        return;
    guint64 generation = native_config_generation(control);
    if (!generation)
        return;
    guint64 handle = ++control->next_focus_context_handle;
    if (!handle)
        handle = ++control->next_focus_context_handle;
    NativeFocusContext* context = g_new0(NativeFocusContext, 1);
    context->generation = generation;
    context->expires_at_us = g_get_monotonic_time() + FOCUS_CONTEXT_LIFETIME_US;
    context->timestamp = clutter_event_get_time(pending->event);
    capture_focus_identity(control, context, pending->event);
    if (clutter_event_type(pending->event) == CLUTTER_BUTTON_PRESS) {
        gfloat x = 0, y = 0;
        clutter_event_get_coords(pending->event, &x, &y);
        MetaBackend* backend = meta_context_get_backend(meta_display_get_context(control->display));
        ClutterActor* stage = backend ? meta_backend_get_stage(backend) : NULL;
        ClutterActor* actor = pending->actor;
        if (!actor && stage && CLUTTER_IS_STAGE(stage))
            actor =
                clutter_stage_get_actor_at_pos(CLUTTER_STAGE(stage), CLUTTER_PICK_REACTIVE, x, y);
        MetaWindowActor* window_actor = native_input_window_actor_from_stage_actor(actor);
        MetaWindow* window = window_actor ? meta_window_actor_get_meta_window(window_actor) : NULL;
        if (window) {
            context->pointer_binding = TRUE;
            context->pointer_button = clutter_event_get_button(pending->event);
            context->pointer_target_sequence = meta_window_get_stable_sequence(window);
            context->pointer_x = x;
            context->pointer_y = y;
            MtkRectangle frame;
            meta_window_get_frame_rect(window, &frame);
            g_autofree char* id = native_window_id(window);
            GVariantBuilder record, rect;
            g_variant_builder_init(&record, G_VARIANT_TYPE_VARDICT);
            g_variant_builder_init(&rect, G_VARIANT_TYPE_VARDICT);
            g_variant_builder_add(&rect, "{sv}", "x", g_variant_new_int32(frame.x));
            g_variant_builder_add(&rect, "{sv}", "y", g_variant_new_int32(frame.y));
            g_variant_builder_add(&rect, "{sv}", "width", g_variant_new_int32(frame.width));
            g_variant_builder_add(&rect, "{sv}", "height", g_variant_new_int32(frame.height));
            g_variant_builder_add(&record, "{sv}", "id", g_variant_new_string(id));
            g_variant_builder_add(&record, "{sv}", "frame", g_variant_builder_end(&rect));
            g_variant_builder_add(&record, "{sv}", "movable",
                                  g_variant_new_boolean(meta_window_allows_move(window)));
            g_variant_builder_add(&record, "{sv}", "resizable",
                                  g_variant_new_boolean(meta_window_allows_resize(window)));
            pending->target_window = g_variant_ref_sink(g_variant_builder_end(&record));
        }
    }
    guint64* key = g_new(guint64, 1);
    *key = handle;
    g_hash_table_insert(control->focus_contexts, key, context);
    pending->focus_context_handle = handle;
    pending->focus_context_expires_at_us = context->expires_at_us;
}

static void native_pending_input_free(gpointer data) {
    NativePendingInputEvent* pending = data;
    if (!pending)
        return;
    if (pending->event)
        clutter_event_free(pending->event);
    g_clear_object(&pending->actor);
    g_clear_pointer(&pending->handler_ids, g_variant_unref);
    g_clear_pointer(&pending->target_window, g_variant_unref);
    g_free(pending->held_disposition_key);
    g_free(pending);
}

static gboolean native_input_send_head(GnoblinNativeControl* control) {
    if (!control || !control->pending_input_events ||
        g_queue_is_empty(control->pending_input_events))
        return TRUE;
    /* Unmatched events held solely for source order do not round-trip through
     * Lua. Replay them locally, preserving any already decided stream. */
    while (!g_queue_is_empty(control->pending_input_events)) {
        NativePendingInputEvent* queued = g_queue_peek_head(control->pending_input_events);
        if (g_variant_n_children(queued->handler_ids))
            break;
        g_autofree char* key = native_input_held_key(control, queued->event);
        gpointer disposition = key && control->held_input_dispositions
                                   ? g_hash_table_lookup(control->held_input_dispositions, key)
                                   : NULL;
        gboolean consume =
            queued->reserved_press || (disposition && GPOINTER_TO_INT(disposition) != 1);
        if (key && native_input_is_stream_begin(queued->event) && control->held_input_dispositions)
            g_hash_table_replace(control->held_input_dispositions, g_strdup(key),
                                 GINT_TO_POINTER(queued->reserved_press ? 3
                                                 : consume              ? 2
                                                                        : 1));
        if (key && native_input_is_terminal(queued->event) && control->held_input_dispositions)
            g_hash_table_remove(control->held_input_dispositions, key);
        g_queue_pop_head(control->pending_input_events);
        meta_display_resume_input_event(control->display, queued->event, queued->actor, consume);
        native_pending_input_free(queued);
    }
    if (g_queue_is_empty(control->pending_input_events))
        return TRUE;
    NativePendingInputEvent* pending = g_queue_peek_head(control->pending_input_events);
    if (pending->dispatched)
        return TRUE;
    const char* type = native_input_event_type(pending->event);
    ClutterInputDevice* device = clutter_event_get_source_device(pending->event);
    GVariantBuilder event, pointer;
    g_variant_builder_init(&event, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&event, "{sv}", "type", g_variant_new_string(type ? type : "unknown"));
    g_variant_builder_add(&event, "{sv}", "phase",
                          g_variant_new_string(native_input_event_phase(pending->event) ?: ""));
    g_variant_builder_add(&event, "{sv}", "time",
                          g_variant_new_uint32(clutter_event_get_time(pending->event)));
    g_variant_builder_add(&event, "{sv}", "modifiers",
                          g_variant_new_uint32(clutter_event_get_state(pending->event)));
    g_variant_builder_add(
        &event, "{sv}", "device_id",
        g_variant_new_string(device ? native_input_device_id(control, device) : ""));
    if (type && g_str_equal(type, "key")) {
        g_variant_builder_add(&event, "{sv}", "keycode",
                              g_variant_new_uint32(clutter_event_get_key_code(pending->event)));
        g_variant_builder_add(&event, "{sv}", "symbol",
                              g_variant_new_uint32(clutter_event_get_key_symbol(pending->event)));
    }
    if (clutter_event_type(pending->event) == CLUTTER_BUTTON_PRESS ||
        clutter_event_type(pending->event) == CLUTTER_BUTTON_RELEASE ||
        clutter_event_type(pending->event) == CLUTTER_PAD_BUTTON_PRESS ||
        clutter_event_type(pending->event) == CLUTTER_PAD_BUTTON_RELEASE)
        g_variant_builder_add(&event, "{sv}", "button",
                              g_variant_new_uint32(clutter_event_get_button(pending->event)));
    if (type && g_str_equal(type, "touch")) {
        ClutterEventSequence* sequence = clutter_event_get_event_sequence(pending->event);
        if (sequence)
            g_variant_builder_add(&event, "{sv}", "slot",
                                  g_variant_new_int32(clutter_event_sequence_get_slot(sequence)));
    }
    if (pending->target_window)
        g_variant_builder_add(&event, "{sv}", "window", pending->target_window);
    if (type && g_str_equal(type, "scroll")) {
        double dx = 0, dy = 0;
        clutter_event_get_scroll_delta(pending->event, &dx, &dy);
        g_variant_builder_add(&event, "{sv}", "dx", g_variant_new_double(dx));
        g_variant_builder_add(&event, "{sv}", "dy", g_variant_new_double(dy));
        g_variant_builder_add(
            &event, "{sv}", "direction",
            g_variant_new_uint32(clutter_event_get_scroll_direction(pending->event)));
    }
    if (type && g_str_equal(type, "gesture")) {
        double dx = 0, dy = 0;
        clutter_event_get_gesture_motion_delta(pending->event, &dx, &dy);
        g_variant_builder_add(&event, "{sv}", "dx", g_variant_new_double(dx));
        g_variant_builder_add(&event, "{sv}", "dy", g_variant_new_double(dy));
        g_variant_builder_add(
            &event, "{sv}", "fingers",
            g_variant_new_uint32(clutter_event_get_touchpad_gesture_finger_count(pending->event)));
        if (clutter_event_type(pending->event) == CLUTTER_TOUCHPAD_PINCH) {
            g_variant_builder_add(
                &event, "{sv}", "scale",
                g_variant_new_double(clutter_event_get_gesture_pinch_scale(pending->event)));
            g_variant_builder_add(
                &event, "{sv}", "angle_delta",
                g_variant_new_double(clutter_event_get_gesture_pinch_angle_delta(pending->event)));
        }
    }
    if (type && g_str_equal(type, "tablet")) {
        guint number = 0, mode = 0;
        ClutterInputDevicePadSource source;
        double value = 0;
        if (clutter_event_get_pad_details(pending->event, &number, &mode, &source, &value)) {
            g_variant_builder_add(&event, "{sv}", "number", g_variant_new_uint32(number));
            g_variant_builder_add(&event, "{sv}", "mode", g_variant_new_uint32(mode));
            g_variant_builder_add(&event, "{sv}", "value", g_variant_new_double(value));
        }
    }
    gfloat x = 0, y = 0;
    clutter_event_get_coords(pending->event, &x, &y);
    g_variant_builder_init(&pointer, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&pointer, "{sv}", "x", g_variant_new_double(x));
    g_variant_builder_add(&pointer, "{sv}", "y", g_variant_new_double(y));
    g_variant_builder_add(&event, "{sv}", "pointer", g_variant_builder_end(&pointer));
    GVariantBuilder payload;
    g_variant_builder_init(&payload, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&payload, "{sv}", "handler_ids", pending->handler_ids);
    g_variant_builder_add(&payload, "{sv}", "event", g_variant_builder_end(&event));
    g_variant_builder_add(&payload, "{sv}", "config_generation",
                          g_variant_new_uint64(pending->generation));
    if (pending->focus_context_handle) {
        g_variant_builder_add(&payload, "{sv}", "focus_context_handle",
                              g_variant_new_uint64(pending->focus_context_handle));
        g_variant_builder_add(&payload, "{sv}", "focus_context_generation",
                              g_variant_new_uint64(pending->generation));
        g_variant_builder_add(&payload, "{sv}", "focus_context_expires_at_us",
                              g_variant_new_int64(pending->focus_context_expires_at_us));
    }
    g_autoptr(GVariant) built = g_variant_ref_sink(g_variant_builder_end(&payload));
    g_autoptr(GError) error = NULL;
    if (!native_runtime_send(control, GNOBLIN_RUNTIME_PACKET_INPUT, pending->request_id, built,
                             &error)) {
        control->input_policy_bypassed = TRUE;
        native_input_clear_requests(control);
        g_free(control->input_policy_error);
        control->input_policy_error =
            g_strdup("Lua input policy runtime is unavailable; bypassing until reload");
        g_warning("gnoblin-input: cannot send Lua input decision request: %s",
                  error ? error->message : "unknown error");
        return FALSE;
    }
    pending->dispatched = TRUE;
    return TRUE;
}

static void native_input_forward_all(GnoblinNativeControl* control, gboolean consume_head) {
    if (!control || !control->pending_input_events)
        return;
    gboolean first = TRUE;
    while (!g_queue_is_empty(control->pending_input_events)) {
        NativePendingInputEvent* pending = g_queue_pop_head(control->pending_input_events);
        g_autofree char* stream_key = native_input_held_key(control, pending->event);
        gpointer disposition =
            stream_key && control->held_input_dispositions
                ? g_hash_table_lookup(control->held_input_dispositions, stream_key)
                : NULL;
        /* On fail-open, do not leak events from a stream that Lua already
         * consumed. A release-only reservation also stays consumed: its press
         * was never delivered, so forwarding the release would orphan it. */
        gboolean consume = first && consume_head;
        if (disposition)
            consume = GPOINTER_TO_INT(disposition) != 1;
        if (stream_key && native_input_is_stream_begin(pending->event) && !disposition &&
            control->held_input_dispositions)
            g_hash_table_replace(control->held_input_dispositions, g_strdup(stream_key),
                                 GINT_TO_POINTER(consume ? 2 : 1));
        if (stream_key && native_input_is_terminal(pending->event) &&
            control->held_input_dispositions)
            g_hash_table_remove(control->held_input_dispositions, stream_key);
        meta_display_resume_input_event(control->display, pending->event, pending->actor, consume);
        native_pending_input_free(pending);
        first = FALSE;
    }
}

/* A held event is always released through Mutter's normal path while the
 * session is usable. During lock and worker transitions it is discarded: an
 * event captured for the old session must never be replayed into the lock UI. */
void native_input_cancel_pending(GnoblinNativeControl* control, gboolean forward) {
    if (!control)
        return;
    if (control->input_decision_timeout_id) {
        g_source_remove(control->input_decision_timeout_id);
        control->input_decision_timeout_id = 0;
    }
    native_input_clear_requests(control);
    /* Keep dispositions when forwarding: a press already consumed before a
     * config reload still owns its physical release. Discarding a session
     * queue (lock/restart) drops them with the session. */
    if (!forward && control->held_input_dispositions)
        g_hash_table_remove_all(control->held_input_dispositions);
    if (forward && control->held_input_dispositions) {
        GHashTableIter held;
        gpointer value;
        g_hash_table_iter_init(&held, control->held_input_dispositions);
        while (g_hash_table_iter_next(&held, NULL, &value))
            if (GPOINTER_TO_INT(value) == 3)
                g_hash_table_iter_replace(&held, GINT_TO_POINTER(2));
    }
    if (forward)
        native_input_forward_all(control, FALSE);
    else if (control->pending_input_events)
        g_queue_clear_full(control->pending_input_events, native_pending_input_free);
}

static gboolean native_input_decision_timeout(gpointer data) {
    GnoblinNativeControl* control = data;
    control->input_decision_timeout_id = 0;
    if (!control || control->stopping)
        return G_SOURCE_REMOVE;
    control->input_policy_bypassed = TRUE;
    g_free(control->input_policy_error);
    control->input_policy_error =
        g_strdup("Lua input policy decision exceeded 16ms; bypassing until reload");
    g_warning("gnoblin-input: %s", control->input_policy_error);
    native_input_cancel_pending(control, TRUE);
    return G_SOURCE_REMOVE;
}

gboolean native_input_handle_decision(GnoblinNativeControl* control,
                                      const GnoblinRuntimePacket* packet, GError** error) {
    if (!control || !control->pending_input_events ||
        g_queue_is_empty(control->pending_input_events))
        return TRUE; /* late decision after timeout/reload */
    NativePendingInputEvent* pending = g_queue_peek_head(control->pending_input_events);
    guint64 generation = 0;
    const char* action = NULL;
    if (!packet->request_id || !pending->dispatched || packet->request_id != pending->request_id ||
        !g_variant_lookup(packet->payload, "config_generation", "t", &generation) ||
        generation != pending->generation || generation != native_config_generation(control) ||
        !g_variant_lookup(packet->payload, "action", "&s", &action) ||
        (!g_str_equal(action, "forward") && !g_str_equal(action, "consume"))) {
        /* A bad current response fails open but does not let stale input
         * affect a newly reloaded configuration. */
        if (packet->request_id != pending->request_id)
            return TRUE;
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                            "Lua sent an invalid input decision");
        return FALSE;
    }
    const char* lua_error = NULL;
    if (g_variant_lookup(packet->payload, "error", "&s", &lua_error) && lua_error && *lua_error) {
        g_free(control->input_policy_error);
        control->input_policy_error = g_strdup(lua_error);
    }
    if (control->input_decision_timeout_id) {
        g_source_remove(control->input_decision_timeout_id);
        control->input_decision_timeout_id = 0;
    }
    g_queue_pop_head(control->pending_input_events);
    gboolean consume = g_str_equal(action, "consume");
    g_autofree char* stream_key = native_input_held_key(control, pending->event);
    gpointer prior_disposition =
        stream_key && control->held_input_dispositions
            ? g_hash_table_lookup(control->held_input_dispositions, stream_key)
            : NULL;
    if (prior_disposition)
        consume = GPOINTER_TO_INT(prior_disposition) != 1;
    if (pending->reserved_press)
        consume = TRUE;
    native_input_accept_request(control, pending->request_id, pending->generation);
    if (pending->held_disposition_key && control->held_input_dispositions)
        g_hash_table_replace(control->held_input_dispositions,
                             g_steal_pointer(&pending->held_disposition_key),
                             GINT_TO_POINTER(pending->reserved_press ? 3
                                             : consume               ? 2
                                                                     : 1));
    if (stream_key && native_input_is_terminal(pending->event) && control->held_input_dispositions)
        g_hash_table_remove(control->held_input_dispositions, stream_key);
    meta_display_resume_input_event(control->display, pending->event, pending->actor, consume);
    native_pending_input_free(pending);
    if (!g_queue_is_empty(control->pending_input_events)) {
        NativePendingInputEvent* next = g_queue_peek_head(control->pending_input_events);
        next->request_id = ++control->next_input_request_id;
        if (!next->request_id)
            next->request_id = ++control->next_input_request_id;
        if (!native_input_send_head(control))
            native_input_forward_all(control, FALSE);
        else if (!g_queue_is_empty(control->pending_input_events))
            control->input_decision_timeout_id =
                g_timeout_add(16, native_input_decision_timeout, control);
    }
    return TRUE;
}

void gnoblin_native_control_set_console_toggle_handler(MetaDisplay* display,
                                                       GnoblinNativeConsoleToggleFunc handler,
                                                       gpointer user_data) {
    if (!display)
        return;
    NativeConsoleToggleBridge* bridge = g_new0(NativeConsoleToggleBridge, 1);
    bridge->handler = handler;
    bridge->user_data = user_data;
    g_object_set_data_full(G_OBJECT(display), NATIVE_CONSOLE_TOGGLE_BRIDGE_DATA_KEY, bridge,
                           g_free);
    GnoblinNativeControl* control =
        g_object_get_data(G_OBJECT(display), NATIVE_CONTROL_OBJECT_DATA_KEY);
    if (control) {
        control->console_toggle_handler = handler;
        control->console_toggle_handler_data = user_data;
    }
}

gboolean gnoblin_native_control_toggle_console(MetaDisplay* display, GError** error) {
    GnoblinNativeControl* control =
        display ? g_object_get_data(G_OBJECT(display), NATIVE_CONTROL_OBJECT_DATA_KEY) : NULL;
    if (!control || !control->console_toggle_handler) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_SUPPORTED,
                            "the Gnoblin diagnostic console is unavailable");
        return FALSE;
    }
    if (control->wayland_compositor &&
        meta_wayland_session_lock_is_active(control->wayland_compositor)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "the Gnoblin diagnostic console is unavailable while locked");
        return FALSE;
    }
    if (!control->console_toggle_handler(display, control->console_toggle_handler_data)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_FAILED,
                            "the Gnoblin diagnostic console could not be toggled");
        return FALSE;
    }
    return TRUE;
}

const char* gnoblin_native_control_input_policy_error(MetaDisplay* display) {
    GnoblinNativeControl* control =
        display ? g_object_get_data(G_OBJECT(display), NATIVE_CONTROL_OBJECT_DATA_KEY) : NULL;
    return control ? control->input_policy_error : NULL;
}

static void native_input_discard_stale_stream_disposition(GnoblinNativeControl* control,
                                                          const ClutterEvent* event,
                                                          const char* held_key) {
    if (!control || !event)
        return;
    gnoblin_input_disposition_discard_stale(
        control->held_input_dispositions, held_key, native_input_is_stream_begin(event),
        (clutter_event_get_flags(event) & CLUTTER_EVENT_FLAG_REPEATED) != 0);
}

gboolean gnoblin_native_control_filter_input_event(MetaDisplay* display, const ClutterEvent* event,
                                                   ClutterActor* event_actor) {
    GnoblinNativeControl* control =
        display ? g_object_get_data(G_OBJECT(display), NATIVE_CONTROL_OBJECT_DATA_KEY) : NULL;
    const char* type = event ? native_input_event_type(event) : NULL;
    if (!control || control->stopping)
        return FALSE;
    /* A pointer button pressed while a bare Super session captures keys is a window chord such as
     * Super + drag, not a Super tap. End the session so it neither swallows the button nor fires on
     * Super's release, and the press reaches the compositor's pointer bindings. */
    if (event && clutter_event_type(event) == CLUTTER_BUTTON_PRESS &&
        !(clutter_event_get_flags(event) &
          (CLUTTER_EVENT_FLAG_SYNTHETIC | CLUTTER_EVENT_FLAG_INPUT_METHOD)) &&
        control->bare_super_shortcut &&
        control->active_shortcut_session == control->bare_super_shortcut &&
        control->shortcut_session_capture_active)
        dynamic_shortcut_end_session(control, control->bare_super_shortcut, "chord");
    if (!meta_display_gnoblin_input_policy_allowed(display) ||
        (control->wayland_compositor &&
         meta_wayland_session_lock_is_active(control->wayland_compositor))) {
        native_input_cancel_pending(control, FALSE);
        return FALSE;
    }
    if (!type && control->pending_input_events &&
        !g_queue_is_empty(control->pending_input_events)) {
        if (control->pending_input_events->length >= 256) {
            control->input_policy_bypassed = TRUE;
            g_free(control->input_policy_error);
            control->input_policy_error =
                g_strdup("Lua input policy queue overflow; bypassing until reload");
            native_input_cancel_pending(control, TRUE);
            return FALSE;
        }
        NativePendingInputEvent* pending = g_new0(NativePendingInputEvent, 1);
        pending->request_id = ++control->next_input_request_id;
        if (!pending->request_id)
            pending->request_id = ++control->next_input_request_id;
        pending->generation = native_config_generation(control);
        pending->event = clutter_event_copy(event);
        GVariantBuilder ids;
        g_variant_builder_init(&ids, G_VARIANT_TYPE("as"));
        pending->handler_ids = g_variant_ref_sink(g_variant_builder_end(&ids));
        g_queue_push_tail(control->pending_input_events, pending);
        return TRUE;
    }
    if (!type)
        return FALSE;
    g_autofree char* held_key = native_input_held_key(control, event);
    native_input_discard_stale_stream_disposition(control, event, held_key);
    gpointer active_disposition =
        held_key && control->held_input_dispositions
            ? g_hash_table_lookup(control->held_input_dispositions, held_key)
            : NULL;
    if (control->input_policy_bypassed || control->runtime_worker_suspended ||
        control->runtime_recovery_failed || !control->runtime_hello_sent) {
        if (active_disposition) {
            gboolean consume = GPOINTER_TO_INT(active_disposition) != 1;
            if (native_input_is_terminal(event))
                g_hash_table_remove(control->held_input_dispositions, held_key);
            if (consume) {
                meta_display_resume_input_event(display, event, event_actor, TRUE);
                return TRUE;
            }
        }
        return FALSE;
    }
    g_autoptr(GVariant) document = native_config_document(control);
    g_autoptr(GVariant) handlers =
        document ? g_variant_lookup_value(document, "input-handlers", NULL) : NULL;
    if (!handlers || !g_variant_is_of_type(handlers, G_VARIANT_TYPE("av")))
        return FALSE;
    ClutterInputDevice* device = clutter_event_get_source_device(event);
    const char* device_id = device ? native_input_device_id(control, device) : "";
    const char* phase = native_input_event_phase(event);
    if (!phase)
        return FALSE;
    gboolean shortcuts_inhibited =
        display->focus_window && meta_window_shortcuts_inhibited(display->focus_window, device);
    gboolean preserve_order =
        control->pending_input_events && !g_queue_is_empty(control->pending_input_events);
    GVariantBuilder ids;
    g_variant_builder_init(&ids, G_VARIANT_TYPE("as"));
    guint matches = 0;
    gboolean release_reservation = FALSE;
    for (gsize i = 0; i < g_variant_n_children(handlers); i++) {
        g_autoptr(GVariant) boxed = g_variant_get_child_value(handlers, i);
        g_autoptr(GVariant) handler = g_variant_is_of_type(boxed, G_VARIANT_TYPE_VARIANT)
                                          ? g_variant_get_variant(boxed)
                                          : g_variant_ref(boxed);
        const char* id = NULL;
        g_autoptr(GVariant) match =
            g_variant_lookup_value(handler, "match", G_VARIANT_TYPE_VARDICT);
        if (!match)
            continue;
        gboolean allow_when_shortcuts_inhibited = FALSE;
        g_variant_lookup(match, "allow-when-shortcuts-inhibited", "b",
                         &allow_when_shortcuts_inhibited);
        const char* configured_phase = NULL;
        if (held_key &&
            (clutter_event_type(event) == CLUTTER_KEY_PRESS ||
             clutter_event_type(event) == CLUTTER_BUTTON_PRESS) &&
            g_variant_lookup(match, "phase", "&s", &configured_phase) &&
            g_str_equal(configured_phase, "release") &&
            (!shortcuts_inhibited || allow_when_shortcuts_inhibited) &&
            native_input_match(control, match, type, device_id, "release", event))
            release_reservation = TRUE;
        if (g_variant_lookup(handler, "id", "&s", &id) && id &&
            (!shortcuts_inhibited || allow_when_shortcuts_inhibited) &&
            native_input_match(control, match, type, device_id, phase, event)) {
            g_variant_builder_add(&ids, "s", id);
            matches++;
        }
    }
    if (!matches && release_reservation && held_key && control->held_input_dispositions &&
        !preserve_order) {
        g_hash_table_replace(control->held_input_dispositions, g_steal_pointer(&held_key),
                             GINT_TO_POINTER(3));
        meta_display_resume_input_event(display, event, event_actor, TRUE);
        return TRUE;
    }
    if (!matches && active_disposition && !preserve_order) {
        gboolean consume = GPOINTER_TO_INT(active_disposition) != 1;
        if (native_input_is_terminal(event))
            g_hash_table_remove(control->held_input_dispositions, held_key);
        if (consume) {
            meta_display_resume_input_event(display, event, event_actor, TRUE);
            return TRUE;
        }
        return FALSE;
    }
    if (!matches && !preserve_order) {
        if (held_key && native_input_is_stream_begin(event) && control->held_input_dispositions)
            g_hash_table_replace(control->held_input_dispositions, g_steal_pointer(&held_key),
                                 GINT_TO_POINTER(1));
        return FALSE;
    }
    if (!control->pending_input_events)
        control->pending_input_events = g_queue_new();
    if (control->pending_input_events->length >= 256) {
        control->input_policy_bypassed = TRUE;
        g_free(control->input_policy_error);
        control->input_policy_error =
            g_strdup("Lua input policy queue overflow; bypassing until reload");
        g_warning("gnoblin-input: %s", control->input_policy_error);
        native_input_cancel_pending(control, TRUE);
        return FALSE;
    }
    NativePendingInputEvent* pending = g_new0(NativePendingInputEvent, 1);
    pending->request_id = ++control->next_input_request_id;
    if (!pending->request_id)
        pending->request_id = ++control->next_input_request_id;
    pending->generation = native_config_generation(control);
    pending->event = clutter_event_copy(event);
    pending->actor = event_actor ? g_object_ref(event_actor) : NULL;
    pending->handler_ids = g_variant_ref_sink(g_variant_builder_end(&ids));
    pending->reserved_press = release_reservation && !matches;
    if (held_key && native_input_is_stream_begin(event))
        pending->held_disposition_key = g_steal_pointer(&held_key);
    if (matches)
        native_input_issue_focus_context(control, pending);
    g_queue_push_tail(control->pending_input_events, pending);
    if (control->pending_input_events->length == 1) {
        if (!native_input_send_head(control)) {
            native_input_forward_all(control, FALSE);
            return TRUE;
        }
        if (!g_queue_is_empty(control->pending_input_events))
            control->input_decision_timeout_id =
                g_timeout_add(16, native_input_decision_timeout, control);
    }
    return TRUE;
}

GVariant* input_device_snapshot(GnoblinNativeControl* control) {
    GVariantBuilder devices;
    GVariantBuilder snapshot;
    g_autoptr(GList) device_list = NULL;
    g_variant_builder_init(&devices, G_VARIANT_TYPE("av"));

    if (control->input_seat)
        device_list = clutter_seat_list_devices(control->input_seat);

    for (GList* item = device_list; item; item = item->next) {
        g_autoptr(GVariant) record = input_device_record(control, item->data);
        g_variant_builder_add_value(&devices, g_variant_new_variant(record));
    }

    g_variant_builder_init(&snapshot, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&snapshot, "{sv}", "devices", g_variant_builder_end(&devices));
    g_variant_builder_add(&snapshot, "{sv}", "revision",
                          g_variant_new_int64(control->state_revision));
    return g_variant_ref_sink(g_variant_builder_end(&snapshot));
}
