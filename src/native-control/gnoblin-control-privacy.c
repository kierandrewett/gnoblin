/* Privacy and location glue: screen-sharing, recording, microphone, camera and location state,
 * the privacy snapshot published to Lua, and the location authorization requests that wait for
 * a Lua answer. The PipeWire callback and the location.authorize_app operation stay in
 * gnoblin-native-control.c. */
#ifndef _GNU_SOURCE
#define _GNU_SOURCE
#endif
#include "config.h"

#include "core/gnoblin-control-internal.h"

#define MAX_PENDING_LOCATION_AUTHORIZATIONS 32
#define LOCATION_AUTHORIZATION_TIMEOUT_SECONDS 25

static GVariant* privacy_snapshot_new(GnoblinNativeControl* control) {
    GVariantBuilder available;
    GVariantBuilder snapshot;
    g_variant_builder_init(&available, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&available, "{sv}", "screen_sharing", g_variant_new_boolean(TRUE));
    g_variant_builder_add(&available, "{sv}", "recording", g_variant_new_boolean(TRUE));
    g_variant_builder_add(&available, "{sv}", "microphone_in_use",
                          g_variant_new_boolean(control->privacy_microphone_available));
    g_variant_builder_add(&available, "{sv}", "camera_in_use",
                          g_variant_new_boolean(control->privacy_camera_available));
    g_variant_builder_add(&available, "{sv}", "location_in_use",
                          g_variant_new_boolean(control->privacy_location_available));
    g_variant_builder_init(&snapshot, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&snapshot, "{sv}", "available", g_variant_builder_end(&available));
    g_variant_builder_add(&snapshot, "{sv}", "screen_sharing",
                          g_variant_new_boolean(control->privacy_screen_sharing));
    g_variant_builder_add(&snapshot, "{sv}", "recording",
                          g_variant_new_boolean(control->privacy_recording));
    if (control->privacy_microphone_available)
        g_variant_builder_add(&snapshot, "{sv}", "microphone_in_use",
                              g_variant_new_boolean(control->privacy_microphone_in_use));
    if (control->privacy_camera_available)
        g_variant_builder_add(&snapshot, "{sv}", "camera_in_use",
                              g_variant_new_boolean(control->privacy_camera_in_use));
    if (control->privacy_location_available)
        g_variant_builder_add(&snapshot, "{sv}", "location_in_use",
                              g_variant_new_boolean(control->privacy_location_in_use));
    return g_variant_ref_sink(g_variant_builder_end(&snapshot));
}

static void publish_privacy_snapshot(GnoblinNativeControl* control, gboolean changed);

static void privacy_refresh_state(GnoblinNativeControl* control) {
    gboolean screen_sharing = FALSE;
    gboolean recording = FALSE;
    GHashTableIter iter;
    gpointer key;
    if (!control || control->stopping)
        return;

    g_hash_table_iter_init(&iter, control->privacy_handles);
    while (g_hash_table_iter_next(&iter, &key, NULL)) {
        gboolean is_recording = FALSE;
        g_object_get(key, "is-recording", &is_recording, NULL);
        if (is_recording)
            recording = TRUE;
        else
            screen_sharing = TRUE;
    }

    if (screen_sharing == control->privacy_screen_sharing &&
        recording == control->privacy_recording)
        return;
    control->privacy_screen_sharing = screen_sharing;
    control->privacy_recording = recording;
    control->privacy_revision++;
    publish_privacy_snapshot(control, TRUE);
}

