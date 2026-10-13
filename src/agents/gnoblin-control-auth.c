/* Polkit agent glue: forwards authentication requests to Lua handlers and relays their answers. */
#ifndef _GNU_SOURCE
#define _GNU_SOURCE
#endif
#include "config.h"

#include "core/gnoblin-control-internal.h"

/* A polkit prompt waits for a shell or Lua handler. A request that nothing claims with
 * auth.begin or auth.cancel is cancelled quickly so pkexec fails instead of hanging. */
#define MAX_PENDING_AUTH_REQUESTS 8
#define AUTH_UNCLAIMED_TIMEOUT_SECONDS 15
#define AUTH_ANSWER_TIMEOUT_SECONDS 120

typedef struct {
    GnoblinNativeControl* control;
    GnoblinAuthRequest* request;
    GHashTable* recipient_client_ids;
    guint64 request_id;
    guint timeout_source_id;
    GnoblinAuthResult result;
    gboolean begun;
} PendingAuth;

static const char* auth_result_name(GnoblinAuthResult result) {
    switch (result) {
    case GNOBLIN_AUTH_RESULT_AUTHORIZED:
        return "authorized";
    case GNOBLIN_AUTH_RESULT_DENIED:
        return "denied";
    default:
        return "cancelled";
    }
}

static void pending_auth_publish_finished(PendingAuth* pending) {
    GVariantBuilder fields;
    g_variant_builder_init(&fields, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&fields, "{sv}", "request_id",
                          g_variant_new_int64((gint64)pending->request_id));
    g_variant_builder_add(&fields, "{sv}", "result",
                          g_variant_new_string(auth_result_name(pending->result)));
    g_autoptr(GVariant) payload = g_variant_ref_sink(g_variant_builder_end(&fields));
    gnoblin_control_publish_event(pending->control, "gnoblin.auth.finished", payload);
}

static void pending_auth_free(gpointer user_data) {
    PendingAuth* pending = user_data;
    if (pending->timeout_source_id)
        g_source_remove(pending->timeout_source_id);
    pending->timeout_source_id = 0;
    /* A finished request ignores this. A pending one ends as cancelled. */
    gnoblin_auth_request_cancel(pending->request);
    pending_auth_publish_finished(pending);
    gnoblin_auth_request_unref(pending->request);
    g_clear_pointer(&pending->recipient_client_ids, g_hash_table_unref);
    g_free(pending);
}

static void clear_pending_auth(GnoblinNativeControl* control) {
    if (control && control->pending_auth)
        g_hash_table_remove_all(control->pending_auth);
}

static gboolean pending_auth_timeout(gpointer user_data) {
    PendingAuth* pending = user_data;
    pending->timeout_source_id = 0;
    if (pending->control->pending_auth) {
        gint64 request_id = (gint64)pending->request_id;
        g_hash_table_remove(pending->control->pending_auth, &request_id);
    }
    return G_SOURCE_REMOVE;
}

static void pending_auth_arm(PendingAuth* pending, guint seconds) {
    if (pending->timeout_source_id)
        g_source_remove(pending->timeout_source_id);
    pending->timeout_source_id = g_timeout_add_seconds(seconds, pending_auth_timeout, pending);
}

static PendingAuth* pending_auth_lookup(GnoblinNativeControl* control,
                                        GnoblinAuthRequest* request) {
    gint64 request_id = (gint64)gnoblin_auth_request_get_id(request);
    return control->pending_auth ? g_hash_table_lookup(control->pending_auth, &request_id) : NULL;
}

