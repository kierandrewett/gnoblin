#define _GNU_SOURCE 1

#include "config.h"

#include <gio/gio.h>
#include <gio/gunixsocketaddress.h>
#include <json-glib/json-glib.h>
#include <string.h>

#include <glib/gi18n.h>

#include "globalshortcuts.h"
#include "shell-dbus.h"
#include "request.h"
#include "utils.h"
#include "session.h"

static OrgGnoblinShellGlobalShortcutsProvider* settings;
static XdpImplGlobalShortcuts* global_shortcuts;
static OrgGnoblinShellGlobalShortcutsRebind* global_shortcuts_rebind;

typedef struct _Accelerator {
    char* accelerator;
    char* native_id;
    gboolean bound;
} Accelerator;

typedef struct _GlobalShortcutsSession GlobalShortcutsSession;
static GlobalShortcutsSession* global_shortcuts_find_binding(const char*, const char**);

typedef struct _NativeControlRequest NativeControlRequest;
typedef void (*NativeControlCallback)(JsonObject* result, const char* error, gpointer user_data);

struct _NativeControlRequest {
    char* id;
    char* line;
    NativeControlCallback callback;
    gpointer user_data;
};

typedef struct _NativeControl {
    GSocketClient* client;
    GSocketConnection* connection;
    GDataInputStream* input;
    GOutputStream* output;
    GQueue* queued;
    GHashTable* pending;
    guint64 next_id;
    guint64 next_binding_id;
    gboolean connecting;
    gboolean reading;
    gboolean writing;
    gboolean awaiting_hello;
    gboolean subscribing;
    gboolean subscription_fallback;
    gboolean ready;
    gboolean failing;
    guint retry_source;
} NativeControl;

static NativeControl native_control;
static GHashTable* global_shortcuts_sessions;

typedef struct _Shortcut {
    gchar* shortcut_id; /* Client identifier */
    gchar* desc;
    GArray* accelerators;
} Shortcut;

typedef struct {
    GDBusMethodInvocation* invocation;
    Session* session;
    Request* request;
    GVariant* shortcuts;
    GPtrArray* mapped_accelerators;
    gboolean canceled;
    gboolean rebind;
    gboolean failed;
    guint next_accelerator;
} BindShortcutsHandle;

struct _GlobalShortcutsSession {
    Session parent;
    char* app_id;
    char* bind_handle;
    GArray* shortcuts;
    GVariant* initial_shortcuts;

    gboolean bound;
    gboolean closing;
};

typedef struct _GlobalShortcutsSessionClass {
    SessionClass parent_class;
} GlobalShortcutsSessionClass;

GType global_shortcuts_session_get_type();
G_DEFINE_TYPE(GlobalShortcutsSession, global_shortcuts_session, session_get_type())

static gboolean is_global_shortcuts_session(Session* session) {
    return G_TYPE_CHECK_INSTANCE_TYPE(session, global_shortcuts_session_get_type());
}

static void clear_shortcut(Shortcut* shortcut) {
    g_array_unref(shortcut->accelerators);
    g_free(shortcut->shortcut_id);
    g_free(shortcut->desc);
}

static void clear_accelerator(Accelerator* accel) {
    g_free(accel->accelerator);
    g_free(accel->native_id);
}

static void native_control_connect(void);
static void native_control_read_next(void);
static void native_control_write_next(void);
static void native_control_startup_write_done(GObject*, GAsyncResult*, gpointer);
static void native_control_rebind_shortcuts(void);
static gboolean native_control_retry(gpointer user_data);
static void native_unbind_done(JsonObject*, const char*, gpointer);

static void native_control_mark_shortcuts_unbound(void) {
    GHashTableIter iter;
    gpointer value;

    if (!global_shortcuts_sessions)
        return;

    g_hash_table_iter_init(&iter, global_shortcuts_sessions);
    while (g_hash_table_iter_next(&iter, NULL, &value)) {
        GlobalShortcutsSession* session = value;

        for (guint i = 0; i < session->shortcuts->len; i++) {
            Shortcut* shortcut = &g_array_index(session->shortcuts, Shortcut, i);

            for (guint j = 0; j < shortcut->accelerators->len; j++) {
                Accelerator* accel = &g_array_index(shortcut->accelerators, Accelerator, j);
                accel->bound = FALSE;
            }
        }
    }
}

static void native_control_request_free(NativeControlRequest* request) {
    g_free(request->id);
    g_free(request->line);
    g_free(request);
}

static void native_control_fail(const char* message) {
    NativeControlRequest* request;

    if (native_control.failing)
        return;
    native_control.failing = TRUE;
    native_control_mark_shortcuts_unbound();
    native_control.ready = FALSE;
    native_control.connecting = FALSE;
    native_control.reading = FALSE;
    native_control.writing = FALSE;
    native_control.awaiting_hello = FALSE;
    native_control.subscribing = FALSE;
    g_clear_object(&native_control.input);
    g_clear_object(&native_control.connection);
    g_clear_object(&native_control.client);
    native_control.output = NULL;

    while ((request = g_queue_pop_head(native_control.queued))) {
        if (request->callback)
            request->callback(NULL, message, request->user_data);
        native_control_request_free(request);
    }

    if (native_control.pending) {
        GHashTableIter iter;
        gpointer value;

        g_hash_table_iter_init(&iter, native_control.pending);
        while (g_hash_table_iter_next(&iter, NULL, &value)) {
            request = value;
            if (request->callback)
                request->callback(NULL, message, request->user_data);
            g_hash_table_iter_remove(&iter);
            native_control_request_free(request);
        }
    }
    if (!native_control.retry_source)
        native_control.retry_source = g_timeout_add_seconds(2, native_control_retry, NULL);
    native_control.failing = FALSE;
}

static char* native_control_socket_path(void) {
    const char* path = g_getenv("GNOBLIN_COMPOSITOR_SOCKET");
    const char* runtime_dir;

    if (path && *path)
        return g_strdup(path);

    runtime_dir = g_get_user_runtime_dir();
    if (!runtime_dir || !*runtime_dir)
        return NULL;

    return g_build_filename(runtime_dir, "gnoblin", "compositor-v1.sock", NULL);
}

static void native_control_send_line(const char* line) {
    gsize length = strlen(line);

    native_control.writing = TRUE;
    g_output_stream_write_all_async(native_control.output, line, length, G_PRIORITY_DEFAULT, NULL,
                                    native_control_startup_write_done, NULL);
}

