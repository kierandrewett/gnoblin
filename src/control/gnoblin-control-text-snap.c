#include "gnoblin-native-control-private.h"

GVariant* native_socket_create_snap_context(Client* client, JsonObject* arguments, GError** error) {
    NativeFocusGrant grant = {0};
    if (!native_socket_take_focus_grant(client, arguments, &grant, error))
        return NULL;
    GVariant* result = gnoblin_native_control_create_snap_context(
        client->control->display, grant.handle, grant.generation, grant.expires_at_us, error);
    if (!result)
        return NULL;

    const char* token = NULL;
    if (!g_variant_lookup(result, "context", "&s", &token) || !token) {
        g_variant_unref(result);
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                            "native SnapContext result has no context token");
        return NULL;
    }
    NativeSnapContext* context = g_hash_table_lookup(client->control->snap_contexts, token);
    if (!context) {
        g_variant_unref(result);
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                            "native SnapContext was not retained");
        return NULL;
    }
    context->socket_owner_client_id = client->client_id;

    const char* fields[] = {"focus_context"};
    if (!native_socket_has_exact_fields(arguments, fields, G_N_ELEMENTS(fields))) {
        g_hash_table_remove(client->control->snap_contexts, token);
        g_variant_unref(result);
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "window.snap_context accepts only focus_context");
        return NULL;
    }
    GVariantBuilder public_result;
    g_variant_builder_init(&public_result, G_VARIANT_TYPE_VARDICT);
    GVariantIter iter;
    const char* key;
    GVariant* value;
    g_variant_iter_init(&iter, result);
    while (g_variant_iter_next(&iter, "{&sv}", &key, &value)) {
        g_autoptr(GVariant) owned_value = value;
        if (!g_str_equal(key, "expires_at_us"))
            g_variant_builder_add(&public_result, "{sv}", key, owned_value);
    }
    g_variant_unref(result);
    return g_variant_ref_sink(g_variant_builder_end(&public_result));
}

static GVariant* native_commit_snap_context_owned(MetaDisplay* display, GVariant* arguments,
                                                  guint64 socket_owner_client_id, GError** error) {
    GnoblinNativeControl* control =
        display ? g_object_get_data(G_OBJECT(display), NATIVE_CONTROL_OBJECT_DATA_KEY) : NULL;
    const char* token = NULL;
    const char* monitor_id = NULL;
    g_autoptr(GVariant) frame_value = NULL;
    MtkRectangle frame, monitor_rect, work_area;
    if (!control || control->stopping || !control->snap_contexts ||
        !g_variant_is_of_type(arguments, G_VARIANT_TYPE_VARDICT) ||
        !g_variant_lookup(arguments, "context", "&s", &token) || !token ||
        !focus_token_has_valid_shape(token)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "window.snap requires a SnapContext token");
        return NULL;
    }
    NativeSnapContext* stored = g_hash_table_lookup(control->snap_contexts, token);
    if (!stored) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "SnapContext is expired, consumed or revoked");
        return NULL;
    }
    if (stored->socket_owner_client_id != socket_owner_client_id) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "SnapContext belongs to another connection");
        return NULL;
    }
    NativeSnapContext context = *stored;
    g_autofree char* window_id = g_strdup(stored->window_id);
    g_hash_table_remove(control->snap_contexts, token);
    if (g_variant_n_children(arguments) != 3 ||
        !g_variant_lookup(arguments, "monitor_id", "&s", &monitor_id) || !monitor_id ||
        !*monitor_id ||
        !(frame_value = g_variant_lookup_value(arguments, "frame", G_VARIANT_TYPE_VARDICT)) ||
        !gnoblin_control_variant_rect(frame_value, &frame)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "window.snap requires a monitor_id and valid frame");
        return NULL;
    }
    int monitor_number = -1;
    if (context.settings_revision != native_config_revision(control) ||
        context.native_generation != native_config_generation(control) ||
        context.expires_at_us <= g_get_monotonic_time() ||
        (control->wayland_compositor &&
         meta_wayland_session_lock_is_active(control->wayland_compositor)) ||
        !native_monitor_lookup(control, monitor_id, &monitor_number, &monitor_rect)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "SnapContext or target monitor is stale");
        return NULL;
    }
    g_autoptr(GList) windows = meta_display_list_all_windows(display);
    MetaWindow* window = NULL;
    for (GList* item = windows; item; item = item->next) {
        MetaWindow* candidate = item->data;
        g_autofree char* id = native_window_id(candidate);
        if (g_str_equal(id, window_id)) {
            window = candidate;
            break;
        }
    }
    if (!window || meta_window_is_skip_taskbar(window) ||
        meta_window_is_override_redirect(window) || !meta_window_allows_move(window)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_FOUND,
                            "SnapContext window is no longer available for moving");
        return NULL;
    }
    meta_window_get_work_area_for_monitor(window, monitor_number, &work_area);
    if (!gnoblin_control_rect_contains(&work_area, &frame)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "snap frame falls outside the target work area");
        return NULL;
    }
    if (meta_window_get_maximize_flags(window) != 0)
        meta_window_set_unmaximize_flags(window, META_MAXIMIZE_BOTH);
    if (!g_hash_table_contains(control->snap_restore_frames, window)) {
        MtkRectangle* original_frame = g_new(MtkRectangle, 1);
        *original_frame = context.original_frame;
        g_hash_table_insert(control->snap_restore_frames, window, original_frame);
    }
    meta_window_move_resize_frame(window, TRUE, frame.x, frame.y, frame.width, frame.height);
    GVariantBuilder result;
    g_variant_builder_init(&result, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&result, "{sv}", "window_id", g_variant_new_string(window_id));
    g_variant_builder_add(&result, "{sv}", "monitor_id", g_variant_new_string(monitor_id));
    g_variant_builder_add(&result, "{sv}", "committed", g_variant_new_boolean(TRUE));
    return g_variant_ref_sink(g_variant_builder_end(&result));
}

