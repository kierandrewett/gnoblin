/* Window drag and snap: drag state, gnoblin.window.drag.* events, snap offers and the
 * compositor entry points the drag grab calls. The snap contexts and the release-time snap
 * commit stay in gnoblin-native-control.c with the focus-context code they share. */
#ifndef _GNU_SOURCE
#define _GNU_SOURCE
#endif
#include "config.h"

#include "core/gnoblin-control-internal.h"

#include <string.h>

#define NATIVE_DRAG_LIFETIME_US (5 * G_USEC_PER_SEC)
#define MAX_WINDOW_DRAG_CLIENTS 256

static void native_snap_target_free(gpointer data) {
    NativeSnapTarget* target = data;
    g_free(target->id);
    g_free(target);
}

void gnoblin_control_window_drag_free(gpointer data) {
    NativeWindowDrag* drag = data;
    g_free(drag->window_id);
    g_free(drag->monitor_id);
    g_free(drag->committed_target_id);
    g_clear_pointer(&drag->client_tokens, g_hash_table_unref);
    g_clear_pointer(&drag->targets, g_ptr_array_unref);
    g_free(drag);
}

void gnoblin_control_cancel_window_drags(GnoblinNativeControl* control, const char* reason) {
    if (!control || !control->window_drags)
        return;
    GArray* ids = g_array_new(FALSE, FALSE, sizeof(guint64));
    GHashTableIter iter;
    gpointer key;
    g_hash_table_iter_init(&iter, control->window_drags);
    while (g_hash_table_iter_next(&iter, &key, NULL)) {
        guint64 id = *(guint64*)key;
        g_array_append_val(ids, id);
    }
    for (guint i = 0; i < ids->len; i++)
        gnoblin_native_control_window_drag_end(control->display, g_array_index(ids, guint64, i),
                                               FALSE, reason);
    g_array_unref(ids);
}

void gnoblin_control_window_drag_client_disconnected(GnoblinNativeControl* control,
                                                     guint64 client_id) {
    if (!control || !control->window_drags)
        return;
    GHashTableIter iter;
    gpointer value;
    g_hash_table_iter_init(&iter, control->window_drags);
    while (g_hash_table_iter_next(&iter, NULL, &value)) {
        NativeWindowDrag* drag = value;
        if (drag->client_tokens)
            g_hash_table_remove(drag->client_tokens, &client_id);
        if (drag->socket_owner_client_id == client_id) {
            drag->socket_owner_client_id = 0;
            drag->runtime_offer_claimed = FALSE;
            g_ptr_array_set_size(drag->targets, 0);
        }
    }
}

gboolean gnoblin_control_drag_monitor_snapshot(GnoblinNativeControl* control, MetaWindow* window,
                                               char** monitor_id, MtkRectangle* monitor_rect,
                                               MtkRectangle* work_area) {
    int monitor_number = meta_window_get_monitor(window);
    gboolean found = FALSE;
    GHashTableIter iter;
    gpointer value;
    meta_window_get_work_area_for_monitor(window, monitor_number, work_area);
    if (control->monitor_state) {
        g_hash_table_iter_init(&iter, control->monitor_state);
        while (g_hash_table_iter_next(&iter, NULL, &value)) {
            NativeMonitorState* state = value;
            g_autoptr(JsonParser) parser = json_parser_new();
            if (!json_parser_load_from_data(parser, state->json, -1, NULL))
                continue;
            JsonObject* record = json_node_get_object(json_parser_get_root(parser));
            if (json_object_get_int_member_with_default(record, "index", -1) != monitor_number)
                continue;
            const char* id = json_object_get_string_member_with_default(record, "id", NULL);
            if (!id || !*id)
                continue;
            *monitor_id = g_strdup(id);
            *monitor_rect = (MtkRectangle){
                .x = (int)json_object_get_int_member(record, "x"),
                .y = (int)json_object_get_int_member(record, "y"),
                .width = (int)json_object_get_int_member(record, "width"),
                .height = (int)json_object_get_int_member(record, "height"),
            };
            found = monitor_rect->width > 0 && monitor_rect->height > 0;
            break;
        }
    }
    return found && work_area->width > 0 && work_area->height > 0;
}

