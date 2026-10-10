#include "gnoblin-native-control-private.h"

static void dispatch_dynamic_shortcut_repeat(GnoblinNativeControl* control, guint action,
                                             const ClutterEvent* event);

static void publish_shortcut_focus_event(GnoblinNativeControl* control, const char* shortcut,
                                         guint64 handle, guint64 generation, gint64 expires_at_us);

static ClutterModifierType native_pointer_button_mask(guint button) {
    /* The configured-pointer parser intentionally supports Button1..Button3. */
    if (button < CLUTTER_BUTTON_PRIMARY || button > CLUTTER_BUTTON_SECONDARY)
        return 0;
    if (button == CLUTTER_BUTTON_PRIMARY)
        return CLUTTER_BUTTON1_MASK;
    /* Mutter's native seat indexes its mask table in evdev order (left, right, middle) with a
     * Clutter button number, so it reports the right button as BUTTON2_MASK and the middle button
     * as BUTTON3_MASK. Other backends use the plain numbering. Accept both masks for buttons 2
     * and 3 so Super+Button3 stays "held" on every backend. */
    return CLUTTER_BUTTON2_MASK | CLUTTER_BUTTON3_MASK;
}

gboolean native_pointer_button_is_down(GnoblinNativeControl* control, guint button) {
    ClutterModifierType required = native_pointer_button_mask(button);
    ClutterModifierType state = 0;
    if (!control || !control->input_seat || !required ||
        !clutter_seat_query_state(control->input_seat, NULL, NULL, &state))
        return FALSE;
    return (state & required) != 0;
}