GVariant* gnoblin_native_control_commit_snap_context(MetaDisplay* display, GVariant* arguments,
                                                     GError** error) {
    return native_commit_snap_context_owned(display, arguments, 0, error);
}

GVariant* native_socket_commit_snap_context(Client* client, JsonObject* json_arguments,
                                            GError** error) {
    JsonNode* token_node =
        json_arguments ? json_object_get_member(json_arguments, "context") : NULL;
    if (!client || client->closing || !client->control || !token_node ||
        !JSON_NODE_HOLDS_VALUE(token_node) ||
        json_node_get_value_type(token_node) != G_TYPE_STRING) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "window.snap requires a context token");
        return NULL;
    }
    const char* token = json_node_get_string(token_node);
    if (!focus_token_has_valid_shape(token)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "SnapContext is invalid or revoked");
        return NULL;
    }

    JsonNode* monitor_node = json_object_get_member(json_arguments, "monitor_id");
    JsonNode* frame_node = json_object_get_member(json_arguments, "frame");
    const char* monitor_id = monitor_node && JSON_NODE_HOLDS_VALUE(monitor_node) &&
                                     json_node_get_value_type(monitor_node) == G_TYPE_STRING
                                 ? json_node_get_string(monitor_node)
                                 : NULL;
    const char* fields[] = {"context", "monitor_id", "frame"};
    g_autoptr(GVariant) frame = native_socket_snap_rect(frame_node);
    GVariantBuilder builder;
    g_variant_builder_init(&builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&builder, "{sv}", "context", g_variant_new_string(token));
    if (native_socket_has_exact_fields(json_arguments, fields, G_N_ELEMENTS(fields)) &&
        monitor_id && *monitor_id && frame) {
        g_variant_builder_add(&builder, "{sv}", "monitor_id", g_variant_new_string(monitor_id));
        g_variant_builder_add(&builder, "{sv}", "frame", frame);
    }
    g_autoptr(GVariant) arguments = g_variant_ref_sink(g_variant_builder_end(&builder));
    return native_commit_snap_context_owned(client->control->display, arguments, client->client_id,
                                            error);
}