static void native_control_startup_write_done(GObject* source, GAsyncResult* result,
                                              gpointer user_data) {
    g_autoptr(GError) error = NULL;
    gsize written = 0;

    if (G_OUTPUT_STREAM(source) != native_control.output)
        return;

    if (!g_output_stream_write_all_finish(G_OUTPUT_STREAM(source), result, &written, &error)) {
        native_control_fail(error->message);
        return;
    }

    native_control.writing = FALSE;
    native_control_write_next();
}

static void native_control_write_done(GObject* source, GAsyncResult* result, gpointer user_data) {
    g_autoptr(GError) error = NULL;
    gsize written = 0;

    if (G_OUTPUT_STREAM(source) != native_control.output)
        return;

    if (!g_output_stream_write_all_finish(G_OUTPUT_STREAM(source), result, &written, &error)) {
        native_control_fail(error->message);
        return;
    }

    native_control.writing = FALSE;
    native_control_write_next();
}

static void native_control_write_next(void) {
    NativeControlRequest* request;

    if (!native_control.ready || native_control.writing || g_queue_is_empty(native_control.queued))
        return;

    request = g_queue_pop_head(native_control.queued);
    g_hash_table_insert(native_control.pending, g_strdup(request->id), request);
    native_control.writing = TRUE;
    g_output_stream_write_all_async(native_control.output, request->line, strlen(request->line),
                                    G_PRIORITY_DEFAULT, NULL, native_control_write_done, NULL);
}

static void native_control_read_done(GObject* source, GAsyncResult* result, gpointer user_data) {
    g_autoptr(GError) error = NULL;
    g_autofree char* line = NULL;
    gsize length = 0;
    g_autoptr(JsonParser) parser = NULL;
    JsonObject* object;
    const char* event;

    if (G_DATA_INPUT_STREAM(source) != native_control.input)
        return;

    line =
        g_data_input_stream_read_line_finish(G_DATA_INPUT_STREAM(source), result, &length, &error);
    native_control.reading = FALSE;
    if (!line) {
        native_control_fail(error ? error->message : "compositor socket closed");
        return;
    }

    parser = json_parser_new();
    if (!json_parser_load_from_data(parser, line, length, &error) ||
        !JSON_NODE_HOLDS_OBJECT(json_parser_get_root(parser))) {
        native_control_fail(error ? error->message : "invalid compositor response");
        return;
    }

    object = json_node_get_object(json_parser_get_root(parser));
    event = json_object_get_string_member_with_default(object, "event", "");
    if (native_control.awaiting_hello) {
        gboolean supports_deactivation = FALSE;
        JsonArray* advertised_events = json_object_get_array_member(object, "events");

        if (!g_str_equal(event, "hello") ||
            json_object_get_int_member_with_default(object, "api_major", 0) != 1) {
            native_control_fail("unexpected compositor socket greeting");
            return;
        }

        for (guint i = 0; advertised_events && i < json_array_get_length(advertised_events); i++)
            if (g_strcmp0(json_array_get_string_element(advertised_events, i),
                          "gnoblin.shortcut.binding-deactivated") == 0) {
                supports_deactivation = TRUE;
                break;
            }

        native_control.awaiting_hello = FALSE;
        native_control.subscribing = TRUE;
        native_control.subscription_fallback = !supports_deactivation;
        if (supports_deactivation)
            native_control_send_line(
                "{\"op\":\"events\",\"api_version\":{\"major\":1,\"minor\":36},"
                "\"events\":[\"gnoblin.shortcut.binding-activated\","
                "\"gnoblin.shortcut.binding-deactivated\"]}\n");
        else
            native_control_send_line(
                "{\"op\":\"events\",\"api_version\":{\"major\":1,\"minor\":11},"
                "\"events\":[\"gnoblin.shortcut.binding-activated\"]}\n");
    } else if (native_control.subscribing) {
        if (g_str_equal(event, "error")) {
            const char* message = json_object_get_string_member_with_default(
                object, "message", "compositor event subscription failed");
            if (!native_control.subscription_fallback &&
                strstr(message, "deactivation events require API version 1.36")) {
                native_control.subscription_fallback = TRUE;
                native_control_send_line(
                    "{\"op\":\"events\",\"api_version\":{\"major\":1,\"minor\":11},"
                    "\"events\":[\"gnoblin.shortcut.binding-activated\"]}\n");
                native_control_read_next();
                return;
            }
            native_control_fail(message);
            return;
        }
        if (!g_str_equal(event, "subscribed")) {
            native_control_fail("unexpected compositor event subscription response");
            return;
        }

        native_control.subscribing = FALSE;
        native_control.ready = TRUE;
        native_control_rebind_shortcuts();
        native_control_write_next();
    } else if (g_str_equal(event, "gnoblin.shortcut.binding-activated")) {
        const char* id = json_object_get_string_member_with_default(object, "id", "");
        uint32_t timestamp =
            (uint32_t)json_object_get_int_member_with_default(object, "input_time", 0);
        const char* shortcut_id = NULL;
        GlobalShortcutsSession* session = global_shortcuts_find_binding(id, &shortcut_id);

        if (session && shortcut_id)
            xdp_impl_global_shortcuts_emit_activated(
                global_shortcuts, session_get_id((Session*)session), shortcut_id, timestamp,
                g_variant_new_array(G_VARIANT_TYPE("{sv}"), NULL, 0));
    } else if (g_str_equal(event, "gnoblin.shortcut.binding-deactivated")) {
        const char* id = json_object_get_string_member_with_default(object, "id", "");
        uint32_t timestamp =
            (uint32_t)json_object_get_int_member_with_default(object, "input_time", 0);
        const char* shortcut_id = NULL;
        GlobalShortcutsSession* session = global_shortcuts_find_binding(id, &shortcut_id);

        if (session && shortcut_id)
            xdp_impl_global_shortcuts_emit_deactivated(
                global_shortcuts, session_get_id((Session*)session), shortcut_id, timestamp,
                g_variant_new_array(G_VARIANT_TYPE("{sv}"), NULL, 0));
    } else if (g_str_equal(event, "reply") || g_str_equal(event, "error")) {
        const char* id = json_object_get_string_member_with_default(object, "id", "");
        NativeControlRequest* request = g_hash_table_lookup(native_control.pending, id);

        if (request) {
            g_hash_table_remove(native_control.pending, id);
            if (request->callback)
                request->callback(object,
                                  g_str_equal(event, "error")
                                      ? json_object_get_string_member_with_default(
                                            object, "message", "compositor request failed")
                                      : NULL,
                                  request->user_data);
            native_control_request_free(request);
        }
    }

    native_control_read_next();
}

