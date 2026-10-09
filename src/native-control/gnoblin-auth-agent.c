/* Polkit authentication agent. See gnoblin-auth-agent.h for the contract. */
#include "gnoblin-auth-agent.h"

/* libpolkitagent marks its API as unstable. Every desktop agent defines this, and the
 * listener class has been stable for many years. */
#define POLKIT_AGENT_I_KNOW_API_IS_SUBJECT_TO_CHANGE

#include <grp.h>
#include <polkit/polkit.h>
#include <polkitagent/polkitagent.h>
#include <pwd.h>
#include <string.h>
#include <unistd.h>

typedef struct {
    char* kind;
    char* name;
    PolkitIdentity* identity;
} AuthIdentity;

struct _GnoblinAuthAgent {
    GnoblinAuthCallbacks callbacks;
    gpointer user_data;
    GObject* listener;
    gpointer registration;
    GHashTable* requests;
    guint64 next_id;
    gboolean stopping;
};

struct _GnoblinAuthRequest {
    gatomicrefcount refs;
    GnoblinAuthAgent* agent;
    guint64 id;
    char* action_id;
    char* message;
    char* icon_name;
    char* cookie;
    GVariant* details;
    GPtrArray* identities;
    GTask* task;
    GCancellable* cancellable;
    gulong cancel_handler;
    PolkitAgentSession* session;
    gulong session_handlers[4];
    gboolean begun;
    gboolean prompt_waiting;
    gboolean finished;
};

static void request_finish(GnoblinAuthRequest* request, GnoblinAuthResult result);

/* Listener subclass */

typedef struct {
    PolkitAgentListener parent_instance;
    GnoblinAuthAgent* agent;
} GnoblinPolkitListener;

typedef struct {
    PolkitAgentListenerClass parent_class;
} GnoblinPolkitListenerClass;

GType gnoblin_polkit_listener_get_type(void);
G_DEFINE_TYPE(GnoblinPolkitListener, gnoblin_polkit_listener, POLKIT_AGENT_TYPE_LISTENER)

static void auth_identity_free(gpointer data) {
    AuthIdentity* identity = data;
    g_free(identity->kind);
    g_free(identity->name);
    g_clear_object(&identity->identity);
    g_free(identity);
}

static AuthIdentity* auth_identity_new(PolkitIdentity* polkit_identity) {
    AuthIdentity* identity = g_new0(AuthIdentity, 1);
    identity->identity = g_object_ref(polkit_identity);

    if (POLKIT_IS_UNIX_USER(polkit_identity)) {
        struct passwd entry;
        struct passwd* found = NULL;
        char buffer[1024];
        identity->kind = g_strdup("user");
        if (getpwuid_r(polkit_unix_user_get_uid(POLKIT_UNIX_USER(polkit_identity)), &entry, buffer,
                       sizeof buffer, &found) == 0 &&
            found)
            identity->name = g_strdup(found->pw_name);
    } else if (POLKIT_IS_UNIX_GROUP(polkit_identity)) {
        struct group entry;
        struct group* found = NULL;
        char buffer[1024];
        identity->kind = g_strdup("group");
        if (getgrgid_r(polkit_unix_group_get_gid(POLKIT_UNIX_GROUP(polkit_identity)), &entry,
                       buffer, sizeof buffer, &found) == 0 &&
            found)
            identity->name = g_strdup(found->gr_name);
    } else {
        identity->kind = g_strdup("other");
    }
    if (!identity->name)
        identity->name = polkit_identity_to_string(polkit_identity);
    return identity;
}

static GVariant* details_to_variant(PolkitDetails* details) {
    GVariantBuilder builder;
    g_variant_builder_init(&builder, G_VARIANT_TYPE("a{ss}"));
    if (details) {
        g_auto(GStrv) keys = polkit_details_get_keys(details);
        for (gsize i = 0; keys && keys[i]; i++) {
            const char* value = polkit_details_lookup(details, keys[i]);
            g_variant_builder_add(&builder, "{ss}", keys[i], value ? value : "");
        }
    }
    return g_variant_ref_sink(g_variant_builder_end(&builder));
}

GnoblinAuthRequest* gnoblin_auth_request_ref(GnoblinAuthRequest* request) {
    g_atomic_ref_count_inc(&request->refs);
    return request;
}