gboolean gnoblin_control_window_drag_refresh(GnoblinNativeControl* control, NativeWindowDrag* drag,
                                             MetaWindow* window, int pointer_x, int pointer_y,
                                             guint32 modifiers) {
    g_autofree char* monitor_id = NULL;
    MtkRectangle monitor_rect, work_area;
    if (!window || meta_window_is_skip_taskbar(window) ||
        meta_window_is_override_redirect(window) ||
        !gnoblin_control_drag_monitor_snapshot(control, window, &monitor_id, &monitor_rect,
                                               &work_area))
        return FALSE;
    g_autofree char* window_id = gnoblin_control_native_window_id(window);
    if (drag->window_id && !g_str_equal(drag->window_id, window_id))
        return FALSE;
    g_free(drag->window_id);
    drag->window_id = g_steal_pointer(&window_id);
    g_free(drag->monitor_id);
    drag->monitor_id = g_steal_pointer(&monitor_id);
    meta_window_get_frame_rect(window, &drag->frame);
    drag->monitor = monitor_rect;
    drag->work_area = work_area;
    drag->pointer_x = pointer_x;
    drag->pointer_y = pointer_y;
    drag->modifiers = modifiers;
    drag->maximized = meta_window_is_maximized(window);
    drag->settings_revision = gnoblin_control_config_revision(control);
    drag->expires_at_us = g_get_monotonic_time() + NATIVE_DRAG_LIFETIME_US;
    return TRUE;
}

static GVariant* native_drag_rect_variant(const MtkRectangle* rect) {
    GVariantBuilder value;
    g_variant_builder_init(&value, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&value, "{sv}", "x", g_variant_new_int32(rect->x));
    g_variant_builder_add(&value, "{sv}", "y", g_variant_new_int32(rect->y));
    g_variant_builder_add(&value, "{sv}", "width", g_variant_new_int32(rect->width));
    g_variant_builder_add(&value, "{sv}", "height", g_variant_new_int32(rect->height));
    return g_variant_ref_sink(g_variant_builder_end(&value));
}

