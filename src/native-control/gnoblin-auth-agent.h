/* Polkit authentication agent for the native Gnoblin runtime.
 *
 * GNOME Shell normally registers as the polkit agent for the session. Gnoblin has no
 * shell, so this module registers instead and hands each request to its owner as plain
 * data. The owner decides how to ask the user and then answers through the request.
 *
 * All calls must run on the thread that owns the GMainContext used at start time.
 * A request that nobody answers stays pending until the owner cancels it or polkit
 * withdraws it. The owner is responsible for any timeout.
 */
#pragma once

#include <gio/gio.h>

G_BEGIN_DECLS

typedef struct _GnoblinAuthAgent GnoblinAuthAgent;
typedef struct _GnoblinAuthRequest GnoblinAuthRequest;

typedef enum {
    GNOBLIN_AUTH_RESULT_AUTHORIZED,
    GNOBLIN_AUTH_RESULT_DENIED,
    GNOBLIN_AUTH_RESULT_CANCELLED,
} GnoblinAuthResult;

typedef struct {
    /* A new request arrived. Borrowed; ref it to keep it. */
    void (*started)(GnoblinAuthAgent* agent, GnoblinAuthRequest* request, gpointer user_data);
    /* The helper asked for input. Call gnoblin_auth_request_respond() once. */
    void (*prompt)(GnoblinAuthAgent* agent, GnoblinAuthRequest* request, const char* prompt,
                   gboolean echo, gpointer user_data);
    /* The helper sent a text line for the user. */
    void (*message)(GnoblinAuthAgent* agent, GnoblinAuthRequest* request, const char* text,
                    gboolean is_error, gpointer user_data);
    /* The request ended. No further calls for it are accepted. */
    void (*finished)(GnoblinAuthAgent* agent, GnoblinAuthRequest* request,
                     GnoblinAuthResult result, gpointer user_data);
} GnoblinAuthCallbacks;

GnoblinAuthAgent* gnoblin_auth_agent_new(const GnoblinAuthCallbacks* callbacks,
                                         gpointer user_data);
/* Registers with polkit for this login session. Returns FALSE and sets error when polkit
 * is unreachable, the session is unknown, or another agent already owns the session. */
gboolean gnoblin_auth_agent_start(GnoblinAuthAgent* agent, GError** error);
/* Cancels every pending request and unregisters. Safe to call twice. */
void gnoblin_auth_agent_stop(GnoblinAuthAgent* agent);
void gnoblin_auth_agent_free(GnoblinAuthAgent* agent);
gboolean gnoblin_auth_agent_is_registered(GnoblinAuthAgent* agent);

GnoblinAuthRequest* gnoblin_auth_request_ref(GnoblinAuthRequest* request);
void gnoblin_auth_request_unref(GnoblinAuthRequest* request);

/* Stable for the life of the agent; never reused and never zero. */
guint64 gnoblin_auth_request_get_id(GnoblinAuthRequest* request);
const char* gnoblin_auth_request_get_action_id(GnoblinAuthRequest* request);
const char* gnoblin_auth_request_get_message(GnoblinAuthRequest* request);
const char* gnoblin_auth_request_get_icon_name(GnoblinAuthRequest* request);
/* New a{ss} variant of the action details. The cookie is never exposed. */
GVariant* gnoblin_auth_request_dup_details(GnoblinAuthRequest* request);
guint gnoblin_auth_request_get_identity_count(GnoblinAuthRequest* request);
/* kind is "user", "group" or "other". name is a display name. Both are owned by the request. */
gboolean gnoblin_auth_request_get_identity(GnoblinAuthRequest* request, guint index,
                                           const char** kind, const char** name);

/* Starts checking the chosen identity. Errors on a second call, a bad index, or a
 * finished request. Prompts then arrive through the callbacks. */
gboolean gnoblin_auth_request_begin(GnoblinAuthRequest* request, guint identity_index,
                                    GError** error);
/* Answers the latest prompt. Errors when no prompt is waiting. The caller keeps ownership of
 * response; the agent does not store it. */
gboolean gnoblin_auth_request_respond(GnoblinAuthRequest* request, const char* response,
                                      GError** error);
/* Ends the request as cancelled. Later calls are ignored. */
void gnoblin_auth_request_cancel(GnoblinAuthRequest* request);

G_DEFINE_AUTOPTR_CLEANUP_FUNC(GnoblinAuthAgent, gnoblin_auth_agent_free)
G_DEFINE_AUTOPTR_CLEANUP_FUNC(GnoblinAuthRequest, gnoblin_auth_request_unref)

G_END_DECLS