void gnoblin_native_control_pointer_binding_activated(MetaDisplay* display, MetaWindow* window,
                                                      const ClutterEvent* event,
                                                      const char* binding) {
    GnoblinNativeControl* control =
        display ? g_object_get_data(G_OBJECT(display), NATIVE_CONTROL_OBJECT_DATA_KEY) : NULL;
    if (!control || control->stopping || !window || !event || !binding || !*binding ||
        clutter_event_type(event) != CLUTTER_BUTTON_PRESS ||
        (clutter_event_get_flags(event) &
         (CLUTTER_EVENT_FLAG_SYNTHETIC | CLUTTER_EVENT_FLAG_INPUT_METHOD)) ||
        (control->wayland_compositor &&
         meta_wayland_session_lock_is_active(control->wayland_compositor)))
        return;

    guint button = clutter_event_get_button(event);
    if (!native_pointer_button_mask(button) || !native_pointer_button_is_down(control, button))
        return;
    guint64 generation = native_config_generation(control);
    if (!generation)
        return;
    gint64 now = g_get_monotonic_time();
    prune_focus_contexts(control, now);
    if (!control->focus_contexts ||
        g_hash_table_size(control->focus_contexts) >= MAX_FOCUS_CONTEXTS)
        return;

    guint64 handle = ++control->next_focus_context_handle;
    if (!handle)
        handle = ++control->next_focus_context_handle;
    guint64* key = g_new(guint64, 1);
    *key = handle;
    NativeFocusContext* context = g_new0(NativeFocusContext, 1);
    context->generation = generation;
    context->expires_at_us = now + FOCUS_CONTEXT_LIFETIME_US;
    context->timestamp = clutter_event_get_time(event);
    context->pointer_binding = TRUE;
    context->pointer_button = button;
    context->pointer_target_sequence = meta_window_get_stable_sequence(window);
    gfloat pointer_x = 0, pointer_y = 0;
    clutter_event_get_coords(event, &pointer_x, &pointer_y);
    context->pointer_x = pointer_x;
    context->pointer_y = pointer_y;
    /* Pointer bindings authorise operations on their pressed window. Unlike
     * keyboard shortcuts, this also covers Xwayland windows which have no
     * Wayland focus identity to capture. */
    g_hash_table_insert(control->focus_contexts, key, context);

    MtkRectangle frame;
    meta_window_get_frame_rect(window, &frame);
    g_autofree char* window_id = native_window_id(window);
    GVariantBuilder payload, window_payload, pointer_payload, frame_payload;
    g_variant_builder_init(&payload, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_init(&window_payload, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_init(&pointer_payload, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_init(&frame_payload, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&frame_payload, "{sv}", "x", g_variant_new_int32(frame.x));
    g_variant_builder_add(&frame_payload, "{sv}", "y", g_variant_new_int32(frame.y));
    g_variant_builder_add(&frame_payload, "{sv}", "width", g_variant_new_int32(frame.width));
    g_variant_builder_add(&frame_payload, "{sv}", "height", g_variant_new_int32(frame.height));
    g_variant_builder_add(&window_payload, "{sv}", "id", g_variant_new_string(window_id));
    g_variant_builder_add(&window_payload, "{sv}", "frame", g_variant_builder_end(&frame_payload));
    g_variant_builder_add(&window_payload, "{sv}", "movable",
                          g_variant_new_boolean(meta_window_allows_move(window)));
    g_variant_builder_add(&window_payload, "{sv}", "resizable",
                          g_variant_new_boolean(meta_window_allows_resize(window)));
    g_variant_builder_add(&pointer_payload, "{sv}", "x", g_variant_new_double(context->pointer_x));
    g_variant_builder_add(&pointer_payload, "{sv}", "y", g_variant_new_double(context->pointer_y));
    g_variant_builder_add(&pointer_payload, "{sv}", "button", g_variant_new_uint32(button));
    g_variant_builder_add(&payload, "{sv}", "binding", g_variant_new_string(binding));
    g_variant_builder_add(&payload, "{sv}", "window", g_variant_builder_end(&window_payload));
    g_variant_builder_add(&payload, "{sv}", "pointer", g_variant_builder_end(&pointer_payload));
    g_variant_builder_add(&payload, "{sv}", "focus_context_handle", g_variant_new_uint64(handle));
    g_variant_builder_add(&payload, "{sv}", "focus_context_generation",
                          g_variant_new_uint64(generation));
    g_variant_builder_add(&payload, "{sv}", "focus_context_expires_at_us",
                          g_variant_new_int64(context->expires_at_us));
    g_autoptr(GVariant) event_payload = g_variant_ref_sink(g_variant_builder_end(&payload));
    if (!native_runtime_dispatch_event(control, "gnoblin.pointer.binding-activated", event_payload))
        g_hash_table_remove(control->focus_contexts, &handle);
}

void native_trusted_shortcut_activated(MetaDisplay* display, guint action,
                                       const ClutterEvent* event, gpointer user_data) {
    GnoblinNativeControl* control = user_data;
    if (!control || control->stopping || !event || !control->shortcuts)
        return;
    if (control->wayland_compositor &&
        meta_wayland_session_lock_is_active(control->wayland_compositor))
        return;
    ClutterEventFlags flags = clutter_event_get_flags(event);
    if (clutter_event_type(event) != CLUTTER_KEY_PRESS ||
        (flags & (CLUTTER_EVENT_FLAG_SYNTHETIC | CLUTTER_EVENT_FLAG_INPUT_METHOD)))
        return;

    if (flags & CLUTTER_EVENT_FLAG_REPEATED) {
        dispatch_dynamic_shortcut_repeat(control, action, event);
        return;
    }

    dispatch_dynamic_shortcut_activated(control, action, event);

    guint64 generation = native_config_generation(control);
    if (generation == 0)
        return;
    gint64 now = g_get_monotonic_time();
    prune_focus_contexts(control, now);

    for (guint i = 0; i < control->shortcuts->len; i++) {
        NativeShortcut* shortcut = g_ptr_array_index(control->shortcuts, i);
        if (shortcut->overlay || shortcut->release || shortcut->action != action)
            continue;
        if (g_hash_table_size(control->focus_contexts) >= MAX_FOCUS_CONTEXTS)
            break;

        guint64 handle = ++control->next_focus_context_handle;
        if (handle == 0)
            handle = ++control->next_focus_context_handle;
        guint64* key_copy = g_new(guint64, 1);
        *key_copy = handle;
        NativeFocusContext* context = g_new0(NativeFocusContext, 1);
        context->generation = generation;
        context->expires_at_us = now + FOCUS_CONTEXT_LIFETIME_US;
        context->timestamp = clutter_event_get_time(event);
        capture_focus_identity(control, context, event);
        g_hash_table_insert(control->focus_contexts, key_copy, context);

        GVariantBuilder payload_builder;
        g_variant_builder_init(&payload_builder, G_VARIANT_TYPE_VARDICT);
        g_variant_builder_add(&payload_builder, "{sv}", "shortcut",
                              g_variant_new_string(shortcut->name));
        g_variant_builder_add(&payload_builder, "{sv}", "trigger", g_variant_new_string("press"));
        g_autoptr(GVariant) payload = g_variant_ref_sink(g_variant_builder_end(&payload_builder));
        meta_display_dispatch_gnoblin_shortcut_event(display, "gnoblin.shortcut.activated", payload,
                                                     handle, generation, context->expires_at_us);
        if (g_hash_table_contains(control->focus_contexts, &handle))
            publish_shortcut_focus_event(control, shortcut->name, handle, generation,
                                         context->expires_at_us);
    }
}

gboolean focus_token_random(char token[65]) {
    static const char hex[] = "0123456789abcdef";
    guint8 bytes[32];
    gsize offset = 0;
    while (offset < sizeof(bytes)) {
        ssize_t count = getrandom(bytes + offset, sizeof(bytes) - offset, 0);
        if (count < 0 && errno == EINTR)
            continue;
        if (count <= 0)
            return FALSE;
        offset += (gsize)count;
    }
    for (gsize i = 0; i < sizeof(bytes); i++) {
        token[i * 2] = hex[bytes[i] >> 4];
        token[i * 2 + 1] = hex[bytes[i] & 0x0f];
    }
    token[64] = '\0';
    return TRUE;
}

static gboolean focus_token_exists(GnoblinNativeControl* control, const char* token) {
    GHashTableIter clients;
    gpointer client_value;
    g_hash_table_iter_init(&clients, control->clients);
    while (g_hash_table_iter_next(&clients, &client_value, NULL)) {
        Client* client = client_value;
        if (client->focus_grants && g_hash_table_contains(client->focus_grants, token))
            return TRUE;
    }
    return FALSE;
}

void revoke_focus_grants_for_handle(GnoblinNativeControl* control, guint64 handle) {
    if (!control || !control->clients)
        return;
    GHashTableIter clients;
    gpointer client_value;
    g_hash_table_iter_init(&clients, control->clients);
    while (g_hash_table_iter_next(&clients, &client_value, NULL)) {
        Client* client = client_value;
        if (!client->focus_grants)
            continue;
        GHashTableIter grants;
        gpointer grant_value;
        g_hash_table_iter_init(&grants, client->focus_grants);
        while (g_hash_table_iter_next(&grants, NULL, &grant_value)) {
            NativeFocusGrant* grant = grant_value;
            if (grant->handle == handle)
                g_hash_table_iter_remove(&grants);
        }
    }
}

void revoke_focus_contexts(GnoblinNativeControl* control) {
    if (!control)
        return;
    if (control->focus_contexts)
        g_hash_table_remove_all(control->focus_contexts);
    revoke_text_targets(control);
    if (control->snap_contexts)
        g_hash_table_remove_all(control->snap_contexts);
    revoke_menu_contexts(control);
    if (!control->clients)
        return;
    GHashTableIter clients;
    gpointer client_value;
    g_hash_table_iter_init(&clients, control->clients);
    while (g_hash_table_iter_next(&clients, &client_value, NULL)) {
        Client* client = client_value;
        if (client->focus_grants)
            g_hash_table_remove_all(client->focus_grants);
    }
}

void native_menu_context_free(gpointer data) {
    NativeMenuContext* context = data;
    if (!context)
        return;
    g_free(context->window_id);
    g_free(context);
}

void revoke_menu_contexts(GnoblinNativeControl* control) {
    if (!control)
        return;
    if (control->menu_contexts)
        g_hash_table_remove_all(control->menu_contexts);
    if (!control->clients)
        return;
    GHashTableIter clients;
    gpointer value;
    g_hash_table_iter_init(&clients, control->clients);
    while (g_hash_table_iter_next(&clients, &value, NULL)) {
        Client* client = value;
        if (client->menu_grants)
            g_hash_table_remove_all(client->menu_grants);
    }
}

guint64 native_menu_context_create(GnoblinNativeControl* control, const char* window_id,
                                   guint64 socket_owner_client_id, gint64* expires_at_us) {
    if (!control || control->stopping || !control->menu_contexts || !window_id || !*window_id ||
        !control->wayland_compositor ||
        meta_wayland_session_lock_is_active(control->wayland_compositor))
        return 0;
    gint64 now = g_get_monotonic_time();
    GHashTableIter stale;
    gpointer stale_value;
    g_hash_table_iter_init(&stale, control->menu_contexts);
    while (g_hash_table_iter_next(&stale, NULL, &stale_value)) {
        NativeMenuContext* old = stale_value;
        if (old->expires_at_us <= now)
            g_hash_table_iter_remove(&stale);
    }
    if (g_hash_table_size(control->menu_contexts) >= MAX_MENU_CONTEXTS)
        return 0;
    guint64 handle = ++control->next_menu_context_handle;
    if (!handle)
        handle = ++control->next_menu_context_handle;
    guint64* key = g_new(guint64, 1);
    *key = handle;
    gint64 expires = now + 5 * G_USEC_PER_SEC;
    NativeMenuContext* context = g_new0(NativeMenuContext, 1);
    context->window_id = g_strdup(window_id);
    context->generation = native_config_generation(control);
    context->expires_at_us = expires;
    context->socket_owner_client_id = socket_owner_client_id;
    g_hash_table_insert(control->menu_contexts, key, context);
    if (expires_at_us)
        *expires_at_us = expires;
    return handle;
}

static void native_menu_context_revoke_client(GnoblinNativeControl* control, guint64 client_id) {
    if (!control || !control->menu_contexts || !client_id)
        return;
    GHashTableIter contexts;
    gpointer value;
    g_hash_table_iter_init(&contexts, control->menu_contexts);
    while (g_hash_table_iter_next(&contexts, NULL, &value)) {
        NativeMenuContext* context = value;
        if (context->socket_owner_client_id == client_id)
            g_hash_table_iter_remove(&contexts);
    }
}

void native_menu_prune_client_grants(Client* client, gint64 now) {
    if (!client || !client->menu_grants || !client->control)
        return;
    GHashTableIter grants;
    gpointer key, value;
    g_hash_table_iter_init(&grants, client->menu_grants);
    while (g_hash_table_iter_next(&grants, &key, &value)) {
        NativeMenuGrant* grant = value;
        if (grant->expires_at_us <= now) {
            g_hash_table_remove(client->control->menu_contexts, &grant->handle);
            g_hash_table_iter_remove(&grants);
        }
    }
}

void revoke_text_targets(GnoblinNativeControl* control) {
    if (control && control->text_targets)
        g_hash_table_remove_all(control->text_targets);
}

void native_socket_revoke_client_tokens(Client* client) {
    if (!client || !client->control)
        return;
    GnoblinNativeControl* control = client->control;
    native_menu_context_revoke_client(control, client->client_id);
    if (client->menu_grants)
        g_hash_table_remove_all(client->menu_grants);
    if (control->text_targets) {
        GHashTableIter targets;
        gpointer value;
        g_hash_table_iter_init(&targets, control->text_targets);
        while (g_hash_table_iter_next(&targets, NULL, &value)) {
            NativeTextTarget* target = value;
            if (target->socket_owner_client_id == client->client_id)
                g_hash_table_iter_remove(&targets);
        }
    }
    if (control->snap_contexts) {
        GHashTableIter contexts;
        gpointer value;
        g_hash_table_iter_init(&contexts, control->snap_contexts);
        while (g_hash_table_iter_next(&contexts, NULL, &value)) {
            NativeSnapContext* context = value;
            if (context->socket_owner_client_id == client->client_id)
                g_hash_table_iter_remove(&contexts);
        }
    }
}

void prune_focus_contexts(GnoblinNativeControl* control, gint64 now) {
    if (!control || !control->focus_contexts)
        return;
    GHashTableIter contexts;
    gpointer key, value;
    g_hash_table_iter_init(&contexts, control->focus_contexts);
    while (g_hash_table_iter_next(&contexts, &key, &value)) {
        NativeFocusContext* context = value;
        if (context->expires_at_us <= now) {
            guint64 handle = *(guint64*)key;
            g_hash_table_iter_remove(&contexts);
            revoke_focus_grants_for_handle(control, handle);
        }
    }

    if (control->text_targets) {
        GHashTableIter targets;
        gpointer target_value;
        g_hash_table_iter_init(&targets, control->text_targets);
        while (g_hash_table_iter_next(&targets, NULL, &target_value)) {
            NativeTextTarget* target = target_value;
            guint64 surface_id = 0, client_id = 0, focus_epoch = 0;
            graphene_rect_t caret;
            gboolean valid = target->expires_at_us > now &&
                             target->generation == native_config_generation(control) &&
                             control->wayland_compositor && control->wayland_compositor->seat &&
                             control->wayland_compositor->seat->text_input &&
                             !meta_wayland_session_lock_is_active(control->wayland_compositor) &&
                             meta_wayland_text_input_get_gnoblin_state(
                                 control->wayland_compositor->seat->text_input, &surface_id,
                                 &client_id, &focus_epoch, &caret) &&
                             surface_id == target->surface_id && client_id == target->client_id &&
                             focus_epoch == target->focus_epoch;
            if (!valid)
                g_hash_table_iter_remove(&targets);
        }
    }

    if (control->snap_contexts) {
        GHashTableIter snap_contexts;
        gpointer snap_value;
        g_hash_table_iter_init(&snap_contexts, control->snap_contexts);
        while (g_hash_table_iter_next(&snap_contexts, NULL, &snap_value)) {
            NativeSnapContext* context = snap_value;
            if (context->expires_at_us <= now ||
                context->native_generation != native_config_generation(control) ||
                context->settings_revision != native_config_revision(control))
                g_hash_table_iter_remove(&snap_contexts);
        }
    }

    GHashTableIter clients;
    gpointer client_value;
    g_hash_table_iter_init(&clients, control->clients);
    while (g_hash_table_iter_next(&clients, &client_value, NULL)) {
        Client* client = client_value;
        if (!client->focus_grants)
            continue;
        GHashTableIter grants;
        gpointer grant_value;
        g_hash_table_iter_init(&grants, client->focus_grants);
        while (g_hash_table_iter_next(&grants, NULL, &grant_value)) {
            NativeFocusGrant* grant = grant_value;
            if (grant->expires_at_us <= now ||
                !g_hash_table_contains(control->focus_contexts, &grant->handle))
                g_hash_table_iter_remove(&grants);
        }
    }
}

gboolean focus_context_expiry_tick(gpointer user_data) {
    GnoblinNativeControl* control = user_data;
    if (!control || control->stopping)
        return G_SOURCE_REMOVE;
    prune_focus_contexts(control, g_get_monotonic_time());
    return G_SOURCE_CONTINUE;
}

static void publish_shortcut_focus_event(GnoblinNativeControl* control, const char* shortcut,
                                         guint64 handle, guint64 generation, gint64 expires_at_us) {
    if (!control || control->stopping || !shortcut ||
        (control->wayland_compositor &&
         meta_wayland_session_lock_is_active(control->wayland_compositor)))
        return;

    GList* clients = g_hash_table_get_keys(control->clients);
    for (GList* item = clients; item; item = item->next) {
        Client* client = item->data;
        if (client->closing || client->event_api_minor < 10 || !client->event_subscriptions ||
            !g_hash_table_contains(client->event_subscriptions, "gnoblin.shortcut.activated"))
            continue;
        prune_focus_contexts(control, g_get_monotonic_time());
        if (!g_hash_table_contains(control->focus_contexts, &handle) ||
            expires_at_us <= g_get_monotonic_time())
            break;
        if (!client->focus_grants || g_hash_table_size(client->focus_grants) >= MAX_FOCUS_CONTEXTS)
            continue;

        char token[65];
        gboolean unique = FALSE;
        for (guint attempt = 0; attempt < 4; attempt++) {
            if (!focus_token_random(token))
                break;
            if (!focus_token_exists(control, token)) {
                unique = TRUE;
                break;
            }
        }
        if (!unique)
            continue;

        NativeFocusGrant* grant = g_new0(NativeFocusGrant, 1);
        grant->handle = handle;
        grant->generation = generation;
        grant->expires_at_us = expires_at_us;
        g_hash_table_insert(client->focus_grants, g_strdup(token), grant);

        JsonObject* object = json_object_new();
        json_object_set_string_member(object, "event", "gnoblin.shortcut.activated");
        json_object_set_string_member(object, "shortcut", shortcut);
        json_object_set_string_member(object, "trigger", "press");
        json_object_set_string_member(object, "focus_context", token);
        json_object_set_int_member(object, "revision", control->state_revision);
        json_object_set_int_member(object, "sequence", ++control->event_sequence);
        json_object_set_int_member(object, "time", g_get_monotonic_time());
        g_autoptr(JsonNode) event = json_node_new(JSON_NODE_OBJECT);
        json_node_take_object(event, object);
        g_autofree char* encoded = json_to_string(event, FALSE);
        send_response(client, g_strconcat(encoded, "\n", NULL));
    }
    g_list_free(clients);
}

static gboolean dynamic_shortcut_owner_subscribed(NativeDynamicShortcut* shortcut,
                                                  const char* event) {
    return shortcut && shortcut->client && !shortcut->client->closing &&
           shortcut->client->event_api_minor >= 9 && shortcut->client->event_subscriptions &&
           g_hash_table_contains(shortcut->client->event_subscriptions, event);
}

static gboolean dynamic_shortcut_routes_to_lua_runtime(NativeDynamicShortcut* shortcut) {
    return shortcut && shortcut->owner_id &&
           (g_str_has_prefix(shortcut->owner_id, "lua:") ||
            g_str_has_prefix(shortcut->owner_id, "config:"));
}

void dynamic_shortcut_publish_event(GnoblinNativeControl* control, NativeDynamicShortcut* shortcut,
                                    const char* event, GVariant* payload) {
    if (!control || !shortcut || !event || !payload ||
        !g_variant_is_of_type(payload, G_VARIANT_TYPE_VARDICT))
        return;
    if (dynamic_shortcut_routes_to_lua_runtime(shortcut)) {
        native_runtime_dispatch_event(control, event, payload);
        return;
    }
    if (!dynamic_shortcut_owner_subscribed(shortcut, event))
        return;
    g_autoptr(JsonNode) json = json_from_variant(payload);
    if (!json || !JSON_NODE_HOLDS_OBJECT(json))
        return;
    JsonObject* object = json_node_get_object(json);
    json_object_set_string_member(object, "event", event);
    g_autofree char* encoded = json_to_string(json, FALSE);
    send_response(shortcut->client, g_strconcat(encoded, "\n", NULL));
}

gboolean issue_focus_context_for_session_key(GnoblinNativeControl* control,
                                             NativeDynamicShortcut* shortcut,
                                             const ClutterEvent* event, guint64* handle_out,
                                             guint64* generation_out, gint64* expires_at_us_out,
                                             char token_out[65]) {
    if (handle_out)
        *handle_out = 0;
    if (generation_out)
        *generation_out = 0;
    if (expires_at_us_out)
        *expires_at_us_out = 0;
    if (token_out)
        token_out[0] = '\0';

    if (!control || control->stopping || !control->focus_contexts || !shortcut ||
        !shortcut->active || control->active_shortcut_session != shortcut || !event ||
        (clutter_event_type(event) != CLUTTER_KEY_PRESS &&
         clutter_event_type(event) != CLUTTER_KEY_RELEASE))
        return FALSE;

    ClutterEventFlags flags = clutter_event_get_flags(event);
    if (flags & (CLUTTER_EVENT_FLAG_SYNTHETIC | CLUTTER_EVENT_FLAG_INPUT_METHOD |
                 CLUTTER_EVENT_FLAG_REPEATED))
        return FALSE;
    if (control->wayland_compositor &&
        meta_wayland_session_lock_is_active(control->wayland_compositor))
        return FALSE;

    Client* client = shortcut->client;
    if ((!client && !dynamic_shortcut_routes_to_lua_runtime(shortcut)) ||
        (client && (client->closing || client->api_minor < 68 || !client->focus_grants ||
                    g_hash_table_size(client->focus_grants) >= MAX_FOCUS_CONTEXTS)))
        return FALSE;

    guint64 generation = native_config_generation(control);
    gint64 now = g_get_monotonic_time();
    if (!generation)
        return FALSE;
    prune_focus_contexts(control, now);
    if (g_hash_table_size(control->focus_contexts) >= MAX_FOCUS_CONTEXTS)
        return FALSE;

    char token[65] = {0};
    if (client) {
        gboolean unique = FALSE;
        for (guint attempt = 0; attempt < 4; attempt++) {
            if (!focus_token_random(token))
                break;
            if (!focus_token_exists(control, token)) {
                unique = TRUE;
                break;
            }
        }
        if (!unique)
            return FALSE;
    }

    guint64 handle = ++control->next_focus_context_handle;
    if (handle == 0)
        handle = ++control->next_focus_context_handle;
    guint64* key_copy = g_new(guint64, 1);
    *key_copy = handle;
    NativeFocusContext* context = g_new0(NativeFocusContext, 1);
    context->generation = generation;
    context->expires_at_us = now + FOCUS_CONTEXT_LIFETIME_US;
    context->timestamp = clutter_event_get_time(event);
    capture_focus_identity(control, context, event);
    g_hash_table_insert(control->focus_contexts, key_copy, context);

    if (client) {
        NativeFocusGrant* grant = g_new0(NativeFocusGrant, 1);
        grant->handle = handle;
        grant->generation = generation;
        grant->expires_at_us = context->expires_at_us;
        g_hash_table_insert(client->focus_grants, g_strdup(token), grant);
    }

    if (handle_out)
        *handle_out = handle;
    if (generation_out)
        *generation_out = generation;
    if (expires_at_us_out)
        *expires_at_us_out = context->expires_at_us;
    if (token_out && client)
        g_strlcpy(token_out, token, 65);
    return TRUE;
}

void dynamic_shortcut_end_session(GnoblinNativeControl* control, NativeDynamicShortcut* shortcut,
                                  const char* reason) {
    if (!control || !shortcut || !shortcut->active)
        return;
    shortcut->active = FALSE;
    if (shortcut->timeout_id) {
        g_source_remove(shortcut->timeout_id);
        shortcut->timeout_id = 0;
    }
    if (control->active_shortcut_session == shortcut) {
        if (control->shortcut_session_capture_active) {
            control->shortcut_session_capture_active = FALSE;
            meta_display_stop_native_key_capture(control->display);
            meta_display_unregister_native_key_capture_handler(control->display, control);
        }
        control->active_shortcut_session = NULL;
    }
    GVariantBuilder builder;
    g_variant_builder_init(&builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&builder, "{sv}", "id", g_variant_new_string(shortcut->id));
    g_variant_builder_add(&builder, "{sv}", "session_id",
                          g_variant_new_uint64(shortcut->session_id));
    g_variant_builder_add(&builder, "{sv}", "reason",
                          g_variant_new_string(reason ? reason : "cancelled"));
    g_variant_builder_add(&builder, "{sv}", "time", g_variant_new_int64(g_get_monotonic_time()));
    g_autoptr(GVariant) payload = g_variant_ref_sink(g_variant_builder_end(&builder));
    dynamic_shortcut_publish_event(control, shortcut, "gnoblin.shortcut.session.ended", payload);
}

void gnoblin_control_dynamic_shortcut_end_session(GnoblinNativeControl* control,
                                                  NativeDynamicShortcut* shortcut,
                                                  const char* reason) {
    dynamic_shortcut_end_session(control, shortcut, reason);
}

gboolean native_shortcut_session_timeout(gpointer user_data) {
    NativeDynamicShortcut* shortcut = user_data;
    GnoblinNativeControl* control = shortcut ? shortcut->control : NULL;
    if (shortcut)
        shortcut->timeout_id = 0;
    if (control)
        dynamic_shortcut_end_session(control, shortcut, "timed_out");
    return G_SOURCE_REMOVE;
}

gboolean apply_configured_capture_shortcut(GnoblinNativeControl* control, GVariant* document,
                                           GError** error) {
    g_autoptr(GVariant) declarations =
        document ? g_variant_lookup_value(document, "shortcuts", G_VARIANT_TYPE("av")) : NULL;
    guint capture_count = 0;
    g_autofree char* capture_name = NULL;
    if (!declarations)
        return TRUE;
    for (gsize i = 0; i < g_variant_n_children(declarations); i++) {
        g_autoptr(GVariant) boxed = g_variant_get_child_value(declarations, i);
        g_autoptr(GVariant) entry = g_variant_get_variant(boxed);
        g_autoptr(GVariant) capture = g_variant_lookup_value(entry, "capture-input", NULL);
        if (!capture)
            continue;
        if (!g_variant_is_of_type(capture, G_VARIANT_TYPE_BOOLEAN)) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                                "capture_input must be a boolean");
            return FALSE;
        }
        if (!g_variant_get_boolean(capture))
            continue;
        const char* name = NULL;
        const char* binding = NULL;
        const char* trigger = "press";
        g_autoptr(GVariant) command = g_variant_lookup_value(entry, "command", NULL);
        g_autoptr(GVariant) action = g_variant_lookup_value(entry, "action", NULL);
        g_autoptr(GVariant) trigger_value = g_variant_lookup_value(entry, "trigger", NULL);
        if (!g_variant_lookup(entry, "name", "&s", &name) ||
            !g_variant_lookup(entry, "binding", "&s", &binding)) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                                "capture_input shortcut needs a name and binding");
            return FALSE;
        }
        if (trigger_value && g_variant_is_of_type(trigger_value, G_VARIANT_TYPE_STRING))
            trigger = g_variant_get_string(trigger_value, NULL);
        capture_count++;
        if (command || action || !g_str_equal(binding, "Super") ||
            !g_str_equal(trigger, "release") || capture_count > 1) {
            g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                        "shortcuts.%s.capture_input requires the sole command-free, action-free "
                        "bare Super binding with trigger='release'",
                        name);
            return FALSE;
        }
        capture_name = g_strdup(name);
    }
    if (capture_name) {
        g_autofree char* owner_id =
            g_strdup_printf("config:%" G_GUINT64_FORMAT, native_config_generation(control));
        guint64 session_id = ++control->next_shortcut_session_id;
        if (!session_id)
            session_id = ++control->next_shortcut_session_id;
        if (!gnoblin_control_arm_bare_super_shortcut(control, capture_name, owner_id, NULL,
                                                     session_id, error))
            return FALSE;
    }
    return TRUE;
}