static GVariant* native_drag_event_payload(NativeWindowDrag* drag) {
    GVariantBuilder record, pointer, modifiers;
    g_variant_builder_init(&record, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_init(&pointer, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_init(&modifiers, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&record, "{sv}", "id", g_variant_new_uint64(drag->id));
    g_variant_builder_add(&record, "{sv}", "window_id", g_variant_new_string(drag->window_id));
    g_variant_builder_add(&record, "{sv}", "monitor_id", g_variant_new_string(drag->monitor_id));
    g_variant_builder_add(&record, "{sv}", "settings_revision",
                          g_variant_new_uint64(drag->settings_revision));
    g_variant_builder_add(&record, "{sv}", "pointer_x", g_variant_new_int32(drag->pointer_x));
    g_variant_builder_add(&record, "{sv}", "pointer_y", g_variant_new_int32(drag->pointer_y));
    g_variant_builder_add(&record, "{sv}", "maximized", g_variant_new_boolean(drag->maximized));
    g_variant_builder_add(&pointer, "{sv}", "x", g_variant_new_int32(drag->pointer_x));
    g_variant_builder_add(&pointer, "{sv}", "y", g_variant_new_int32(drag->pointer_y));
    g_variant_builder_add(&modifiers, "{sv}", "control",
                          g_variant_new_boolean((drag->modifiers & CLUTTER_CONTROL_MASK) != 0));
    g_variant_builder_add(&modifiers, "{sv}", "shift",
                          g_variant_new_boolean((drag->modifiers & CLUTTER_SHIFT_MASK) != 0));
    g_autoptr(GVariant) monitor = native_drag_rect_variant(&drag->monitor);
    g_autoptr(GVariant) work_area = native_drag_rect_variant(&drag->work_area);
    g_autoptr(GVariant) frame = native_drag_rect_variant(&drag->frame);
    g_variant_builder_add(&record, "{sv}", "monitor", monitor);
    g_variant_builder_add(&record, "{sv}", "work_area", work_area);
    g_variant_builder_add(&record, "{sv}", "frame", frame);
    g_variant_builder_add(&record, "{sv}", "pointer", g_variant_builder_end(&pointer));
    g_variant_builder_add(&record, "{sv}", "modifiers", g_variant_builder_end(&modifiers));
    return g_variant_ref_sink(g_variant_builder_end(&record));
}

gboolean gnoblin_control_rect_contains(const MtkRectangle* outer, const MtkRectangle* inner) {
    return inner->x >= outer->x && inner->y >= outer->y &&
           (gint64)inner->x + inner->width <= (gint64)outer->x + outer->width &&
           (gint64)inner->y + inner->height <= (gint64)outer->y + outer->height;
}

gboolean gnoblin_control_variant_rect(GVariant* value, MtkRectangle* rect) {
    return value && g_variant_is_of_type(value, G_VARIANT_TYPE_VARDICT) &&
           g_variant_n_children(value) == 4 && g_variant_lookup(value, "x", "i", &rect->x) &&
           g_variant_lookup(value, "y", "i", &rect->y) &&
           g_variant_lookup(value, "width", "i", &rect->width) &&
           g_variant_lookup(value, "height", "i", &rect->height) && rect->x >= -100000 &&
           rect->x <= 100000 && rect->y >= -100000 && rect->y <= 100000 && rect->width >= 1 &&
           rect->width <= 32768 && rect->height >= 1 && rect->height <= 32768;
}

static gboolean native_snap_target_parse(GVariant* value, NativeWindowDrag* drag,
                                         NativeSnapTarget** out_target) {
    const char* id = NULL;
    g_autoptr(GVariant) hit = NULL;
    g_autoptr(GVariant) frame = NULL;
    g_autoptr(GVariant) required = NULL;
    g_autoptr(GVariant) forbidden = NULL;
    g_autoptr(GVariant) maximize_value = NULL;
    gboolean maximize = FALSE;
    NativeSnapTarget* target = g_new0(NativeSnapTarget, 1);
    if (!value || !g_variant_is_of_type(value, G_VARIANT_TYPE_VARDICT))
        goto invalid;
    if (g_variant_n_children(value) < 3 || g_variant_n_children(value) > 6)
        goto invalid;
    GVariantIter fields;
    const char* field_name;
    GVariant* field_value;
    g_variant_iter_init(&fields, value);
    while (g_variant_iter_next(&fields, "{&sv}", &field_name, &field_value)) {
        g_autoptr(GVariant) owned_field = field_value;
        if (!g_str_equal(field_name, "id") && !g_str_equal(field_name, "hit") &&
            !g_str_equal(field_name, "frame") && !g_str_equal(field_name, "maximize") &&
            !g_str_equal(field_name, "required_modifiers") &&
            !g_str_equal(field_name, "forbidden_modifiers"))
            goto invalid;
    }
    hit = g_variant_lookup_value(value, "hit", G_VARIANT_TYPE_VARDICT);
    frame = g_variant_lookup_value(value, "frame", G_VARIANT_TYPE_VARDICT);
    required = g_variant_lookup_value(value, "required_modifiers", NULL);
    forbidden = g_variant_lookup_value(value, "forbidden_modifiers", NULL);
    if (!g_variant_lookup(value, "id", "&s", &id) || !id || !*id || strlen(id) > 64 ||
        !g_utf8_validate(id, -1, NULL) || !gnoblin_control_variant_rect(hit, &target->hit) ||
        !gnoblin_control_variant_rect(frame, &target->frame) ||
        !gnoblin_control_rect_contains(&drag->work_area, &target->hit) ||
        !gnoblin_control_rect_contains(&drag->work_area, &target->frame) ||
        (required && !g_variant_is_of_type(required, G_VARIANT_TYPE_STRING_ARRAY)) ||
        (forbidden && !g_variant_is_of_type(forbidden, G_VARIANT_TYPE_STRING_ARRAY)))
        goto invalid;
    if (required) {
        for (gsize i = 0; i < g_variant_n_children(required); i++) {
            const char* modifier = NULL;
            g_variant_get_child(required, i, "&s", &modifier);
            if (!g_str_equal(modifier, "control") ||
                (target->required_modifiers & NATIVE_DRAG_MOD_CONTROL))
                goto invalid;
            target->required_modifiers |= NATIVE_DRAG_MOD_CONTROL;
        }
    }
    if (forbidden) {
        for (gsize i = 0; i < g_variant_n_children(forbidden); i++) {
            const char* modifier = NULL;
            g_variant_get_child(forbidden, i, "&s", &modifier);
            if (!g_str_equal(modifier, "control") ||
                (target->forbidden_modifiers & NATIVE_DRAG_MOD_CONTROL))
                goto invalid;
            target->forbidden_modifiers |= NATIVE_DRAG_MOD_CONTROL;
        }
    }
    if ((target->required_modifiers & target->forbidden_modifiers) != 0)
        goto invalid;
    maximize_value = g_variant_lookup_value(value, "maximize", NULL);
    if (maximize_value && !g_variant_lookup(value, "maximize", "b", &maximize))
        goto invalid;
    target->maximize = maximize;
    target->id = g_strdup(id);
    *out_target = target;
    return TRUE;
invalid:
    native_snap_target_free(target);
    return FALSE;
}

gboolean gnoblin_control_runtime_window_snap_offer(GnoblinNativeControl* control,
                                                   GVariant* arguments, guint64 owner_generation,
                                                   GError** error) {
    guint64 drag_id = 0, settings_revision = 0;
    g_autoptr(GVariant) targets =
        g_variant_lookup_value(arguments, "targets", G_VARIANT_TYPE("av"));
    if (!g_variant_lookup(arguments, "drag_id", "t", &drag_id) || !drag_id ||
        !g_variant_lookup(arguments, "settings_revision", "t", &settings_revision) ||
        !owner_generation || owner_generation != control->runtime_generation ||
        settings_revision != gnoblin_control_config_revision(control) || !targets ||
        g_variant_n_children(targets) == 0 ||
        g_variant_n_children(targets) > NATIVE_DRAG_MAX_TARGETS) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "window.snap.offer requires a live drag, revision and 1-128 targets");
        return FALSE;
    }
    NativeWindowDrag* drag = g_hash_table_lookup(control->window_drags, &drag_id);
    if (!drag || drag->settings_revision != settings_revision ||
        drag->expires_at_us <= g_get_monotonic_time() ||
        drag->owner_generation != owner_generation || drag->socket_owner_client_id != 0) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "window.snap.offer belongs to an ended drag or stale runtime");
        return FALSE;
    }
    if (!gnoblin_control_snap_offer_set_targets(drag, targets, error))
        return FALSE;
    drag->owner_generation = owner_generation;
    drag->runtime_offer_claimed = TRUE;
    return TRUE;
}