static void native_control_read_next(void) {
    if (native_control.reading || !native_control.input)
        return;

    native_control.reading = TRUE;
    g_data_input_stream_read_line_async(native_control.input, G_PRIORITY_DEFAULT, NULL,
                                        native_control_read_done, NULL);
}

static void native_control_connected(GObject* source, GAsyncResult* result, gpointer user_data) {
    g_autoptr(GError) error = NULL;
    native_control.connection =
        g_socket_client_connect_finish(G_SOCKET_CLIENT(source), result, &error);
    native_control.connecting = FALSE;
    g_clear_object(&native_control.client);
    if (!native_control.connection) {
        native_control_fail(error->message);
        return;
    }
    if (native_control.retry_source) {
        g_source_remove(native_control.retry_source);
        native_control.retry_source = 0;
    }

    native_control.input = g_data_input_stream_new(
        g_io_stream_get_input_stream(G_IO_STREAM(native_control.connection)));
    native_control.output = g_io_stream_get_output_stream(G_IO_STREAM(native_control.connection));
    native_control.awaiting_hello = TRUE;
    native_control_read_next();
}

static void native_control_connect(void) {
    g_autofree char* path = NULL;
    g_autoptr(GSocketAddress) address = NULL;

    if (native_control.connection || native_control.connecting)
        return;

    path = native_control_socket_path();
    if (!path) {
        native_control_fail("XDG_RUNTIME_DIR is unavailable");
        return;
    }

    native_control.client = g_socket_client_new();
    address = g_unix_socket_address_new(path);
    native_control.connecting = TRUE;
    g_socket_client_connect_async(native_control.client, G_SOCKET_CONNECTABLE(address), NULL,
                                  native_control_connected, NULL);
}

static gboolean native_control_retry(gpointer user_data) {
    (void)user_data;
    native_control.retry_source = 0;
    native_control_connect();
    return G_SOURCE_REMOVE;
}

static void native_control_request(const char* method, const char* binding_id,
                                   const char* accelerator, const char* trigger,
                                   NativeControlCallback callback, gpointer user_data) {
    NativeControlRequest* request = g_new0(NativeControlRequest, 1);
    g_autoptr(JsonNode) root = json_node_new(JSON_NODE_OBJECT);
    JsonObject* object = json_object_new();
    g_autofree char* encoded = NULL;
    JsonObject* args = json_object_new();

    request->id = g_strdup_printf("portal-%" G_GUINT64_FORMAT, ++native_control.next_id);
    request->callback = callback;
    request->user_data = user_data;
    json_object_set_string_member(object, "op", "api");
    json_object_set_string_member(object, "id", request->id);
    JsonObject* version = json_object_new();
    json_object_set_int_member(version, "major", 1);
    json_object_set_int_member(version, "minor", 22);
    json_object_set_object_member(object, "api_version", version);
    json_object_set_string_member(object, "method", method);
    json_object_set_string_member(args, "id", binding_id);
    if (accelerator)
        json_object_set_string_member(args, "accelerator", accelerator);
    if (trigger)
        json_object_set_string_member(args, "trigger", trigger);
    json_object_set_object_member(object, "arguments", args);
    json_node_take_object(root, object);
    encoded = json_to_string(root, FALSE);
    request->line = g_strconcat(encoded, "\n", NULL);
    if (native_control.failing) {
        if (callback)
            callback(NULL, "compositor socket is unavailable", user_data);
        native_control_request_free(request);
        return;
    }
    g_queue_push_tail(native_control.queued, request);
    native_control_connect();
    native_control_write_next();
}

typedef struct {
    GlobalShortcutsSession* session;
    char* native_id;
} NativeRebindRequest;

static void native_rebind_request_free(NativeRebindRequest* request) {
    g_clear_object(&request->session);
    g_free(request->native_id);
    g_free(request);
}

static Accelerator* global_shortcuts_find_accelerator(GlobalShortcutsSession* session,
                                                      const char* native_id) {
    for (guint i = 0; i < session->shortcuts->len; i++) {
        Shortcut* shortcut = &g_array_index(session->shortcuts, Shortcut, i);

        for (guint j = 0; j < shortcut->accelerators->len; j++) {
            Accelerator* accel = &g_array_index(shortcut->accelerators, Accelerator, j);
            if (g_strcmp0(accel->native_id, native_id) == 0)
                return accel;
        }
    }

    return NULL;
}

static void native_rebind_done(JsonObject* result, const char* error, gpointer user_data) {
    NativeRebindRequest* request = user_data;
    Accelerator* accel = global_shortcuts_find_accelerator(request->session, request->native_id);

    (void)result;

    if (error)
        g_warning("Gnoblin could not restore portal shortcut %s: %s", request->native_id, error);
    else if (request->session->closing || !accel)
        native_control_request("shortcut.unbind", request->native_id, NULL, NULL,
                               native_unbind_done, NULL);
    else
        accel->bound = TRUE;

    native_rebind_request_free(request);
}

static void native_control_rebind_shortcuts(void) {
    GHashTableIter iter;
    gpointer value;

    if (!global_shortcuts_sessions)
        return;

    g_hash_table_iter_init(&iter, global_shortcuts_sessions);
    while (g_hash_table_iter_next(&iter, NULL, &value)) {
        GlobalShortcutsSession* session = value;
        if (!session->bound || session->closing)
            continue;

        for (guint i = 0; i < session->shortcuts->len; i++) {
            Shortcut* shortcut = &g_array_index(session->shortcuts, Shortcut, i);

            for (guint j = 0; j < shortcut->accelerators->len; j++) {
                Accelerator* accel = &g_array_index(shortcut->accelerators, Accelerator, j);
                if (accel->bound || !accel->native_id || !accel->accelerator)
                    continue;

                NativeRebindRequest* request = g_new0(NativeRebindRequest, 1);
                request->session = g_object_ref(session);
                request->native_id = g_strdup(accel->native_id);
                native_control_request("shortcut.bind", accel->native_id, accel->accelerator,
                                       "press", native_rebind_done, request);
            }
        }
    }
}

static void shortcuts_handle_free(BindShortcutsHandle* handle) {
    if (handle->invocation)
        g_clear_object(&handle->invocation);
    if (handle->request) {
        if (handle->request->exported)
            request_unexport(handle->request);
        g_object_unref(handle->request);
    }
    g_clear_pointer(&handle->shortcuts, g_variant_unref);
    g_clear_object(&handle->session);
    g_free(handle);
}