void dispatch_dynamic_shortcut_activated(GnoblinNativeControl* control, guint action,
                                         const ClutterEvent* event) {
    if (!control || control->stopping || !event || !control->dynamic_shortcuts ||
        (control->wayland_compositor &&
         meta_wayland_session_lock_is_active(control->wayland_compositor)))
        return;
    NativeDynamicShortcut* shortcut =
        action ? g_hash_table_lookup(control->dynamic_shortcuts, GUINT_TO_POINTER(action))
               : control->bare_super_shortcut;
    if (!shortcut || !shortcut->armed ||
        (shortcut->client && (shortcut->client->closing || !shortcut->client->control ||
                              shortcut->client->api_minor < 11)))
        return;
    Client* client = shortcut->client;
    gboolean starts_session = shortcut->hold_mask || shortcut->modal || shortcut->capture_input;
    gboolean first_activation =
        !starts_session || !shortcut->active || !shortcut->activation_dispatched;
    gboolean wants_binding_event = !client || dynamic_shortcut_owner_subscribed(
                                                  shortcut, "gnoblin.shortcut.binding-activated");
    gboolean wants_session_event =
        !client ||
        dynamic_shortcut_owner_subscribed(shortcut, "gnoblin.shortcut.session.activated") ||
        dynamic_shortcut_owner_subscribed(shortcut, "gnoblin.shortcut.session.key") ||
        dynamic_shortcut_owner_subscribed(shortcut, "gnoblin.shortcut.session.ended");
    if ((starts_session && !wants_session_event && !wants_binding_event) ||
        (!starts_session && !wants_binding_event))
        return;
    if (shortcut->trigger_release && !shortcut->releasing) {
        if (shortcut->trigger_event)
            clutter_event_free(shortcut->trigger_event);
        shortcut->trigger_event = clutter_event_copy(event);
        return;
    }
    guint64 generation = native_config_generation(control);
    gint64 now = g_get_monotonic_time();
    prune_focus_contexts(control, now);
    gboolean issue_focus_context =
        generation != 0 && (!client || wants_binding_event) &&
        g_hash_table_size(control->focus_contexts) < MAX_FOCUS_CONTEXTS &&
        (!client ||
         (client->focus_grants && g_hash_table_size(client->focus_grants) < MAX_FOCUS_CONTEXTS));

    guint64 handle = 0;
    NativeFocusContext* context = NULL;
    if (issue_focus_context) {
        handle = ++control->next_focus_context_handle;
        if (handle == 0)
            handle = ++control->next_focus_context_handle;
        guint64* key_copy = g_new(guint64, 1);
        *key_copy = handle;
        context = g_new0(NativeFocusContext, 1);
        context->generation = generation;
        context->expires_at_us = now + FOCUS_CONTEXT_LIFETIME_US;
        context->timestamp = clutter_event_get_time(event);
        capture_focus_identity(control, context, event);
        g_hash_table_insert(control->focus_contexts, key_copy, context);
    }

    guint64 session_id = shortcut->session_id;
    if (starts_session && !shortcut->active) {
        session_id = ++control->next_shortcut_session_id;
        if (session_id == 0)
            session_id = ++control->next_shortcut_session_id;
        shortcut->session_id = session_id;
        shortcut->active = TRUE;
        shortcut->activation_dispatched = FALSE;
        shortcut->held_mask = shortcut->hold_mask;
        control->active_shortcut_session = shortcut;
        shortcut->timeout_id = g_timeout_add_seconds(10, native_shortcut_session_timeout, shortcut);
    }

    char token[65] = {0};
    if (client && wants_binding_event) {
        gboolean unique = FALSE;
        if (issue_focus_context) {
            for (guint attempt = 0; attempt < 4; attempt++) {
                if (!focus_token_random(token))
                    break;
                if (!focus_token_exists(control, token)) {
                    unique = TRUE;
                    break;
                }
            }
        }
        if (unique) {
            NativeFocusGrant* grant = g_new0(NativeFocusGrant, 1);
            grant->handle = handle;
            grant->generation = generation;
            grant->expires_at_us = context->expires_at_us;
            g_hash_table_insert(client->focus_grants, g_strdup(token), grant);
        } else if (handle) {
            g_hash_table_remove(control->focus_contexts, &handle);
            handle = 0;
            context = NULL;
        }
    }

    GVariantBuilder payload_builder;
    g_variant_builder_init(&payload_builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&payload_builder, "{sv}", "id", g_variant_new_string(shortcut->id));
    g_variant_builder_add(&payload_builder, "{sv}", "accelerator",
                          g_variant_new_string(shortcut->accelerator));
    g_variant_builder_add(&payload_builder, "{sv}", "trigger",
                          g_variant_new_string(shortcut->trigger_release ? "release" : "press"));
    g_variant_builder_add(&payload_builder, "{sv}", "first",
                          g_variant_new_boolean(first_activation));
    g_variant_builder_add(&payload_builder, "{sv}", "modifiers",
                          g_variant_new_uint32(clutter_event_get_state(event)));
    g_variant_builder_add(&payload_builder, "{sv}", "time", g_variant_new_int64(now));
    if (session_id)
        g_variant_builder_add(&payload_builder, "{sv}", "session_id",
                              g_variant_new_uint64(session_id));
    if (handle) {
        g_variant_builder_add(&payload_builder, "{sv}", "focus_context_handle",
                              g_variant_new_uint64(handle));
        g_variant_builder_add(&payload_builder, "{sv}", "focus_context_generation",
                              g_variant_new_uint64(generation));
        g_variant_builder_add(&payload_builder, "{sv}", "focus_context_expires_at_us",
                              g_variant_new_int64(context->expires_at_us));
    }
    g_autoptr(GVariant) payload = g_variant_ref_sink(g_variant_builder_end(&payload_builder));
    if (!client)
        dynamic_shortcut_publish_event(control, shortcut, "gnoblin.shortcut.binding-activated",
                                       payload);
    if (starts_session) {
        GVariantBuilder session_builder;
        g_variant_builder_init(&session_builder, G_VARIANT_TYPE_VARDICT);
        g_variant_builder_add(&session_builder, "{sv}", "id", g_variant_new_string(shortcut->id));
        g_variant_builder_add(&session_builder, "{sv}", "session_id",
                              g_variant_new_uint64(session_id));
        g_variant_builder_add(&session_builder, "{sv}", "first",
                              g_variant_new_boolean(first_activation));
        g_variant_builder_add(
            &session_builder, "{sv}", "trigger",
            g_variant_new_string(shortcut->trigger_release ? "release" : "press"));
        g_variant_builder_add(&session_builder, "{sv}", "modifiers",
                              g_variant_new_uint32(clutter_event_get_state(event)));
        g_variant_builder_add(&session_builder, "{sv}", "time", g_variant_new_int64(now));
        g_autoptr(GVariant) session_payload =
            g_variant_ref_sink(g_variant_builder_end(&session_builder));
        dynamic_shortcut_publish_event(control, shortcut, "gnoblin.shortcut.session.activated",
                                       session_payload);
        shortcut->activation_dispatched = TRUE;
        if ((shortcut->modal || shortcut->capture_input) &&
            !control->shortcut_session_capture_active) {
            if (control->shortcut_capture_active || control->active_shortcut_session != shortcut ||
                !meta_display_can_start_native_key_capture(control->display) ||
                !meta_display_register_native_key_capture_handler(
                    control->display, native_shortcut_capture_key, control, NULL)) {
                dynamic_shortcut_end_session(control, shortcut, "preempted");
                return;
            }
            control->shortcut_session_capture_active = TRUE;
            if (!meta_display_start_native_key_capture(control->display)) {
                control->shortcut_session_capture_active = FALSE;
                meta_display_unregister_native_key_capture_handler(control->display, control);
                dynamic_shortcut_end_session(control, shortcut, "preempted");
                return;
            }
        }
    }
    if (client && wants_binding_event) {
        JsonObject* object = json_object_new();
        json_object_set_string_member(object, "event", "gnoblin.shortcut.binding-activated");
        json_object_set_string_member(object, "id", shortcut->id);
        json_object_set_string_member(object, "accelerator", shortcut->accelerator);
        json_object_set_string_member(object, "trigger",
                                      shortcut->trigger_release ? "release" : "press");
        json_object_set_boolean_member(object, "first", first_activation);
        json_object_set_int_member(object, "modifiers", clutter_event_get_state(event));
        if (session_id)
            json_object_set_int_member(object, "session_id", session_id);
        if (token[0])
            json_object_set_string_member(object, "focus_context", token);
        json_object_set_int_member(object, "revision", control->state_revision);
        json_object_set_int_member(object, "sequence", ++control->event_sequence);
        json_object_set_int_member(object, "input_time", clutter_event_get_time(event));
        json_object_set_int_member(object, "time", now);
        g_autoptr(JsonNode) root = json_node_new(JSON_NODE_OBJECT);
        json_node_take_object(root, object);
        g_autofree char* encoded = json_to_string(root, FALSE);
        send_response(client, g_strconcat(encoded, "\n", NULL));
    }
    shortcut->activated = !shortcut->trigger_release;
}