static void auth_started(GnoblinAuthAgent* agent, GnoblinAuthRequest* request, gpointer user_data) {
    GnoblinNativeControl* control = user_data;
    (void)agent;
    if (!control || control->stopping || !control->supervised_runtime ||
        control->runtime_worker_suspended || !control->runtime_hello_sent ||
        !control->pending_auth ||
        g_hash_table_size(control->pending_auth) >= MAX_PENDING_AUTH_REQUESTS) {
        gnoblin_auth_request_cancel(request);
        return;
    }

    PendingAuth* pending = g_new0(PendingAuth, 1);
    pending->control = control;
    pending->request = gnoblin_auth_request_ref(request);
    pending->request_id = gnoblin_auth_request_get_id(request);
    pending->result = GNOBLIN_AUTH_RESULT_CANCELLED;
    pending->recipient_client_ids =
        g_hash_table_new_full(g_int64_hash, g_int64_equal, g_free, NULL);
    guint64* key = g_new(guint64, 1);
    *key = pending->request_id;
    g_hash_table_insert(control->pending_auth, key, pending);
    pending_auth_arm(pending, AUTH_UNCLAIMED_TIMEOUT_SECONDS);

    GVariantBuilder fields;
    g_variant_builder_init(&fields, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&fields, "{sv}", "request_id",
                          g_variant_new_int64((gint64)pending->request_id));
    g_variant_builder_add(&fields, "{sv}", "action_id",
                          g_variant_new_string(gnoblin_auth_request_get_action_id(request)));
    g_variant_builder_add(&fields, "{sv}", "message",
                          g_variant_new_string(gnoblin_auth_request_get_message(request)));
    g_variant_builder_add(&fields, "{sv}", "icon_name",
                          g_variant_new_string(gnoblin_auth_request_get_icon_name(request)));

    g_autoptr(GVariant) details = gnoblin_auth_request_dup_details(request);
    GVariantBuilder details_builder;
    GVariantIter details_iter;
    const char* detail_key;
    const char* detail_value;
    g_variant_builder_init(&details_builder, G_VARIANT_TYPE_VARDICT);
    g_variant_iter_init(&details_iter, details);
    while (g_variant_iter_next(&details_iter, "{&s&s}", &detail_key, &detail_value))
        g_variant_builder_add(&details_builder, "{sv}", detail_key,
                              g_variant_new_string(detail_value));
    g_variant_builder_add(&fields, "{sv}", "details", g_variant_builder_end(&details_builder));

    GVariantBuilder identities;
    g_variant_builder_init(&identities, G_VARIANT_TYPE("aa{sv}"));
    for (guint i = 0; i < gnoblin_auth_request_get_identity_count(request); i++) {
        const char* kind = "";
        const char* name = "";
        GVariantBuilder identity;
        gnoblin_auth_request_get_identity(request, i, &kind, &name);
        g_variant_builder_init(&identity, G_VARIANT_TYPE_VARDICT);
        g_variant_builder_add(&identity, "{sv}", "kind", g_variant_new_string(kind));
        g_variant_builder_add(&identity, "{sv}", "name", g_variant_new_string(name));
        g_variant_builder_add_value(&identities, g_variant_builder_end(&identity));
    }
    g_variant_builder_add(&fields, "{sv}", "identities", g_variant_builder_end(&identities));
    g_autoptr(GVariant) payload = g_variant_ref_sink(g_variant_builder_end(&fields));
    gnoblin_control_publish_event(control, "gnoblin.auth.requested", payload);
}

static void auth_prompt(GnoblinAuthAgent* agent, GnoblinAuthRequest* request, const char* prompt,
                        gboolean echo, gpointer user_data) {
    GnoblinNativeControl* control = user_data;
    PendingAuth* pending = control ? pending_auth_lookup(control, request) : NULL;
    (void)agent;
    if (!pending) {
        gnoblin_auth_request_cancel(request);
        return;
    }
    pending_auth_arm(pending, AUTH_ANSWER_TIMEOUT_SECONDS);
    GVariantBuilder fields;
    g_variant_builder_init(&fields, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&fields, "{sv}", "request_id",
                          g_variant_new_int64((gint64)pending->request_id));
    g_variant_builder_add(&fields, "{sv}", "prompt", g_variant_new_string(prompt ? prompt : ""));
    g_variant_builder_add(&fields, "{sv}", "echo", g_variant_new_boolean(echo));
    g_autoptr(GVariant) payload = g_variant_ref_sink(g_variant_builder_end(&fields));
    gnoblin_control_publish_event(control, "gnoblin.auth.prompt", payload);
}