static void publish_privacy_snapshot(GnoblinNativeControl* control, gboolean changed) {
    if (!control || control->stopping)
        return;

    GVariant* snapshot = privacy_snapshot_new(control);
    g_clear_pointer(&control->privacy_snapshot, g_variant_unref);
    control->privacy_snapshot = snapshot;
    gnoblin_control_publish_runtime_snapshot(control, "privacy", control->privacy_snapshot,
                                    control->privacy_revision);

    if (!changed)
        return;

    GVariantBuilder state_builder;
    GVariantIter state_fields;
    const char* state_field;
    GVariant* state_value;
    g_variant_builder_init(&state_builder, G_VARIANT_TYPE_VARDICT);
    g_variant_iter_init(&state_fields, snapshot);
    while (g_variant_iter_next(&state_fields, "{&sv}", &state_field, &state_value)) {
        g_autoptr(GVariant) value = state_value;
        g_variant_builder_add(&state_builder, "{sv}", state_field, g_variant_ref(value));
    }
    g_variant_builder_add(&state_builder, "{sv}", "revision",
                          g_variant_new_int64((gint64)control->privacy_revision));
    g_autoptr(GVariant) state = g_variant_ref_sink(g_variant_builder_end(&state_builder));
    GVariantBuilder payload;
    g_variant_builder_init(&payload, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&payload, "{sv}", "state", state);
    g_variant_builder_add(&payload, "{sv}", "name",
                          g_variant_new_string("gnoblin.privacy.changed"));
    g_variant_builder_add(&payload, "{sv}", "revision",
                          g_variant_new_int64((gint64)control->privacy_revision));
    g_variant_builder_add(&payload, "{sv}", "sequence",
                          g_variant_new_int64((gint64)++control->event_sequence));
    g_variant_builder_add(&payload, "{sv}", "time", g_variant_new_int64(g_get_monotonic_time()));
    g_autoptr(GVariant) enriched = g_variant_ref_sink(g_variant_builder_end(&payload));
    gnoblin_control_dispatch_event(control, "gnoblin.privacy.changed", enriched);
    g_autoptr(JsonNode) json = gnoblin_control_json_from_variant(enriched);
    if (JSON_NODE_HOLDS_OBJECT(json))
        gnoblin_control_publish_socket_event(control, json);
}

void gnoblin_control_publish_privacy_snapshot(GnoblinNativeControl* control, gboolean changed) {
    publish_privacy_snapshot(control, changed);
}

void gnoblin_control_pending_location_authorization_free(gpointer user_data) {
    PendingLocationAuthorization* pending = user_data;
    if (pending->timeout_source_id)
        g_source_remove(pending->timeout_source_id);
    if (!pending->completed)
        gnoblin_location_request_complete(pending->request, FALSE, 0);
    gnoblin_location_request_unref(pending->request);
    g_clear_pointer(&pending->recipient_client_ids, g_hash_table_unref);
    g_free(pending);
}

void gnoblin_control_clear_pending_location_authorizations(GnoblinNativeControl* control) {
    if (control && control->pending_location_authorizations)
        g_hash_table_remove_all(control->pending_location_authorizations);
}

static gboolean pending_location_authorization_timeout(gpointer user_data) {
    PendingLocationAuthorization* pending = user_data;
    pending->timeout_source_id = 0;
    if (pending->control->pending_location_authorizations)
        g_hash_table_remove(pending->control->pending_location_authorizations,
                            &pending->request_id);
    return G_SOURCE_REMOVE;
}

static void location_authorize_app(GnoblinLocationAgent* agent, const char* app_id,
                                   guint requested_accuracy, GnoblinLocationRequest* request,
                                   gpointer user_data) {
    GnoblinNativeControl* control = user_data;
    (void)agent;
    if (!control || control->stopping || !control->supervised_runtime ||
        control->runtime_worker_suspended || !control->runtime_hello_sent ||
        !control->pending_location_authorizations ||
        g_hash_table_size(control->pending_location_authorizations) >=
            MAX_PENDING_LOCATION_AUTHORIZATIONS ||
        control->next_location_request_id >= G_MAXINT64) {
        gnoblin_location_request_complete(request, FALSE, 0);
        return;
    }

    PendingLocationAuthorization* pending = g_new0(PendingLocationAuthorization, 1);
    pending->control = control;
    pending->request = gnoblin_location_request_ref(request);
    pending->request_id = ++control->next_location_request_id;
    pending->requested_accuracy = requested_accuracy;
    pending->expires_at_us =
        g_get_monotonic_time() + LOCATION_AUTHORIZATION_TIMEOUT_SECONDS * G_USEC_PER_SEC;
    pending->recipient_client_ids =
        g_hash_table_new_full(g_int64_hash, g_int64_equal, g_free, NULL);
    guint64* key = g_new(guint64, 1);
    *key = pending->request_id;
    g_hash_table_insert(control->pending_location_authorizations, key, pending);
    pending->timeout_source_id = g_timeout_add_seconds(
        LOCATION_AUTHORIZATION_TIMEOUT_SECONDS, pending_location_authorization_timeout, pending);

    GVariantBuilder fields;
    g_variant_builder_init(&fields, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&fields, "{sv}", "request_id",
                          g_variant_new_int64((gint64)pending->request_id));
    g_variant_builder_add(&fields, "{sv}", "app_id", g_variant_new_string(app_id ? app_id : ""));
    g_variant_builder_add(&fields, "{sv}", "requested_accuracy",
                          g_variant_new_uint32(requested_accuracy));
    g_variant_builder_add(&fields, "{sv}", "expires_at_us",
                          g_variant_new_int64(pending->expires_at_us));
    g_autoptr(GVariant) payload = g_variant_ref_sink(g_variant_builder_end(&fields));
    gnoblin_control_publish_event(control, "gnoblin.location.authorization-requested", payload);
}