void gnoblin_auth_request_unref(GnoblinAuthRequest* request) {
    if (!g_atomic_ref_count_dec(&request->refs))
        return;
    g_free(request->action_id);
    g_free(request->message);
    g_free(request->icon_name);
    /* The cookie authorises one polkit action for this session. Wipe it. */
    if (request->cookie) {
        memset(request->cookie, 0, strlen(request->cookie));
        g_free(request->cookie);
    }
    g_clear_pointer(&request->details, g_variant_unref);
    g_clear_pointer(&request->identities, g_ptr_array_unref);
    g_clear_object(&request->task);
    g_clear_object(&request->cancellable);
    g_clear_object(&request->session);
    g_free(request);
}

static void request_unref_notify(gpointer data) {
    gnoblin_auth_request_unref(data);
}

guint64 gnoblin_auth_request_get_id(GnoblinAuthRequest* request) {
    return request->id;
}

const char* gnoblin_auth_request_get_action_id(GnoblinAuthRequest* request) {
    return request->action_id;
}

const char* gnoblin_auth_request_get_message(GnoblinAuthRequest* request) {
    return request->message;
}

const char* gnoblin_auth_request_get_icon_name(GnoblinAuthRequest* request) {
    return request->icon_name;
}

GVariant* gnoblin_auth_request_dup_details(GnoblinAuthRequest* request) {
    return g_variant_ref(request->details);
}

guint gnoblin_auth_request_get_identity_count(GnoblinAuthRequest* request) {
    return request->identities->len;
}

gboolean gnoblin_auth_request_get_identity(GnoblinAuthRequest* request, guint index,
                                           const char** kind, const char** name) {
    if (index >= request->identities->len)
        return FALSE;
    AuthIdentity* identity = g_ptr_array_index(request->identities, index);
    if (kind)
        *kind = identity->kind;
    if (name)
        *name = identity->name;
    return TRUE;
}

/* Session signal handlers. The request outlives the handlers because request_finish()
 * disconnects them before it drops the agent's reference. */

static void on_session_request(PolkitAgentSession* session, const char* prompt, gboolean echo_on,
                               gpointer user_data) {
    GnoblinAuthRequest* request = user_data;
    GnoblinAuthAgent* agent = request->agent;
    if (request->finished || !agent)
        return;
    request->prompt_waiting = TRUE;
    if (agent->callbacks.prompt)
        agent->callbacks.prompt(agent, request, prompt, echo_on, agent->user_data);
}

static void session_message(GnoblinAuthRequest* request, const char* text, gboolean is_error) {
    GnoblinAuthAgent* agent = request->agent;
    if (request->finished || !agent || !agent->callbacks.message)
        return;
    agent->callbacks.message(agent, request, text, is_error, agent->user_data);
}

static void on_session_show_error(PolkitAgentSession* session, const char* text,
                                  gpointer user_data) {
    session_message(user_data, text, TRUE);
}

static void on_session_show_info(PolkitAgentSession* session, const char* text,
                                 gpointer user_data) {
    session_message(user_data, text, FALSE);
}

static void on_session_completed(PolkitAgentSession* session, gboolean gained,
                                 gpointer user_data) {
    request_finish(user_data, gained ? GNOBLIN_AUTH_RESULT_AUTHORIZED : GNOBLIN_AUTH_RESULT_DENIED);
}

static void request_finish(GnoblinAuthRequest* request, GnoblinAuthResult result) {
    if (request->finished)
        return;
    request->finished = TRUE;
    request->prompt_waiting = FALSE;

    /* Callbacks may drop the last outside reference. */
    gnoblin_auth_request_ref(request);

    if (request->session) {
        for (guint i = 0; i < G_N_ELEMENTS(request->session_handlers); i++) {
            if (request->session_handlers[i])
                g_signal_handler_disconnect(request->session, request->session_handlers[i]);
            request->session_handlers[i] = 0;
        }
        if (result == GNOBLIN_AUTH_RESULT_CANCELLED)
            polkit_agent_session_cancel(request->session);
        g_clear_object(&request->session);
    }
    if (request->cancellable && request->cancel_handler) {
        g_cancellable_disconnect(request->cancellable, request->cancel_handler);
        request->cancel_handler = 0;
    }

    GnoblinAuthAgent* agent = request->agent;
    request->agent = NULL;
    if (agent) {
        guint64 id = request->id;
        g_hash_table_remove(agent->requests, &id);
        if (agent->callbacks.finished)
            agent->callbacks.finished(agent, request, result, agent->user_data);
    }

    if (request->task) {
        if (result == GNOBLIN_AUTH_RESULT_CANCELLED)
            g_task_return_new_error(request->task, G_IO_ERROR, G_IO_ERROR_CANCELLED,
                                    "Authentication cancelled");
        else
            g_task_return_boolean(request->task, TRUE);
        g_clear_object(&request->task);
    }

    gnoblin_auth_request_unref(request);
}