static void auth_message(GnoblinAuthAgent* agent, GnoblinAuthRequest* request, const char* text,
                         gboolean is_error, gpointer user_data) {
    GnoblinNativeControl* control = user_data;
    PendingAuth* pending = control ? pending_auth_lookup(control, request) : NULL;
    (void)agent;
    if (!pending)
        return;
    GVariantBuilder fields;
    g_variant_builder_init(&fields, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&fields, "{sv}", "request_id",
                          g_variant_new_int64((gint64)pending->request_id));
    g_variant_builder_add(&fields, "{sv}", "text", g_variant_new_string(text ? text : ""));
    g_variant_builder_add(&fields, "{sv}", "error", g_variant_new_boolean(is_error));
    g_autoptr(GVariant) payload = g_variant_ref_sink(g_variant_builder_end(&fields));
    gnoblin_control_publish_event(control, "gnoblin.auth.message", payload);
}

static void auth_finished(GnoblinAuthAgent* agent, GnoblinAuthRequest* request,
                          GnoblinAuthResult result, gpointer user_data) {
    GnoblinNativeControl* control = user_data;
    PendingAuth* pending = control ? pending_auth_lookup(control, request) : NULL;
    (void)agent;
    if (!pending)
        return;
    /* Removing the entry publishes gnoblin.auth.finished with this result. */
    pending->result = result;
    gint64 request_id = (gint64)pending->request_id;
    g_hash_table_remove(control->pending_auth, &request_id);
}

GVariant* gnoblin_control_auth_operation(GnoblinNativeControl* control, const char* method,
                                         GVariant* arguments, guint64 client_id, GError** error) {
    gint64 request_id = 0;
    if (!g_variant_lookup(arguments, "request_id", "x", &request_id) || request_id <= 0) {
        g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                    "%s requires a positive request_id", method);
        return NULL;
    }
    PendingAuth* pending =
        control->pending_auth ? g_hash_table_lookup(control->pending_auth, &request_id) : NULL;
    if (!pending) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_FOUND,
                            "authentication request is no longer pending");
        return NULL;
    }
    if (client_id && !g_hash_table_contains(pending->recipient_client_ids, &client_id)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "only a client that received the request can answer it");
        return NULL;
    }

    if (g_str_equal(method, "auth.begin")) {
        gint64 identity = 1;
        if (g_variant_lookup(arguments, "identity", "x", &identity) && identity < 1) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                                "auth.begin identity counts from 1");
            return NULL;
        }
        if (!gnoblin_auth_request_begin(pending->request, (guint)(identity - 1), error))
            return NULL;
        pending->begun = TRUE;
        pending_auth_arm(pending, AUTH_ANSWER_TIMEOUT_SECONDS);
    } else if (g_str_equal(method, "auth.respond")) {
        const char* response = NULL;
        if (!g_variant_lookup(arguments, "response", "&s", &response)) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                                "auth.respond requires a response string");
            return NULL;
        }
        if (!gnoblin_auth_request_respond(pending->request, response, error))
            return NULL;
        pending_auth_arm(pending, AUTH_ANSWER_TIMEOUT_SECONDS);
    } else {
        /* auth.cancel ends the request, which frees the pending entry. */
        gnoblin_auth_request_cancel(pending->request);
    }

    GVariantBuilder result;
    g_variant_builder_init(&result, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&result, "{sv}", "request_id", g_variant_new_int64(request_id));
    g_variant_builder_add(&result, "{sv}", "submitted", g_variant_new_boolean(TRUE));
    return g_variant_ref_sink(g_variant_builder_end(&result));
}

void gnoblin_control_auth_init(GnoblinNativeControl* control) {
    control->pending_auth =
        g_hash_table_new_full(g_int64_hash, g_int64_equal, g_free, pending_auth_free);
}