static void dispatch_dynamic_shortcut_repeat(GnoblinNativeControl* control, guint action,
                                             const ClutterEvent* event) {
    NativeDynamicShortcut* shortcut =
        control && control->dynamic_shortcuts
            ? g_hash_table_lookup(control->dynamic_shortcuts, GUINT_TO_POINTER(action))
            : NULL;
    if (!shortcut || !shortcut->active || !event ||
        (control->wayland_compositor &&
         meta_wayland_session_lock_is_active(control->wayland_compositor)))
        return;
    GVariantBuilder builder;
    g_variant_builder_init(&builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&builder, "{sv}", "id", g_variant_new_string(shortcut->id));
    g_variant_builder_add(&builder, "{sv}", "session_id",
                          g_variant_new_uint64(shortcut->session_id));
    g_variant_builder_add(&builder, "{sv}", "first", g_variant_new_boolean(FALSE));
    g_variant_builder_add(&builder, "{sv}", "trigger",
                          g_variant_new_string(shortcut->trigger_release ? "release" : "press"));
    g_variant_builder_add(&builder, "{sv}", "modifiers",
                          g_variant_new_uint32(clutter_event_get_state(event)));
    g_variant_builder_add(&builder, "{sv}", "time",
                          g_variant_new_int64(clutter_event_get_time(event)));
    g_autoptr(GVariant) payload = g_variant_ref_sink(g_variant_builder_end(&builder));
    dynamic_shortcut_publish_event(control, shortcut, "gnoblin.shortcut.session.activated",
                                   payload);
}