G_DEFINE_AUTOPTR_CLEANUP_FUNC(BindShortcutsHandle, shortcuts_handle_free);

static void settings_response_to_shortcuts(GlobalShortcutsSession* session, GVariant* response) {
    GVariantIter iter;
    g_autofree char* shortcut_id = NULL;
    g_autoptr(GVariant) val = NULL;

    g_variant_iter_init(&iter, response);

    while (g_variant_iter_next(&iter, "(s@a{sv})", &shortcut_id, &val)) {
        g_autoptr(GVariant) desc = NULL, shortcuts = NULL;
        Shortcut shortcut = {
            0,
        };

        shortcut.shortcut_id = g_steal_pointer(&shortcut_id);
        shortcut.accelerators = g_array_new(FALSE, FALSE, sizeof(Accelerator));
        g_array_set_clear_func(shortcut.accelerators, (GDestroyNotify)clear_accelerator);

        desc = g_variant_lookup_value(val, "description", NULL);
        if (desc)
            shortcut.desc = g_variant_dup_string(desc, NULL);

        shortcuts = g_variant_lookup_value(val, "shortcuts", NULL);
        if (shortcuts) {
            GVariantIter iter2;
            g_autofree char* shortcut_str = NULL;

            g_variant_iter_init(&iter2, shortcuts);

            while (g_variant_iter_next(&iter2, "s", &shortcut_str)) {
                Accelerator accel = {
                    0,
                };

                accel.accelerator = g_steal_pointer(&shortcut_str);
                g_array_append_val(shortcut.accelerators, accel);
            }

            g_array_append_val(session->shortcuts, shortcut);
        }
    }
}

static void bind_next_approved_accelerator(BindShortcutsHandle* handle);

static void collect_approved_accelerators(GlobalShortcutsSession* session,
                                          BindShortcutsHandle* handle) {
    for (guint i = 0; i < session->shortcuts->len; i++) {
        Shortcut* shortcut = &g_array_index(session->shortcuts, Shortcut, i);

        for (guint j = 0; j < shortcut->accelerators->len; j++)
            g_ptr_array_add(handle->mapped_accelerators,
                            &g_array_index(shortcut->accelerators, Accelerator, j));
    }
}

static void global_shortcuts_session_init(GlobalShortcutsSession* session) {
    session->shortcuts = g_array_new(FALSE, FALSE, sizeof(Shortcut));
    g_array_set_clear_func(session->shortcuts, (GDestroyNotify)clear_shortcut);
}

static void global_shortcuts_session_finalize(GObject* object) {
    GlobalShortcutsSession* session = (GlobalShortcutsSession*)object;

    g_clear_pointer(&session->shortcuts, g_array_unref);
    g_clear_pointer(&session->initial_shortcuts, g_variant_unref);
    g_free(session->app_id);
    g_free(session->bind_handle);

    G_OBJECT_CLASS(global_shortcuts_session_parent_class)->finalize(object);
}

static void native_unbind_done(JsonObject* result, const char* error, gpointer user_data) {
    (void)result;
    (void)user_data;
    if (error)
        g_warning("Gnoblin could not release portal shortcut: %s", error);
}

static void global_shortcuts_session_close(Session* session) {
    GlobalShortcutsSession* shortcuts_session;

    /* When a session is closed, we need to unbind its shortcuts */
    g_debug("Closing session %s", session_get_id(session));

    if (!is_global_shortcuts_session(session)) {
        g_warning("How did the wrong session get here?!");
        return;
    }

    shortcuts_session = (GlobalShortcutsSession*)session;
    shortcuts_session->closing = TRUE;
    g_hash_table_remove(global_shortcuts_sessions, shortcuts_session);

    for (guint i = 0; i < shortcuts_session->shortcuts->len; i++) {
        Shortcut* shortcut = &g_array_index(shortcuts_session->shortcuts, Shortcut, i);

        for (guint j = 0; j < shortcut->accelerators->len; j++) {
            Accelerator* accel = &g_array_index(shortcut->accelerators, Accelerator, j);

            if (accel->bound) {
                native_control_request("shortcut.unbind", accel->native_id, NULL, NULL,
                                       native_unbind_done, NULL);
                accel->bound = FALSE;
            }
        }
    }
}

static void global_shortcuts_session_class_init(GlobalShortcutsSessionClass* klass) {
    GObjectClass* object_class = G_OBJECT_CLASS(klass);
    SessionClass* session_class = (SessionClass*)klass;

    object_class->finalize = global_shortcuts_session_finalize;

    session_class->close = global_shortcuts_session_close;
}

static gboolean handle_create_session(XdpImplGlobalShortcuts* object,
                                      GDBusMethodInvocation* invocation, const char* arg_handle,
                                      const char* arg_session_handle, const char* arg_app_id,
                                      GVariant* arg_options) {
    const char* sender = g_dbus_method_invocation_get_sender(invocation);
    g_autoptr(GError) error = NULL;
    int response;
    Session* session;
    GVariantBuilder results_builder;

    session = g_object_new(global_shortcuts_session_get_type(), "id", arg_session_handle,
                           "peer-name", sender, NULL);

    ((GlobalShortcutsSession*)session)->app_id = g_strdup(arg_app_id);

    if (!session_export(session, g_dbus_method_invocation_get_connection(invocation), &error)) {
        g_clear_object(&session);
        g_warning("Failed to create global shortcuts session: %s", error->message);
        response = 2;
        goto out;
    }

    g_hash_table_add(global_shortcuts_sessions, session);

    g_debug("creating session %s", session_get_id(session));

    response = 0;

out:
    g_variant_builder_init(&results_builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&results_builder, "{sv}", "session_id",
                          g_variant_new_string(session_get_id(session)));
    xdp_impl_global_shortcuts_complete_create_session(object, invocation, response,
                                                      g_variant_builder_end(&results_builder));

    return TRUE;
}