static void auth_publish_capability_changed(GnoblinNativeControl* control) {
    if (control->stopping || !control->runtime_hello_sent)
        return;
    control->state_revision++;
    g_autoptr(GVariant) capabilities = gnoblin_control_capability_snapshot(control);
    gnoblin_control_publish_runtime_snapshot(control, "capabilities", capabilities,
                                             control->state_revision);
    const NativeCapability* changed = gnoblin_control_capability_by_id("auth-agent");
    if (!changed)
        return;
    GVariantBuilder event;
    g_variant_builder_init(&event, G_VARIANT_TYPE_VARDICT);
    g_autoptr(GVariant) capability = gnoblin_control_capability_snapshot_record(control, changed);
    g_variant_builder_add(&event, "{sv}", "capability", capability);
    g_variant_builder_add(&event, "{sv}", "revision",
                          g_variant_new_int64((gint64)control->state_revision));
    g_autoptr(GVariant) fields = g_variant_ref_sink(g_variant_builder_end(&event));
    gnoblin_control_publish_event(control, "gnoblin.capability.changed", fields);
}

const char* gnoblin_control_auth_unavailable_reason(GnoblinNativeControl* control) {
    if (!control->auth_agent)
        return "auth_disabled";
    return gnoblin_auth_agent_is_registered(control->auth_agent) ? NULL : "polkit_unavailable";
}

gboolean gnoblin_control_auth_available(GnoblinNativeControl* control) {
    return gnoblin_control_auth_unavailable_reason(control) == NULL;
}

static void auth_start(GnoblinNativeControl* control) {
    GnoblinAuthCallbacks auth_callbacks = {auth_started, auth_prompt, auth_message, auth_finished};
    g_autoptr(GError) auth_error = NULL;
    control->auth_agent = gnoblin_auth_agent_new(&auth_callbacks, control);
    if (!gnoblin_auth_agent_start(control->auth_agent, &auth_error))
        g_message("gnoblin-auth: polkit agent unavailable: %s",
                  auth_error ? auth_error->message : "unknown error");
}

/* The agent is opt-in because polkit allows one agent per login session. Taking the role at
 * startup would stop an autostarted agent such as polkit-gnome from registering. */
void gnoblin_control_auth_apply_config(GnoblinNativeControl* control, GVariant* config) {
    if (!control || control->stopping)
        return;
    g_autoptr(GVariant) auth =
        config ? g_variant_lookup_value(config, "auth", G_VARIANT_TYPE_VARDICT) : NULL;
    g_autoptr(GVariant) enabled_value =
        auth ? g_variant_lookup_value(auth, "polkit-agent", G_VARIANT_TYPE_BOOLEAN) : NULL;
    if (!enabled_value && auth)
        enabled_value = g_variant_lookup_value(auth, "polkit_agent", G_VARIANT_TYPE_BOOLEAN);
    gboolean enabled = enabled_value && g_variant_get_boolean(enabled_value);
    if (enabled && !control->auth_agent) {
        auth_start(control);
        auth_publish_capability_changed(control);
    } else if (!enabled && control->auth_agent) {
        gnoblin_control_auth_stop(control);
        auth_publish_capability_changed(control);
    }
}

void gnoblin_control_auth_stop(GnoblinNativeControl* control) {
    g_clear_pointer(&control->auth_agent, gnoblin_auth_agent_free);
    clear_pending_auth(control);
}

void gnoblin_control_auth_note_recipient(GnoblinNativeControl* control, gint64 request_id,
                                         guint64 client_id) {
    PendingAuth* auth_pending =
        request_id > 0 ? g_hash_table_lookup(control->pending_auth, &request_id) : NULL;
    if (auth_pending) {
        guint64* client_key = g_new(guint64, 1);
        *client_key = client_id;
        g_hash_table_add(auth_pending->recipient_client_ids, client_key);
    }
}