static void privacy_location_state_changed(GnoblinLocationAgent* agent, gboolean available,
                                           gboolean in_use, gpointer user_data) {
    GnoblinNativeControl* control = user_data;
    (void)agent;
    if (!control || control->stopping)
        return;
    in_use = available && in_use;
    gboolean availability_changed = control->privacy_location_available != available;
    if (!availability_changed && control->privacy_location_in_use == in_use)
        return;
    control->privacy_location_available = available;
    control->privacy_location_in_use = in_use;
    if (availability_changed) {
        control->state_revision++;
        g_autoptr(GVariant) capabilities = gnoblin_control_capability_snapshot(control);
        gnoblin_control_publish_runtime_snapshot(control, "capabilities", capabilities,
                                        control->state_revision);
        const NativeCapability* changed = gnoblin_control_capability_by_id("location-agent");
        if (changed) {
            GVariantBuilder event;
            g_variant_builder_init(&event, G_VARIANT_TYPE_VARDICT);
            g_autoptr(GVariant) capability =
                gnoblin_control_capability_snapshot_record(control, changed);
            g_variant_builder_add(&event, "{sv}", "capability", capability);
            g_variant_builder_add(&event, "{sv}", "revision",
                                  g_variant_new_int64((gint64)control->state_revision));
            g_autoptr(GVariant) fields = g_variant_ref_sink(g_variant_builder_end(&event));
            gnoblin_control_publish_event(control, "gnoblin.capability.changed", fields);
        }
    }
    control->privacy_revision++;
    publish_privacy_snapshot(control, TRUE);
}

void gnoblin_control_location_agent_create(GnoblinNativeControl* control) {
    control->location_agent = gnoblin_location_agent_new(
        NULL, location_authorize_app, privacy_location_state_changed, control, NULL);
}

static void privacy_handle_stopped(MetaRemoteAccessHandle* handle, gpointer user_data) {
    GnoblinNativeControl* control = user_data;
    if (!control || control->stopping || !g_hash_table_remove(control->privacy_handles, handle))
        return;
    privacy_refresh_state(control);
}

void gnoblin_control_privacy_new_handle(MetaRemoteAccessController* controller,
                               MetaRemoteAccessHandle* handle, gpointer user_data) {
    GnoblinNativeControl* control = user_data;
    if (!control || control->stopping || !handle ||
        g_hash_table_contains(control->privacy_handles, handle))
        return;

    g_hash_table_add(control->privacy_handles, g_object_ref(handle));
    g_signal_connect(handle, "stopped", G_CALLBACK(privacy_handle_stopped), control);
    privacy_refresh_state(control);
}

GVariant* gnoblin_control_stop_privacy_sessions(GnoblinNativeControl* control, gboolean recording,
                                              GError** error) {
    g_autoptr(GPtrArray) handles = g_ptr_array_new_with_free_func(g_object_unref);
    GHashTableIter iter;
    gpointer key;
    guint32 requested = 0;
    if (!control || control->stopping || !control->privacy_handles) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_CLOSED,
                            "privacy session control is unavailable");
        return NULL;
    }

    g_hash_table_iter_init(&iter, control->privacy_handles);
    while (g_hash_table_iter_next(&iter, &key, NULL)) {
        gboolean is_recording = FALSE;
        g_object_get(key, "is-recording", &is_recording, NULL);
        if (is_recording == recording)
            g_ptr_array_add(handles, g_object_ref(key));
    }

    for (guint i = 0; i < handles->len; i++) {
        MetaRemoteAccessHandle* handle = g_ptr_array_index(handles, i);
        meta_remote_access_handle_stop(handle);
        requested++;
    }

    GVariantBuilder result;
    g_variant_builder_init(&result, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&result, "{sv}", "requested", g_variant_new_uint32(requested));
    return g_variant_ref_sink(g_variant_builder_end(&result));
}