gboolean gnoblin_auth_request_begin(GnoblinAuthRequest* request, guint identity_index,
                                    GError** error) {
    if (request->finished) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_CLOSED, "Request already finished");
        return FALSE;
    }
    if (request->begun) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PENDING, "Request already started");
        return FALSE;
    }
    if (identity_index >= request->identities->len) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "Identity index is out of range");
        return FALSE;
    }

    AuthIdentity* identity = g_ptr_array_index(request->identities, identity_index);
    request->begun = TRUE;
    request->session = polkit_agent_session_new(identity->identity, request->cookie);
    request->session_handlers[0] =
        g_signal_connect(request->session, "request", G_CALLBACK(on_session_request), request);
    request->session_handlers[1] = g_signal_connect(request->session, "show-error",
                                                    G_CALLBACK(on_session_show_error), request);
    request->session_handlers[2] = g_signal_connect(request->session, "show-info",
                                                    G_CALLBACK(on_session_show_info), request);
    request->session_handlers[3] = g_signal_connect(request->session, "completed",
                                                    G_CALLBACK(on_session_completed), request);
    polkit_agent_session_initiate(request->session);
    return TRUE;
}

gboolean gnoblin_auth_request_respond(GnoblinAuthRequest* request, const char* response,
                                      GError** error) {
    if (request->finished) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_CLOSED, "Request already finished");
        return FALSE;
    }
    if (!request->session || !request->prompt_waiting) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PENDING, "No prompt is waiting");
        return FALSE;
    }
    request->prompt_waiting = FALSE;
    polkit_agent_session_response(request->session, response ? response : "");
    return TRUE;
}

void gnoblin_auth_request_cancel(GnoblinAuthRequest* request) {
    request_finish(request, GNOBLIN_AUTH_RESULT_CANCELLED);
}

/* Polkit may cancel from any thread. Hop to the main context before touching the request. */
static gboolean cancel_idle(gpointer data) {
    gnoblin_auth_request_cancel(data);
    return G_SOURCE_REMOVE;
}

static void on_cancellable_cancelled(GCancellable* cancellable, gpointer data) {
    g_idle_add_full(G_PRIORITY_DEFAULT, cancel_idle, gnoblin_auth_request_ref(data),
                    request_unref_notify);
}

static void listener_initiate_authentication(PolkitAgentListener* listener, const char* action_id,
                                             const char* message, const char* icon_name,
                                             PolkitDetails* details, const char* cookie,
                                             GList* identities, GCancellable* cancellable,
                                             GAsyncReadyCallback callback, gpointer user_data) {
    GnoblinPolkitListener* self = (GnoblinPolkitListener*)listener;
    GnoblinAuthAgent* agent = self->agent;
    g_autoptr(GTask) task = g_task_new(listener, cancellable, callback, user_data);

    if (!agent || agent->stopping || !identities) {
        g_task_return_new_error(task, G_IO_ERROR, G_IO_ERROR_FAILED,
                                "Gnoblin cannot take this authentication request");
        return;
    }

    GnoblinAuthRequest* request = g_new0(GnoblinAuthRequest, 1);
    g_atomic_ref_count_init(&request->refs);
    request->agent = agent;
    request->id = ++agent->next_id;
    request->action_id = g_strdup(action_id ? action_id : "");
    request->message = g_strdup(message ? message : "");
    request->icon_name = g_strdup(icon_name ? icon_name : "");
    request->cookie = g_strdup(cookie ? cookie : "");
    request->details = details_to_variant(details);
    request->identities = g_ptr_array_new_with_free_func(auth_identity_free);
    for (GList* item = identities; item; item = item->next)
        g_ptr_array_add(request->identities, auth_identity_new(item->data));
    request->task = g_steal_pointer(&task);

    guint64* key = g_new(guint64, 1);
    *key = request->id;
    g_hash_table_insert(agent->requests, key, gnoblin_auth_request_ref(request));

    if (cancellable) {
        request->cancellable = g_object_ref(cancellable);
        request->cancel_handler =
            g_cancellable_connect(cancellable, G_CALLBACK(on_cancellable_cancelled),
                                  gnoblin_auth_request_ref(request), request_unref_notify);
    }

    if (agent->callbacks.started)
        agent->callbacks.started(agent, request, agent->user_data);
    gnoblin_auth_request_unref(request);
}