GVariant* gnoblin_native_control_create_text_target(MetaDisplay* display, guint64 context_handle,
                                                    guint64 generation, gint64 expires_at_us,
                                                    GError** error) {
    GnoblinNativeControl* control =
        display ? g_object_get_data(G_OBJECT(display), NATIVE_CONTROL_OBJECT_DATA_KEY) : NULL;
    if (!control || control->stopping || !control->wayland_compositor ||
        !control->wayland_compositor->seat || !control->focus_contexts ||
        !control->wayland_compositor->seat->text_input || !control->text_targets ||
        !context_handle) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_SUPPORTED,
                            "text insertion requires an active Wayland session");
        return NULL;
    }

    prune_focus_contexts(control, g_get_monotonic_time());
    NativeFocusContext* stored = g_hash_table_lookup(control->focus_contexts, &context_handle);
    if (!stored) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "text target requires an unused shortcut FocusContext");
        return NULL;
    }
    NativeFocusContext context = *stored;
    g_hash_table_remove(control->focus_contexts, &context_handle);
    revoke_focus_grants_for_handle(control, context_handle);
    if (g_hash_table_size(control->text_targets) >= MAX_TEXT_TARGETS) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NO_SPACE,
                            "too many active text targets; retry after one expires");
        return NULL;
    }
    if (context.generation != generation ||
        context.generation != native_config_generation(control) ||
        context.expires_at_us != expires_at_us || context.expires_at_us <= g_get_monotonic_time() ||
        !focus_identity_matches(control, context.surface_id, context.client_id,
                                context.focus_epoch) ||
        !native_text_modifiers_allowed(control, context.allowed_modifier_mask) ||
        (context.modifiers & (CLUTTER_CONTROL_MASK | CLUTTER_MOD1_MASK | CLUTTER_SHIFT_MASK |
                              CLUTTER_LOCK_MASK | CLUTTER_META_MASK | CLUTTER_HYPER_MASK))) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "text target FocusContext is expired or no longer matches focus");
        return NULL;
    }
    if (meta_wayland_session_lock_is_active(control->wayland_compositor)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "text insertion is unavailable while the session is locked");
        return NULL;
    }

    graphene_rect_t caret;
    guint64 surface_id = 0, client_id = 0, focus_epoch = 0;
    if (!meta_wayland_text_input_get_gnoblin_state(control->wayland_compositor->seat->text_input,
                                                   &surface_id, &client_id, &focus_epoch, &caret) ||
        surface_id != context.surface_id || client_id != context.client_id ||
        focus_epoch != context.focus_epoch) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_SUPPORTED,
                            "focused Wayland client has no active text input");
        return NULL;
    }

    char token[65] = {0};
    gboolean unique = FALSE;
    for (guint attempt = 0; attempt < 4; attempt++) {
        if (!focus_token_random(token))
            break;
        if (!g_hash_table_contains(control->text_targets, token)) {
            unique = TRUE;
            break;
        }
    }
    if (!unique) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_FAILED,
                            "could not allocate a one-use text target");
        return NULL;
    }
    NativeTextTarget* target = g_new0(NativeTextTarget, 1);
    target->generation = generation;
    target->expires_at_us = context.expires_at_us;
    target->surface_id = surface_id;
    target->client_id = client_id;
    target->focus_epoch = focus_epoch;
    target->allowed_modifier_mask = context.allowed_modifier_mask;
    g_hash_table_insert(control->text_targets, g_strdup(token), target);

    GVariantBuilder result, caret_result;
    g_variant_builder_init(&result, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_init(&caret_result, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&result, "{sv}", "target", g_variant_new_string(token));
    MetaWaylandSurface* focused_surface =
        meta_wayland_seat_get_input_focus(control->wayland_compositor->seat);
    MetaWindow* focused_window =
        focused_surface ? meta_wayland_surface_get_window(focused_surface) : NULL;
    if (focused_window) {
        g_autofree char* window_id = native_window_id(focused_window);
        g_variant_builder_add(&result, "{sv}", "window_id", g_variant_new_string(window_id));
    }
    g_variant_builder_add(&caret_result, "{sv}", "x", g_variant_new_double(caret.origin.x));
    g_variant_builder_add(&caret_result, "{sv}", "y", g_variant_new_double(caret.origin.y));
    g_variant_builder_add(&caret_result, "{sv}", "width", g_variant_new_double(caret.size.width));
    g_variant_builder_add(&caret_result, "{sv}", "height", g_variant_new_double(caret.size.height));
    g_variant_builder_add(&result, "{sv}", "caret", g_variant_builder_end(&caret_result));
    return g_variant_ref_sink(g_variant_builder_end(&result));
}