static GVariant* shortcuts_to_response_variant(GArray* shortcuts) {
    GVariantBuilder builder;
    int i;

    g_variant_builder_init(&builder, G_VARIANT_TYPE("a(sa{sv})"));

    for (i = 0; i < shortcuts->len; i++) {
        GVariantBuilder accels_builder;
        Shortcut* shortcut;
        g_autofree char* str = NULL;

        shortcut = &g_array_index(shortcuts, Shortcut, i);
        g_variant_builder_init(&accels_builder, G_VARIANT_TYPE("a{sv}"));

        guint bound_count = 0;
        Accelerator* first_bound = NULL;
        Accelerator* second_bound = NULL;
        for (guint j = 0; j < shortcut->accelerators->len; j++) {
            Accelerator* accel = &g_array_index(shortcut->accelerators, Accelerator, j);
            if (accel->bound) {
                if (bound_count++ == 0)
                    first_bound = accel;
                else if (bound_count == 2)
                    second_bound = accel;
            }
        }

        if (bound_count == 0)
            continue;

        if (shortcut->desc) {
            g_variant_builder_add(&accels_builder, "{sv}", "description",
                                  g_variant_new_string(shortcut->desc));
        }

        if (bound_count == 1) {
            /* Translators: This string refers to a key combination */
            str = g_strdup_printf(_("Press %s"), first_bound->accelerator);
        } else if (bound_count == 2) {
            /* Translators: This string refers to 2 key combinations */
            str = g_strdup_printf(_("Press %s or %s"), first_bound->accelerator,
                                  second_bound->accelerator);
        }

        if (str) {
            g_variant_builder_add(&accels_builder, "{sv}", "trigger_description",
                                  g_variant_new_string(str));
        }

        g_variant_builder_add(&builder, "(s@a{sv})", shortcut->shortcut_id,
                              g_variant_builder_end(&accels_builder));
    }

    return g_variant_ref_sink(g_variant_builder_end(&builder));
}

static gboolean on_request_handle_close_cb(XdpImplRequest* object,
                                           GDBusMethodInvocation* invocation,
                                           BindShortcutsHandle* handle) {
    g_debug("Received Close() from the frontend");
    handle->canceled = TRUE;
    return FALSE;
}

static char* portal_trigger_to_settings(const char* trigger) {
    g_auto(GStrv) pieces = NULL;
    struct {
        const char* from;
        const char* to;
    } map[] = {
        {"CTRL", "<ctrl>"}, {"SHIFT", "<shift>"}, {"ALT", "<alt>"},
        {"NUM", "<mod2>"},  {"LOGO", "<super>"},
    };

    GString* result = g_string_new("");

    pieces = g_strsplit(trigger, "+", 0);

    for (int i = 0; pieces[i]; i++) {
        int j;

        for (j = 0; j < G_N_ELEMENTS(map); j++) {
            if (strcmp(pieces[i], map[j].from) == 0) {
                g_string_append(result, map[j].to);
                break;
            }
        }
        if (j == G_N_ELEMENTS(map))
            g_string_append(result, pieces[i]);
    }

    return g_string_free_and_steal(result);
}

static GVariant* portal_variant_to_empty_response_variant(GVariant* portal) {
    GVariantBuilder builder;
    g_autoptr(GVariantIter) iter = g_variant_iter_new(portal);
    g_autofree char* shortcut_id = NULL;
    g_autoptr(GVariant) val = NULL;

    g_variant_builder_init(&builder, G_VARIANT_TYPE("a(sa{sv})"));

    while (g_variant_iter_next(iter, "(s@a{sv})", &shortcut_id, &val)) {
        g_autofree char* description = NULL;

        g_variant_builder_open(&builder, G_VARIANT_TYPE("(sa{sv})"));
        g_variant_builder_add(&builder, "s", shortcut_id);
        g_variant_builder_open(&builder, G_VARIANT_TYPE("a{sv}"));

        if (g_variant_lookup(val, "description", "s", &description))
            g_variant_builder_add(&builder, "{sv}", "description",
                                  g_variant_new_string(description));

        g_variant_builder_close(&builder);
        g_variant_builder_close(&builder);
    }

    return g_variant_ref_sink(g_variant_builder_end(&builder));
}

static GVariant* portal_shortcuts_to_settings(GVariant* input) {
    /* The portal input looks like this:
     *
     * [
     *   ('id1', {'description':       <'the first shortcut'>,
     *            'preferred_trigger': <'CTRL+a'>}),
     *   ('id2', {'description':       <'the second shortcut'>,
     *            'preferred_trigger': <'SHIFT+ALT+Return'>})
     * ]
     *
     * The settings provider input looks like this:
     *
     * [
     *   ('id1', {'description':       <'the first shortcut'>,
     *            'preferred_trigger': <'<ctrl>a'>}),
     *   ('id2', {'description':       <'the second shortcut'>,
     *            'preferred_trigger': <'<shift><alt>Return'>})
     * ]
     */
    GVariantIter iter;
    g_autofree char* shortcut_id = NULL;
    g_autoptr(GVariant) options = NULL;
    g_auto(GVariantBuilder) builder = G_VARIANT_BUILDER_INIT(G_VARIANT_TYPE("a(sa{sv})"));

    g_variant_iter_init(&iter, input);
    while (g_variant_iter_next(&iter, "(s@a{sv})", &shortcut_id, &options)) {
        g_autoptr(GVariant) v = g_steal_pointer(&options);
        g_autofree char* id = g_steal_pointer(&shortcut_id);
        g_autofree char* description = NULL;
        g_autofree char* preferred_trigger = NULL;
        g_autofree char* settings_trigger = NULL;

        g_variant_lookup(v, "preferred_trigger", "s", &preferred_trigger);

        if (!g_variant_lookup(v, "description", "s", &description))
            description = g_strdup(_("Undocumented shortcut"));

        g_variant_builder_open(&builder, G_VARIANT_TYPE("(sa{sv})"));
        g_variant_builder_add(&builder, "s", id);
        g_variant_builder_open(&builder, G_VARIANT_TYPE("a{sv}"));
        g_variant_builder_add(&builder, "{sv}", "description", g_variant_new_string(description));
        if (preferred_trigger) {
            settings_trigger = portal_trigger_to_settings(preferred_trigger);
            g_variant_builder_add(&builder, "{sv}", "preferred_trigger",
                                  g_variant_new_string(settings_trigger));
        }

        g_variant_builder_close(&builder);
        g_variant_builder_close(&builder);
    }

    return g_variant_ref_sink(g_variant_builder_end(&builder));
}