static void native_publish_window_drag_socket_event(GnoblinNativeControl* control,
                                                    NativeWindowDrag* drag, const char* event_name,
                                                    GVariant* payload) {
    g_autoptr(JsonNode) base = gnoblin_control_json_from_variant(payload);
    if (!JSON_NODE_HOLDS_OBJECT(base))
        return;
    GList* clients = g_hash_table_get_keys(control->clients);
    for (GList* item = clients; item; item = item->next) {
        Client* client = item->data;
        if (client->closing || client->event_api_minor < 26 || !client->event_subscriptions ||
            !g_hash_table_contains(client->event_subscriptions, event_name))
            continue;
        const char* token = drag && drag->client_tokens
                                ? g_hash_table_lookup(drag->client_tokens, &client->client_id)
                                : NULL;
        if (!token && !g_str_equal(event_name, "gnoblin.window.drag.ended"))
            continue;
        g_autoptr(JsonNode) event = json_node_copy(base);
        JsonObject* object = json_node_get_object(event);
        json_object_set_string_member(object, "event", event_name);
        if (token)
            json_object_set_string_member(object, "drag_token", token);
        g_autofree char* encoded = json_to_string(event, FALSE);
        gnoblin_control_send_response(client, g_strconcat(encoded, "\n", NULL));
    }
    g_list_free(clients);
}

gboolean gnoblin_control_snap_offer_set_targets(NativeWindowDrag* drag, GVariant* targets,
                                                GError** error) {
    if (!targets || !g_variant_is_of_type(targets, G_VARIANT_TYPE("av")) ||
        g_variant_n_children(targets) == 0 ||
        g_variant_n_children(targets) > NATIVE_DRAG_MAX_TARGETS) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "window.snap.offer requires 1-128 targets");
        return FALSE;
    }
    GPtrArray* parsed = g_ptr_array_new_with_free_func(native_snap_target_free);
    g_autoptr(GHashTable) ids = g_hash_table_new(g_str_hash, g_str_equal);
    for (gsize i = 0; i < g_variant_n_children(targets); i++) {
        g_autoptr(GVariant) boxed = g_variant_get_child_value(targets, i);
        g_autoptr(GVariant) record = g_variant_get_variant(boxed);
        NativeSnapTarget* target = NULL;
        if (!native_snap_target_parse(record, drag, &target) ||
            g_hash_table_contains(ids, target ? target->id : "")) {
            native_snap_target_free(target);
            g_ptr_array_unref(parsed);
            g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                        "window.snap.offer target %zu is invalid or duplicated", i + 1);
            return FALSE;
        }
        g_hash_table_add(ids, target->id);
        g_ptr_array_add(parsed, target);
    }
    g_ptr_array_unref(drag->targets);
    drag->targets = parsed;
    drag->expires_at_us = g_get_monotonic_time() + NATIVE_DRAG_LIFETIME_US;
    return TRUE;
}