GVariant* native_insert_text_owned(MetaDisplay* display, GVariant* arguments,
                                   guint64 socket_owner_client_id, GError** error) {
    GnoblinNativeControl* control =
        display ? g_object_get_data(G_OBJECT(display), NATIVE_CONTROL_OBJECT_DATA_KEY) : NULL;
    const char* token = NULL;
    const char* text = NULL;
    if (!control || !control->text_targets || !arguments ||
        !g_variant_is_of_type(arguments, G_VARIANT_TYPE_VARDICT) ||
        !g_variant_lookup(arguments, "target", "&s", &token) || !token || !*token) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "input.insert_text requires a one-use TextTarget and text");
        return NULL;
    }
    NativeTextTarget* stored = g_hash_table_lookup(control->text_targets, token);
    if (!stored) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "TextTarget was consumed, expired, or revoked");
        return NULL;
    }
    if (stored->socket_owner_client_id != socket_owner_client_id) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "TextTarget belongs to another connection");
        return NULL;
    }
    NativeTextTarget target = *stored;
    g_hash_table_remove(control->text_targets, token);
    if (g_variant_n_children(arguments) != 2 || !g_variant_lookup(arguments, "text", "&s", &text) ||
        !native_text_valid(text)) {
        g_set_error_literal(
            error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
            "inserted text must be 1-256 bytes of UTF-8 without control characters");
        return NULL;
    }
    if (control->stopping || target.generation != native_config_generation(control) ||
        target.expires_at_us <= g_get_monotonic_time() ||
        !native_text_modifiers_allowed(control, target.allowed_modifier_mask) ||
        !control->wayland_compositor || !control->wayland_compositor->seat ||
        !control->wayland_compositor->seat->text_input ||
        meta_wayland_session_lock_is_active(control->wayland_compositor) ||
        !focus_identity_matches(control, target.surface_id, target.client_id, target.focus_epoch) ||
        !meta_wayland_text_input_insert_gnoblin_text(control->wayland_compositor->seat->text_input,
                                                     target.surface_id, target.client_id,
                                                     target.focus_epoch, text)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "TextTarget no longer matches the active Wayland text input");
        return NULL;
    }
    GVariantBuilder result;
    g_variant_builder_init(&result, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&result, "{sv}", "inserted", g_variant_new_boolean(TRUE));
    return g_variant_ref_sink(g_variant_builder_end(&result));
}

GVariant* gnoblin_native_control_insert_text(MetaDisplay* display, GVariant* arguments,
                                             GError** error) {
    return native_insert_text_owned(display, arguments, 0, error);
}