static void finish_native_bind_batch(BindShortcutsHandle* handle) {
    GlobalShortcutsSession* session = (GlobalShortcutsSession*)handle->session;
    g_autoptr(GVariant) shortcuts = shortcuts_to_response_variant(session->shortcuts);

    if (handle->canceled && !handle->rebind) {
        for (guint i = 0; i < session->shortcuts->len; i++) {
            Shortcut* shortcut = &g_array_index(session->shortcuts, Shortcut, i);
            for (guint j = 0; j < shortcut->accelerators->len; j++) {
                Accelerator* accel = &g_array_index(shortcut->accelerators, Accelerator, j);
                if (accel->bound)
                    native_control_request("shortcut.unbind", accel->native_id, NULL, NULL,
                                           native_unbind_done, NULL);
                accel->bound = FALSE;
            }
        }
        GVariantBuilder canceled_results;
        g_variant_builder_init(&canceled_results, G_VARIANT_TYPE_VARDICT);
        xdp_impl_global_shortcuts_complete_bind_shortcuts(global_shortcuts, handle->invocation, 1,
                                                          g_variant_builder_end(&canceled_results));
        return;
    }

    if (handle->rebind) {
        if (session->shortcuts->len == 0) {
            session->bound = FALSE;
            shortcuts = portal_variant_to_empty_response_variant(session->initial_shortcuts);
        }
        if (shortcuts)
            xdp_impl_global_shortcuts_emit_shortcuts_changed(
                global_shortcuts, session_get_id(handle->session), g_variant_ref(shortcuts));
        if (handle->invocation)
            org_gnoblin_shell_global_shortcuts_rebind_complete_rebind_shortcuts(
                global_shortcuts_rebind, handle->invocation);
        return;
    }

    if (shortcuts)
        xdp_impl_global_shortcuts_emit_shortcuts_changed(
            global_shortcuts, session_get_id(handle->session), g_variant_ref(shortcuts));

    GVariantBuilder results_builder;
    g_variant_builder_init(&results_builder, G_VARIANT_TYPE_VARDICT);
    if (shortcuts)
        g_variant_builder_add(&results_builder, "{sv}", "shortcuts", shortcuts);
    xdp_impl_global_shortcuts_complete_bind_shortcuts(global_shortcuts, handle->invocation,
                                                      handle->failed ? 2 : 0,
                                                      g_variant_builder_end(&results_builder));
}

static void native_bind_done(JsonObject* result, const char* error, gpointer user_data) {
    BindShortcutsHandle* handle = user_data;
    GlobalShortcutsSession* session = (GlobalShortcutsSession*)handle->session;
    Accelerator* accel =
        g_ptr_array_index(handle->mapped_accelerators, handle->next_accelerator - 1);

    (void)result;

    if (error) {
        handle->failed = TRUE;
        g_warning("Gnoblin could not bind approved accelerator %s: %s", accel->accelerator, error);
    } else if (session->closing)
        native_control_request("shortcut.unbind", accel->native_id, NULL, NULL, native_unbind_done,
                               NULL);
    else
        accel->bound = TRUE;

    if (session->closing)
        accel->bound = FALSE;

    bind_next_approved_accelerator(handle);
}

static void bind_next_approved_accelerator(BindShortcutsHandle* handle) {
    if (handle->next_accelerator >= handle->mapped_accelerators->len) {
        finish_native_bind_batch(handle);
        shortcuts_handle_free(handle);
        return;
    }

    Accelerator* accel = g_ptr_array_index(handle->mapped_accelerators, handle->next_accelerator++);
    GlobalShortcutsSession* session = (GlobalShortcutsSession*)handle->session;
    if (handle->canceled || session->closing) {
        handle->failed = TRUE;
        accel->bound = FALSE;
        bind_next_approved_accelerator(handle);
        return;
    }

    g_free(accel->native_id);
    accel->native_id =
        g_strdup_printf("portal-%" G_GUINT64_FORMAT, ++native_control.next_binding_id);
    native_control_request("shortcut.bind", accel->native_id, accel->accelerator, "press",
                           native_bind_done, handle);
}

static void settings_bind_shortcuts_done(GObject* source, GAsyncResult* result, gpointer data) {
    OrgGnoblinShellGlobalShortcutsProvider* proxy = (OrgGnoblinShellGlobalShortcutsProvider*)source;
    g_autoptr(BindShortcutsHandle) handle = data;
    GlobalShortcutsSession* session = (GlobalShortcutsSession*)handle->session;
    g_autoptr(GVariant) response = NULL;
    GVariantBuilder results_builder;
    g_autoptr(GError) error = NULL;

    if (!org_gnoblin_shell_global_shortcuts_provider_call_bind_shortcuts_finish(proxy, &response,
                                                                                result, &error)) {
        g_debug("Error from GlobalShortcutsProvider: %s", error->message);
        g_variant_builder_init(&results_builder, G_VARIANT_TYPE_VARDICT);
        xdp_impl_global_shortcuts_complete_bind_shortcuts(global_shortcuts, handle->invocation, 2,
                                                          g_variant_builder_end(&results_builder));
        return;
    }

    if (handle->canceled) {
        g_variant_builder_init(&results_builder, G_VARIANT_TYPE_VARDICT);
        xdp_impl_global_shortcuts_complete_bind_shortcuts(global_shortcuts, handle->invocation, 1,
                                                          g_variant_builder_end(&results_builder));
        return;
    }

    settings_response_to_shortcuts(session, response);
    collect_approved_accelerators(session, handle);
    bind_next_approved_accelerator(g_steal_pointer(&handle));
}