guint64 gnoblin_native_control_window_drag_begin(MetaDisplay* display, MetaWindow* window,
                                                 int pointer_x, int pointer_y, guint32 modifiers) {
    GnoblinNativeControl* control =
        display ? g_object_get_data(G_OBJECT(display), NATIVE_CONTROL_OBJECT_DATA_KEY) : NULL;
    if (!control || control->stopping || !control->supervised_runtime ||
        !control->runtime_generation || !window || !control->window_drags)
        return 0;
    gnoblin_control_cancel_window_drags(control, "preempted");
    NativeWindowDrag* drag = g_new0(NativeWindowDrag, 1);
    drag->id = ++control->next_window_drag_id;
    if (!drag->id)
        drag->id = ++control->next_window_drag_id;
    drag->owner_generation = control->runtime_generation;
    drag->targets = g_ptr_array_new_with_free_func(native_snap_target_free);
    drag->client_tokens = g_hash_table_new_full(g_int64_hash, g_int64_equal, g_free, g_free);
    if (!gnoblin_control_window_drag_refresh(control, drag, window, pointer_x, pointer_y,
                                             modifiers)) {
        gnoblin_control_window_drag_free(drag);
        return 0;
    }
    drag->original_frame = drag->frame;
    guint token_count = 0;
    GHashTableIter clients_iter;
    gpointer client_value;
    g_hash_table_iter_init(&clients_iter, control->clients);
    while (g_hash_table_iter_next(&clients_iter, NULL, &client_value) &&
           token_count < MAX_WINDOW_DRAG_CLIENTS) {
        Client* client = client_value;
        if (client->closing || client->api_minor < 26 || client->event_api_minor < 26 ||
            !client->event_subscriptions ||
            !g_hash_table_contains(client->event_subscriptions, "gnoblin.window.drag.started"))
            continue;
        char token[65];
        if (!gnoblin_control_focus_token_random(token))
            continue;
        guint64* client_key = g_new(guint64, 1);
        *client_key = client->client_id;
        g_hash_table_insert(drag->client_tokens, client_key, g_strdup(token));
        token_count++;
    }
    g_hash_table_remove_all(control->window_drags);
    g_hash_table_insert(control->window_drags, g_memdup2(&drag->id, sizeof(drag->id)), drag);
    g_autoptr(GVariant) payload = native_drag_event_payload(drag);
    gnoblin_control_dispatch_event(control, "gnoblin.window.drag.started", payload);
    native_publish_window_drag_socket_event(control, drag, "gnoblin.window.drag.started", payload);
    return drag->id;
}

void gnoblin_native_control_window_drag_update(MetaDisplay* display, guint64 drag_id,
                                               MetaWindow* window, int pointer_x, int pointer_y,
                                               guint32 modifiers) {
    GnoblinNativeControl* control =
        display ? g_object_get_data(G_OBJECT(display), NATIVE_CONTROL_OBJECT_DATA_KEY) : NULL;
    NativeWindowDrag* drag = control && control->window_drags
                                 ? g_hash_table_lookup(control->window_drags, &drag_id)
                                 : NULL;
    if (!drag || !gnoblin_control_window_drag_refresh(control, drag, window, pointer_x, pointer_y,
                                                      modifiers))
        return;
    g_autoptr(GVariant) payload = native_drag_event_payload(drag);
    gnoblin_control_dispatch_event(control, "gnoblin.window.drag.updated", payload);
    native_publish_window_drag_socket_event(control, drag, "gnoblin.window.drag.updated", payload);
}

void gnoblin_native_control_window_drag_end(MetaDisplay* display, guint64 drag_id,
                                            gboolean committed, const char* reason) {
    GnoblinNativeControl* control =
        display ? g_object_get_data(G_OBJECT(display), NATIVE_CONTROL_OBJECT_DATA_KEY) : NULL;
    NativeWindowDrag* drag = control && control->window_drags
                                 ? g_hash_table_lookup(control->window_drags, &drag_id)
                                 : NULL;
    if (!drag)
        return;
    GVariantBuilder result;
    g_variant_builder_init(&result, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&result, "{sv}", "drag_id", g_variant_new_uint64(drag_id));
    g_variant_builder_add(&result, "{sv}", "window_id", g_variant_new_string(drag->window_id));
    g_variant_builder_add(&result, "{sv}", "reason", g_variant_new_string(reason));
    g_variant_builder_add(&result, "{sv}", "committed", g_variant_new_boolean(committed));
    if (committed && drag->committed_target_id)
        g_variant_builder_add(&result, "{sv}", "target_id",
                              g_variant_new_string(drag->committed_target_id));
    g_autoptr(GVariant) payload = g_variant_ref_sink(g_variant_builder_end(&result));
    gnoblin_control_dispatch_event(control, "gnoblin.window.drag.ended", payload);
    native_publish_window_drag_socket_event(control, drag, "gnoblin.window.drag.ended", payload);
    g_hash_table_remove(control->window_drags, &drag_id);
}