static gboolean listener_initiate_authentication_finish(PolkitAgentListener* listener,
                                                        GAsyncResult* result, GError** error) {
    return g_task_propagate_boolean(G_TASK(result), error);
}

static void gnoblin_polkit_listener_init(GnoblinPolkitListener* self) {}

static void gnoblin_polkit_listener_class_init(GnoblinPolkitListenerClass* klass) {
    PolkitAgentListenerClass* listener_class = POLKIT_AGENT_LISTENER_CLASS(klass);
    listener_class->initiate_authentication = listener_initiate_authentication;
    listener_class->initiate_authentication_finish = listener_initiate_authentication_finish;
}

/* Agent */

GnoblinAuthAgent* gnoblin_auth_agent_new(const GnoblinAuthCallbacks* callbacks,
                                         gpointer user_data) {
    GnoblinAuthAgent* agent = g_new0(GnoblinAuthAgent, 1);
    if (callbacks)
        agent->callbacks = *callbacks;
    agent->user_data = user_data;
    agent->requests = g_hash_table_new_full(g_int64_hash, g_int64_equal, g_free,
                                            request_unref_notify);
    return agent;
}

gboolean gnoblin_auth_agent_is_registered(GnoblinAuthAgent* agent) {
    return agent->registration != NULL;
}

gboolean gnoblin_auth_agent_start(GnoblinAuthAgent* agent, GError** error) {
    if (agent->registration)
        return TRUE;

    g_autoptr(GError) process_error = NULL;
    g_autoptr(PolkitSubject) subject =
        polkit_unix_session_new_for_process_sync(getpid(), NULL, &process_error);
    if (!subject) {
        const char* session_id = g_getenv("XDG_SESSION_ID");
        if (session_id && *session_id)
            subject = polkit_unix_session_new(session_id);
    }
    if (!subject) {
        g_propagate_error(error, g_steal_pointer(&process_error));
        return FALSE;
    }

    GnoblinPolkitListener* listener = g_object_new(gnoblin_polkit_listener_get_type(), NULL);
    listener->agent = agent;
    agent->stopping = FALSE;
    agent->registration = polkit_agent_listener_register(
        POLKIT_AGENT_LISTENER(listener), POLKIT_AGENT_REGISTER_FLAGS_NONE, subject, NULL, NULL,
        error);
    if (!agent->registration) {
        listener->agent = NULL;
        g_object_unref(listener);
        return FALSE;
    }
    agent->listener = G_OBJECT(listener);
    return TRUE;
}

void gnoblin_auth_agent_stop(GnoblinAuthAgent* agent) {
    agent->stopping = TRUE;

    /* Finishing a request edits the table, so cancel from a snapshot. */
    g_autoptr(GPtrArray) pending = g_ptr_array_new_with_free_func(request_unref_notify);
    GHashTableIter iter;
    gpointer value;
    g_hash_table_iter_init(&iter, agent->requests);
    while (g_hash_table_iter_next(&iter, NULL, &value))
        g_ptr_array_add(pending, gnoblin_auth_request_ref(value));
    for (guint i = 0; i < pending->len; i++)
        gnoblin_auth_request_cancel(g_ptr_array_index(pending, i));

    if (agent->registration) {
        polkit_agent_listener_unregister(agent->registration);
        agent->registration = NULL;
    }
    if (agent->listener) {
        ((GnoblinPolkitListener*)agent->listener)->agent = NULL;
        g_clear_object(&agent->listener);
    }
}

void gnoblin_auth_agent_free(GnoblinAuthAgent* agent) {
    if (!agent)
        return;
    gnoblin_auth_agent_stop(agent);
    g_clear_pointer(&agent->requests, g_hash_table_unref);
    g_free(agent);
}