GVariant* gnoblin_native_control_begin_window_grab(MetaDisplay* display, const char* method,
                                                   GVariant* arguments, guint64 context_handle,
                                                   guint64 generation, GError** error) {
    GnoblinNativeControl* control =
        display ? g_object_get_data(G_OBJECT(display), NATIVE_CONTROL_OBJECT_DATA_KEY) : NULL;
    if (!control || control->stopping || !control->focus_contexts || !context_handle ||
        (!g_str_equal(method, "window.begin_move") &&
         !g_str_equal(method, "window.begin_resize"))) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "interactive window operation has no active shortcut context");
        return NULL;
    }

    prune_focus_contexts(control, g_get_monotonic_time());
    NativeFocusContext* stored = g_hash_table_lookup(control->focus_contexts, &context_handle);
    if (!stored) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "interactive window operation context was already used or revoked");
        return NULL;
    }
    NativeFocusContext context = *stored;
    g_hash_table_remove(control->focus_contexts, &context_handle);
    revoke_focus_grants_for_handle(control, context_handle);
    if (context.generation != generation ||
        context.generation != native_config_generation(control) ||
        context.expires_at_us <= g_get_monotonic_time()) {
        g_set_error_literal(
            error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
            "interactive window operation context expired or belongs to an old runtime");
        return NULL;
    }
    if (control->wayland_compositor &&
        meta_wayland_session_lock_is_active(control->wayland_compositor)) {
        g_set_error_literal(
            error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
            "interactive window operations are unavailable while the session is locked");
        return NULL;
    }

    gboolean resize = g_str_equal(method, "window.begin_resize");
    const char* wanted_id = NULL;
    const char* edge = NULL;
    if (!arguments || !g_variant_is_of_type(arguments, G_VARIANT_TYPE_VARDICT) ||
        g_variant_n_children(arguments) != (resize ? 2 : 1) ||
        !g_variant_lookup(arguments, "id", "&s", &wanted_id) || !*wanted_id ||
        (resize && !g_variant_lookup(arguments, "edge", "&s", &edge))) {
        g_set_error_literal(
            error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
            "interactive window operation requires a stable window ID and valid arguments");
        return NULL;
    }

    MetaGrabOp op;
    if (!resize) {
        op = context.pointer_binding
                 ? META_GRAB_OP_MOVING_UNCONSTRAINED
                 : META_GRAB_OP_KEYBOARD_MOVING | META_GRAB_OP_WINDOW_FLAG_UNCONSTRAINED;
    } else if (g_str_equal(edge, "north")) {
        op =
            (context.pointer_binding ? META_GRAB_OP_RESIZING_N : META_GRAB_OP_KEYBOARD_RESIZING_N) |
            META_GRAB_OP_WINDOW_FLAG_UNCONSTRAINED;
    } else if (g_str_equal(edge, "south")) {
        op =
            (context.pointer_binding ? META_GRAB_OP_RESIZING_S : META_GRAB_OP_KEYBOARD_RESIZING_S) |
            META_GRAB_OP_WINDOW_FLAG_UNCONSTRAINED;
    } else if (g_str_equal(edge, "east")) {
        op =
            (context.pointer_binding ? META_GRAB_OP_RESIZING_E : META_GRAB_OP_KEYBOARD_RESIZING_E) |
            META_GRAB_OP_WINDOW_FLAG_UNCONSTRAINED;
    } else if (g_str_equal(edge, "west")) {
        op =
            (context.pointer_binding ? META_GRAB_OP_RESIZING_W : META_GRAB_OP_KEYBOARD_RESIZING_W) |
            META_GRAB_OP_WINDOW_FLAG_UNCONSTRAINED;
    } else if (g_str_equal(edge, "north_east")) {
        op = (context.pointer_binding ? META_GRAB_OP_RESIZING_NE
                                      : META_GRAB_OP_KEYBOARD_RESIZING_NE) |
             META_GRAB_OP_WINDOW_FLAG_UNCONSTRAINED;
    } else if (g_str_equal(edge, "north_west")) {
        op = (context.pointer_binding ? META_GRAB_OP_RESIZING_NW
                                      : META_GRAB_OP_KEYBOARD_RESIZING_NW) |
             META_GRAB_OP_WINDOW_FLAG_UNCONSTRAINED;
    } else if (g_str_equal(edge, "south_east")) {
        op = (context.pointer_binding ? META_GRAB_OP_RESIZING_SE
                                      : META_GRAB_OP_KEYBOARD_RESIZING_SE) |
             META_GRAB_OP_WINDOW_FLAG_UNCONSTRAINED;
    } else if (g_str_equal(edge, "south_west")) {
        op = (context.pointer_binding ? META_GRAB_OP_RESIZING_SW
                                      : META_GRAB_OP_KEYBOARD_RESIZING_SW) |
             META_GRAB_OP_WINDOW_FLAG_UNCONSTRAINED;
    } else {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "window.begin_resize edge is not a valid ResizeEdge");
        return NULL;
    }

    g_autoptr(GList) windows = meta_display_list_all_windows(display);
    MetaWindow* target = NULL;
    for (GList* item = windows; item; item = item->next) {
        MetaWindow* window = item->data;
        if (meta_window_is_skip_taskbar(window) || meta_window_is_override_redirect(window))
            continue;
        g_autofree char* id = native_window_id(window);
        if (g_str_equal(id, wanted_id)) {
            target = window;
            break;
        }
    }
    if (!target) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_FOUND,
                            "window ID not found; list windows first");
        return NULL;
    }
    if (context.pointer_binding &&
        !native_pointer_button_is_down(control, context.pointer_button)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "pointer binding button is no longer held");
        return NULL;
    }
    if (context.pointer_binding &&
        meta_window_get_stable_sequence(target) != context.pointer_target_sequence) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "pointer binding can only operate on its original window");
        return NULL;
    }
    if ((resize && !meta_window_allows_resize(target)) ||
        (!resize && !meta_window_allows_move(target))) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_SUPPORTED,
                            resize ? "window cannot be resized" : "window cannot be moved");
        return NULL;
    }

    MetaBackend* backend = meta_context_get_backend(meta_display_get_context(display));
    ClutterBackend* clutter_backend = backend ? meta_backend_get_clutter_backend(backend) : NULL;
    ClutterActor* stage_actor = backend ? meta_backend_get_stage(backend) : NULL;
    if (!clutter_backend || !stage_actor || !CLUTTER_IS_STAGE(stage_actor)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_SUPPORTED,
                            "interactive window operations need an active Mutter stage");
        return NULL;
    }
    ClutterSprite* sprite =
        clutter_backend_get_pointer_sprite(clutter_backend, CLUTTER_STAGE(stage_actor));
    graphene_point_t pointer_origin = GRAPHENE_POINT_INIT(context.pointer_x, context.pointer_y);
    gboolean began =
        context.pointer_binding
            ? meta_window_begin_pointer_grab_op(target, op, sprite, context.timestamp,
                                                &pointer_origin, context.pointer_button)
            : meta_window_begin_grab_op(target, op, sprite, context.timestamp, NULL);
    if (!began) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_BUSY,
                            "Mutter could not start the interactive window operation");
        return NULL;
    }

    GVariantBuilder result;
    g_variant_builder_init(&result, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&result, "{sv}", "id", g_variant_new_string(wanted_id));
    return g_variant_ref_sink(g_variant_builder_end(&result));
}