void native_shortcut_deactivated(MetaDisplay* display, guint action, gpointer device,
                                 guint timestamp, gpointer user_data) {
    GnoblinNativeControl* control = user_data;
    NativeDynamicShortcut* dynamic =
        control && control->dynamic_shortcuts
            ? g_hash_table_lookup(control->dynamic_shortcuts, GUINT_TO_POINTER(action))
            : NULL;
    if (dynamic && dynamic->trigger_release && dynamic->trigger_event) {
        dynamic->releasing = TRUE;
        dispatch_dynamic_shortcut_activated(control, action, dynamic->trigger_event);
        dynamic->releasing = FALSE;
        clutter_event_free(dynamic->trigger_event);
        dynamic->trigger_event = NULL;
    }
    if (dynamic && !dynamic->trigger_release && dynamic->activated) {
        GVariantBuilder builder;
        g_variant_builder_init(&builder, G_VARIANT_TYPE_VARDICT);
        g_variant_builder_add(&builder, "{sv}", "id", g_variant_new_string(dynamic->id));
        g_variant_builder_add(&builder, "{sv}", "accelerator",
                              g_variant_new_string(dynamic->accelerator));
        g_variant_builder_add(&builder, "{sv}", "input_time", g_variant_new_uint32(timestamp));
        g_autoptr(GVariant) payload = g_variant_ref_sink(g_variant_builder_end(&builder));
        dynamic_shortcut_publish_event(control, dynamic, "gnoblin.shortcut.binding-deactivated",
                                       payload);
        dynamic->activated = FALSE;
    }
    /* A release-triggered binding may only become active in this callback,
     * after the hold key's physical release was already observed. Complete
     * that short session immediately, after publishing activation. */
    if (dynamic && dynamic->trigger_release && dynamic->active && dynamic->hold_mask &&
        !native_shortcut_hold_family_pressed(control, dynamic->hold_mask))
        dynamic_shortcut_end_session(control, dynamic, "released");
    for (guint i = 0; i < control->shortcuts->len; i++) {
        NativeShortcut* shortcut = g_ptr_array_index(control->shortcuts, i);
        if (!shortcut->overlay && shortcut->action == action && shortcut->release)
            gnoblin_control_launch_command("shortcut", shortcut->name, shortcut->argv);
    }
}