static gboolean handle_bind_shortcuts(XdpImplGlobalShortcuts* object,
                                      GDBusMethodInvocation* invocation, const char* arg_handle,
                                      const char* arg_session_handle, GVariant* arg_shortcuts,
                                      const char* arg_parent_window, GVariant* arg_options) {
    const char* sender;
    g_autoptr(Request) request = NULL;
    Session* session;
    GlobalShortcutsSession* shortcuts_session;
    GVariantBuilder results_builder;
    g_autoptr(GVariant) settings_shortcuts = NULL;
    g_autoptr(BindShortcutsHandle) handle = NULL;

    sender = g_dbus_method_invocation_get_sender(invocation);

    session = lookup_session(arg_session_handle);
    if (!session) {
        g_warning("Tried to bind shortcuts on non-existing %s", arg_session_handle);
        goto out;
    }

    if (!is_global_shortcuts_session(session)) {
        g_warning("Tried to bind shortcuts on the wrong session type");
        goto out;
    }

    shortcuts_session = (GlobalShortcutsSession*)session;

    if (shortcuts_session->bound) {
        g_warning("Session already has bound shortcuts");
        goto out;
    }

    request = request_new(sender, shortcuts_session->app_id, arg_handle);
    request_export(request, g_dbus_method_invocation_get_connection(invocation));

    shortcuts_session->initial_shortcuts = g_variant_ref(arg_shortcuts);
    g_free(shortcuts_session->bind_handle);
    shortcuts_session->bind_handle = g_strdup(arg_handle);
    shortcuts_session->bound = TRUE;

    g_assert(object == global_shortcuts);
    handle = g_new0(BindShortcutsHandle, 1);
    handle->session = g_object_ref(session);
    handle->invocation = g_object_ref(invocation);
    handle->request = g_object_ref(request);
    handle->shortcuts = g_variant_ref(arg_shortcuts);
    handle->mapped_accelerators = g_ptr_array_new();

    g_signal_connect(request, "handle-close", G_CALLBACK(on_request_handle_close_cb), handle);

    settings_shortcuts = portal_shortcuts_to_settings(arg_shortcuts);

    org_gnoblin_shell_global_shortcuts_provider_call_bind_shortcuts(
        settings, shortcuts_session->app_id, arg_parent_window, settings_shortcuts, arg_handle,
        NULL, settings_bind_shortcuts_done, g_steal_pointer(&handle));
    return TRUE;

out:
    g_variant_builder_init(&results_builder, G_VARIANT_TYPE_VARDICT);
    xdp_impl_global_shortcuts_complete_bind_shortcuts(object, invocation, 2,
                                                      g_variant_builder_end(&results_builder));
    return TRUE;
}

static gboolean handle_list_shortcuts(XdpImplGlobalShortcuts* object,
                                      GDBusMethodInvocation* invocation, const char* arg_handle,
                                      const char* arg_session_handle) {
    Session* session;
    GlobalShortcutsSession* shortcuts_session;
    GVariantBuilder results_builder;
    g_autoptr(GVariant) shortcuts = NULL;
    int response;

    session = lookup_session(arg_session_handle);
    if (!session) {
        g_warning("Tried to list shortcuts on non-existing %s", arg_session_handle);
        response = 2;
        goto out;
    }

    if (!is_global_shortcuts_session(session)) {
        g_warning("Tried to bind shortcuts on the wrong session type");
        response = 2;
        goto out;
    }

    response = 0;
    shortcuts_session = (GlobalShortcutsSession*)session;
    shortcuts = shortcuts_to_response_variant(shortcuts_session->shortcuts);

out:
    g_variant_builder_init(&results_builder, G_VARIANT_TYPE_VARDICT);
    if (shortcuts)
        g_variant_builder_add(&results_builder, "{sv}", "shortcuts", shortcuts);
    xdp_impl_global_shortcuts_complete_list_shortcuts(global_shortcuts, invocation, response,
                                                      g_variant_builder_end(&results_builder));
    return TRUE;
}

static gboolean match_bind_handle(Session* session, gconstpointer data) {
    const char* bind_handle = data;
    GlobalShortcutsSession* shortcuts_session;

    if (!is_global_shortcuts_session(session))
        return FALSE;

    shortcuts_session = (GlobalShortcutsSession*)session;

    if (g_strcmp0(shortcuts_session->bind_handle, bind_handle) != 0)
        return FALSE;

    return TRUE;
}

static void rebind_session_shortcuts(GlobalShortcutsSession* session, GVariant* shortcuts,
                                     GDBusMethodInvocation* invocation) {
    g_autoptr(BindShortcutsHandle) handle = NULL;

    handle = g_new0(BindShortcutsHandle, 1);
    handle->session = g_object_ref((Session*)session);
    handle->invocation = invocation ? g_object_ref(invocation) : NULL;
    handle->shortcuts = g_variant_ref(shortcuts);
    handle->mapped_accelerators = g_ptr_array_new();
    handle->rebind = TRUE;

    /* Queue old releases before replacement binds on the same socket. */
    for (guint i = 0; i < session->shortcuts->len; i++) {
        Shortcut* shortcut = &g_array_index(session->shortcuts, Shortcut, i);
        for (guint j = 0; j < shortcut->accelerators->len; j++) {
            Accelerator* accel = &g_array_index(shortcut->accelerators, Accelerator, j);
            if (accel->bound)
                native_control_request("shortcut.unbind", accel->native_id, NULL, NULL,
                                       native_unbind_done, NULL);
        }
    }
    g_array_remove_range(session->shortcuts, 0, session->shortcuts->len);
    settings_response_to_shortcuts(session, shortcuts);
    collect_approved_accelerators(session, handle);
    bind_next_approved_accelerator(g_steal_pointer(&handle));
}

static gboolean handle_rebind_shortcuts(OrgGnoblinShellGlobalShortcutsRebind* object,
                                        GDBusMethodInvocation* invocation, const char* arg_app_id,
                                        GVariant* arg_shortcuts, const char* arg_handle) {
    Session* session;

    session = find_session(match_bind_handle, arg_handle);

    if (!session) {
        g_debug("Session for app %s handle %s not found in RebindShortcuts", arg_app_id,
                arg_handle);
        org_gnoblin_shell_global_shortcuts_rebind_complete_rebind_shortcuts(object, invocation);
        return TRUE;
    }

    g_debug("Re-binding shortcuts of session %s", session_get_id(session));

    rebind_session_shortcuts((GlobalShortcutsSession*)session, arg_shortcuts, invocation);

    return TRUE;
}

static void settings_configure_shortcuts_done(GObject* source, GAsyncResult* result,
                                              gpointer data) {
    OrgGnoblinShellGlobalShortcutsProvider* proxy = (OrgGnoblinShellGlobalShortcutsProvider*)source;
    g_autofree char* bind_handle = data;
    g_autoptr(GVariant) response = NULL;
    g_autoptr(GError) error = NULL;
    Session* session;
    GlobalShortcutsSession* shortcuts_session;

    if (!org_gnoblin_shell_global_shortcuts_provider_call_configure_shortcuts_finish(
            proxy, &response, result, &error)) {
        g_debug("Error from GlobalShortcutsProvider ConfigureShortcuts: %s", error->message);
        return;
    }

    session = find_session(match_bind_handle, bind_handle);
    if (!session) {
        g_debug("Session for handle %s not found in ConfigureShortcuts response", bind_handle);
        return;
    }

    shortcuts_session = (GlobalShortcutsSession*)session;

    g_debug("Re-binding shortcuts from ConfigureShortcuts response for session %s",
            session_get_id(session));

    rebind_session_shortcuts(shortcuts_session, response, NULL);
}