GVariant* gnoblin_native_control_begin_menu_window_grab(MetaDisplay* display, const char* method,
                                                        GVariant* arguments, guint64 context_handle,
                                                        guint64 generation,
                                                        guint64 socket_owner_client_id,
                                                        GError** error) {
    GnoblinNativeControl* control =
        display ? g_object_get_data(G_OBJECT(display), NATIVE_CONTROL_OBJECT_DATA_KEY) : NULL;
    if (!control || control->stopping || !control->menu_contexts || !context_handle ||
        (!g_str_equal(method, "window.begin_move") &&
         !g_str_equal(method, "window.begin_resize"))) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "interactive WM menu operation has no live authority");
        return NULL;
    }

    NativeMenuContext* stored = g_hash_table_lookup(control->menu_contexts, &context_handle);
    if (!stored) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "WM menu capability was already used or revoked");
        return NULL;
    }
    NativeMenuContext context = *stored;
    context.window_id = g_strdup(stored->window_id);
    g_hash_table_remove(control->menu_contexts, &context_handle);
    if (context.socket_owner_client_id != socket_owner_client_id ||
        context.generation != generation ||
        context.generation != native_config_generation(control) ||
        context.expires_at_us <= g_get_monotonic_time()) {
        g_free(context.window_id);
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "WM menu capability expired or belongs to another owner/runtime");
        return NULL;
    }
    if (control->wayland_compositor &&
        meta_wayland_session_lock_is_active(control->wayland_compositor)) {
        g_free(context.window_id);
        g_set_error_literal(
            error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
            "interactive window operations are unavailable while the session is locked");
        return NULL;
    }

    gboolean resize = g_str_equal(method, "window.begin_resize");
    const char* edge = NULL;
    gboolean exact = arguments && g_variant_is_of_type(arguments, G_VARIANT_TYPE_VARDICT) &&
                     g_variant_n_children(arguments) == (resize ? 1 : 0) &&
                     (!resize || g_variant_lookup(arguments, "edge", "&s", &edge));
    if (!exact) {
        g_free(context.window_id);
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "WM menu operation arguments are malformed");
        return NULL;
    }

    MetaGrabOp op = META_GRAB_OP_KEYBOARD_MOVING | META_GRAB_OP_WINDOW_FLAG_UNCONSTRAINED;
    if (resize) {
        static const struct {
            const char* name;
            MetaGrabOp op;
        } edges[] = {
            {"north", META_GRAB_OP_KEYBOARD_RESIZING_N},
            {"south", META_GRAB_OP_KEYBOARD_RESIZING_S},
            {"east", META_GRAB_OP_KEYBOARD_RESIZING_E},
            {"west", META_GRAB_OP_KEYBOARD_RESIZING_W},
            {"north_east", META_GRAB_OP_KEYBOARD_RESIZING_NE},
            {"north_west", META_GRAB_OP_KEYBOARD_RESIZING_NW},
            {"south_east", META_GRAB_OP_KEYBOARD_RESIZING_SE},
            {"south_west", META_GRAB_OP_KEYBOARD_RESIZING_SW},
        };
        gboolean matched = FALSE;
        for (guint i = 0; i < G_N_ELEMENTS(edges); i++) {
            if (g_str_equal(edge, edges[i].name)) {
                op = edges[i].op | META_GRAB_OP_WINDOW_FLAG_UNCONSTRAINED;
                matched = TRUE;
                break;
            }
        }
        if (!matched) {
            g_free(context.window_id);
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                                "window.begin_resize edge is not a valid ResizeEdge");
            return NULL;
        }
    }

    g_autoptr(GList) windows = meta_display_list_all_windows(display);
    MetaWindow* target = NULL;
    for (GList* item = windows; item; item = item->next) {
        MetaWindow* window = item->data;
        if (meta_window_is_skip_taskbar(window) || meta_window_is_override_redirect(window))
            continue;
        g_autofree char* id = native_window_id(window);
        if (g_str_equal(id, context.window_id)) {
            target = window;
            break;
        }
    }
    if (!target) {
        g_free(context.window_id);
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_FOUND,
                            "WM menu target is no longer a live window");
        return NULL;
    }
    if ((resize && !meta_window_allows_resize(target)) ||
        (!resize && !meta_window_allows_move(target))) {
        g_free(context.window_id);
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_SUPPORTED,
                            resize ? "WM menu target cannot be resized"
                                   : "WM menu target cannot be moved");
        return NULL;
    }

    MetaBackend* backend = meta_context_get_backend(meta_display_get_context(display));
    ClutterBackend* clutter_backend = backend ? meta_backend_get_clutter_backend(backend) : NULL;
    ClutterActor* stage_actor = backend ? meta_backend_get_stage(backend) : NULL;
    if (!clutter_backend || !stage_actor || !CLUTTER_IS_STAGE(stage_actor)) {
        g_free(context.window_id);
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_SUPPORTED,
                            "interactive window operations need an active Mutter stage");
        return NULL;
    }
    ClutterSprite* sprite =
        clutter_backend_get_pointer_sprite(clutter_backend, CLUTTER_STAGE(stage_actor));
    guint32 timestamp = meta_display_get_current_time_roundtrip(display);
    gboolean began = meta_window_begin_grab_op(target, op, sprite, timestamp, NULL);
    g_autofree char* result_id = g_strdup(context.window_id);
    g_free(context.window_id);
    if (!began) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_BUSY,
                            "Mutter could not start the WM menu window operation");
        return NULL;
    }
    GVariantBuilder result;
    g_variant_builder_init(&result, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&result, "{sv}", "id", g_variant_new_string(result_id));
    return g_variant_ref_sink(g_variant_builder_end(&result));
}

void gnoblin_native_control_revoke_focus_contexts(MetaDisplay* display) {
    GnoblinNativeControl* control =
        display ? g_object_get_data(G_OBJECT(display), NATIVE_CONTROL_OBJECT_DATA_KEY) : NULL;
    revoke_focus_contexts(control);
}