void gnoblin_native_control_observe_key_event(MetaDisplay* display, const ClutterEvent* event) {
    GnoblinNativeControl* control =
        display ? g_object_get_data(G_OBJECT(display), NATIVE_CONTROL_OBJECT_DATA_KEY) : NULL;
    if (!control || control->stopping || !event)
        return;

    ClutterEventType type = clutter_event_type(event);
    if (type != CLUTTER_KEY_PRESS && type != CLUTTER_KEY_RELEASE)
        return;
    ClutterEventFlags flags = clutter_event_get_flags(event);
    if (flags & (CLUTTER_EVENT_FLAG_SYNTHETIC | CLUTTER_EVENT_FLAG_INPUT_METHOD))
        return;

    gboolean pressed = type == CLUTTER_KEY_PRESS;
    gboolean* state = NULL;
    switch (clutter_event_get_key_symbol(event)) {
    case XKB_KEY_Super_L:
        state = &control->super_left_pressed;
        break;
    case XKB_KEY_Super_R:
        state = &control->super_right_pressed;
        break;
    case XKB_KEY_Control_L:
        state = &control->control_left_pressed;
        break;
    case XKB_KEY_Control_R:
        state = &control->control_right_pressed;
        break;
    case XKB_KEY_Alt_L:
        state = &control->alt_left_pressed;
        break;
    case XKB_KEY_Alt_R:
        state = &control->alt_right_pressed;
        break;
    default:
        return;
    }
    *state = pressed;

    NativeDynamicShortcut* shortcut = control->active_shortcut_session;
    if (!shortcut || !shortcut->active || !shortcut->hold_mask || shortcut->modal ||
        shortcut->capture_input)
        return;
    if (!native_shortcut_hold_family_pressed(control, shortcut->hold_mask))
        dynamic_shortcut_end_session(control, shortcut, "released");
}