static gboolean handle_configure_shortcuts(XdpImplGlobalShortcuts* object,
                                           GDBusMethodInvocation* invocation,
                                           const char* arg_session_handle,
                                           const char* arg_parent_window, GVariant* arg_options) {
    Session* session;
    GlobalShortcutsSession* shortcuts_session;

    session = lookup_session(arg_session_handle);
    if (!session) {
        g_warning("Tried to configure shortcuts on non-existing %s", arg_session_handle);
        g_dbus_method_invocation_return_error(invocation, G_DBUS_ERROR, G_DBUS_ERROR_INVALID_ARGS,
                                              "Session %s not found", arg_session_handle);
        return TRUE;
    }

    if (!is_global_shortcuts_session(session)) {
        g_warning("Tried to configure shortcuts on the wrong session type");
        g_dbus_method_invocation_return_error(invocation, G_DBUS_ERROR, G_DBUS_ERROR_INVALID_ARGS,
                                              "Wrong session type");
        return TRUE;
    }

    shortcuts_session = (GlobalShortcutsSession*)session;

    g_debug("Configuring shortcuts for session %s, app_id %s", session_get_id(session),
            shortcuts_session->app_id);

    if (shortcuts_session->bound) {
        org_gnoblin_shell_global_shortcuts_provider_call_configure_shortcuts(
            settings, shortcuts_session->app_id, arg_parent_window, shortcuts_session->bind_handle,
            NULL, settings_configure_shortcuts_done, g_strdup(shortcuts_session->bind_handle));
    } else if (shortcuts_session->initial_shortcuts) {
        g_autoptr(GVariant) settings_shortcuts = NULL;

        g_debug("Recovering session by calling BindShortcuts internally");
        shortcuts_session->bound = TRUE;
        settings_shortcuts = portal_shortcuts_to_settings(shortcuts_session->initial_shortcuts);
        org_gnoblin_shell_global_shortcuts_provider_call_bind_shortcuts(
            settings, shortcuts_session->app_id, arg_parent_window, settings_shortcuts,
            shortcuts_session->bind_handle, NULL, settings_configure_shortcuts_done,
            g_strdup(shortcuts_session->bind_handle));
    } else {
        g_dbus_method_invocation_return_error(invocation, G_DBUS_ERROR, G_DBUS_ERROR_INVALID_ARGS,
                                              "Session not bound, BindShortcuts was not called");
        return TRUE;
    }

    xdp_impl_global_shortcuts_complete_configure_shortcuts(global_shortcuts, invocation);
    return TRUE;
}

static gboolean match_native_binding(Session* session, gconstpointer data) {
    const char* id = data;
    GlobalShortcutsSession* shortcuts_session;

    if (!is_global_shortcuts_session(session))
        return FALSE;

    shortcuts_session = (GlobalShortcutsSession*)session;
    for (guint i = 0; i < shortcuts_session->shortcuts->len; i++) {
        Shortcut* shortcut = &g_array_index(shortcuts_session->shortcuts, Shortcut, i);
        for (guint j = 0; j < shortcut->accelerators->len; j++) {
            Accelerator* accel = &g_array_index(shortcut->accelerators, Accelerator, j);
            if (accel->bound && g_strcmp0(accel->native_id, id) == 0)
                return TRUE;
        }
    }
    return FALSE;
}

static GlobalShortcutsSession* global_shortcuts_find_binding(const char* id,
                                                             const char** shortcut_out) {
    Session* session = find_session(match_native_binding, id);

    if (!session)
        return NULL;

    GlobalShortcutsSession* shortcuts_session = (GlobalShortcutsSession*)session;
    for (guint i = 0; i < shortcuts_session->shortcuts->len; i++) {
        Shortcut* shortcut = &g_array_index(shortcuts_session->shortcuts, Shortcut, i);
        for (guint j = 0; j < shortcut->accelerators->len; j++) {
            Accelerator* accel = &g_array_index(shortcut->accelerators, Accelerator, j);
            if (accel->bound && g_strcmp0(accel->native_id, id) == 0) {
                *shortcut_out = shortcut->shortcut_id;
                return shortcuts_session;
            }
        }
    }
    return NULL;
}

gboolean global_shortcuts_init(GDBusConnection* bus, GError** error) {
    global_shortcuts = xdp_impl_global_shortcuts_skeleton_new();
    xdp_impl_global_shortcuts_set_version(global_shortcuts, 2);

    g_signal_connect(global_shortcuts, "handle-bind-shortcuts", G_CALLBACK(handle_bind_shortcuts),
                     NULL);
    g_signal_connect(global_shortcuts, "handle-create-session", G_CALLBACK(handle_create_session),
                     NULL);
    g_signal_connect(global_shortcuts, "handle-list-shortcuts", G_CALLBACK(handle_list_shortcuts),
                     NULL);
    g_signal_connect(global_shortcuts, "handle-configure-shortcuts",
                     G_CALLBACK(handle_configure_shortcuts), NULL);

    if (!g_dbus_interface_skeleton_export(G_DBUS_INTERFACE_SKELETON(global_shortcuts), bus,
                                          DESKTOP_PORTAL_OBJECT_PATH, error))
        return FALSE;

    global_shortcuts_rebind = org_gnoblin_shell_global_shortcuts_rebind_skeleton_new();
    g_signal_connect(global_shortcuts_rebind, "handle-rebind-shortcuts",
                     G_CALLBACK(handle_rebind_shortcuts), NULL);

    if (!g_dbus_interface_skeleton_export(G_DBUS_INTERFACE_SKELETON(global_shortcuts_rebind), bus,
                                          "/org/gnoblin/Shell/GlobalShortcuts", error))
        return FALSE;

    native_control.queued = g_queue_new();
    native_control.pending = g_hash_table_new_full(g_str_hash, g_str_equal, g_free, NULL);
    global_shortcuts_sessions = g_hash_table_new(g_direct_hash, g_direct_equal);

    settings = org_gnoblin_shell_global_shortcuts_provider_proxy_new_sync(
        bus, G_DBUS_PROXY_FLAGS_NONE, "org.gnoblin.Shell.GlobalShortcutsProvider",
        "/org/gnoblin/Shell/GlobalShortcutsProvider", NULL, error);
    if (settings == NULL)
        return FALSE;

    g_dbus_proxy_set_default_timeout(G_DBUS_PROXY(settings), G_MAXINT);

    g_debug("providing %s",
            g_dbus_interface_skeleton_get_info(G_DBUS_INTERFACE_SKELETON(global_shortcuts))->name);

    return TRUE;
}
