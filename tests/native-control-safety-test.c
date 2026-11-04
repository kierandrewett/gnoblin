/* Built by native-control-safety.py from verbatim production functions and structs.
 *
 * Contract 1: a Lua runtime abort can come before compositor teardown, with native work still in
 * flight. The control must then survive until teardown has finished and the last async callback
 * has returned, and be freed exactly once. Freeing it earlier is a use after free in whichever
 * of the two runs second. The harness links ASan when the toolchain has it; the test also records
 * every free, so an early free fails it without a sanitizer.
 *
 * Contract 2: ending a shortcut session ends the session only. The binding stays registered, armed
 * and owned by the Lua generation or socket client that bound it, and activates for that owner
 * again, with a new session.
 *
 * Contract 3: gnoblin.input.grab is refused while a key is held, with the message that names the
 * key, and granted again once the key lifts. The set of held keys must not keep a key that can no
 * longer lift: a keyboard that is removed drops its keys, and the same key on two keyboards is two
 * entries. (A release swallowed by the diagnostic console is the stage filter's order of calls, in
 * a Mutter patch; the nested-session smoke covers it.)
 *
 * Failure modes this catches: the teardown flag dropped from the free guard, the free moved before
 * the flag, abort freeing; session end that unbinds, disarms, re-owns or releases the accelerator;
 * end reaching a binding of another owner, another generation or another session; a grab granted
 * with a key down, or refused for ever by a key whose keyboard is gone.
 *
 * Mutter, Clutter and the Lua runtime are stubs below. They record what the production code asks
 * of them; they do not model a desktop. */
#include <gio/gio.h>
#include <json-glib/json-glib.h>
#include <string.h>
#include <sys/types.h>

#include "gnoblin-api-operations.h"
#include "gnoblin-chord.h"

#include <xkbcommon/xkbcommon-compose.h>
#include <xkbcommon/xkbcommon.h>

typedef struct _MetaDisplay MetaDisplay;
typedef struct _MetaWaylandCompositor MetaWaylandCompositor;
typedef struct _MetaWorkspaceManager MetaWorkspaceManager;
typedef struct _MetaCursorTracker MetaCursorTracker;
typedef struct _MetaOrientationManager MetaOrientationManager;
typedef struct _MetaMonitorManager MetaMonitorManager;
typedef struct _MetaBackend MetaBackend;
typedef struct _MetaRemoteAccessController MetaRemoteAccessController;
typedef struct _MetaKeymapDescription MetaKeymapDescription;
typedef struct _ClutterSeat ClutterSeat;
typedef struct _GnoblinTouchpadRouter GnoblinTouchpadRouter;
typedef struct _GnoblinLocationAgent GnoblinLocationAgent;
typedef struct _GnoblinRuntimeCache GnoblinRuntimeCache;
typedef struct _GnoblinRuntimeReader GnoblinRuntimeReader;
typedef struct _GnoblinRuntimeWriter GnoblinRuntimeWriter;
typedef struct _GnoblinAuthAgent GnoblinAuthAgent;
typedef gboolean (*GnoblinNativeConsoleToggleFunc)(MetaDisplay* display, gpointer user_data);
typedef void (*GnoblinNativeDebugFunc)(gboolean enabled, gpointer user_data);
typedef guint32 xkb_mod_mask_t;
typedef struct _GnoblinNativeControl GnoblinNativeControl;

typedef struct _ClutterInputDevice ClutterInputDevice;

typedef enum {
    CLUTTER_NOTHING,
    CLUTTER_MOTION,
    CLUTTER_KEY_PRESS,
    CLUTTER_KEY_RELEASE
} ClutterEventType;

typedef enum {
    CLUTTER_EVENT_NONE = 0,
    CLUTTER_EVENT_FLAG_SYNTHETIC = 1 << 0,
    CLUTTER_EVENT_FLAG_INPUT_METHOD = 1 << 1,
    CLUTTER_EVENT_FLAG_REPEATED = 1 << 2,
} ClutterEventFlags;

typedef struct _ClutterEvent {
    guint32 time;
    guint32 state;
    ClutterEventType type;
    ClutterEventFlags flags;
    ClutterInputDevice* source;
    guint32 code;
    xkb_keysym_t keysym;
} ClutterEvent;

#define CLUTTER_SUPER_MASK (1u << 26)
#define CLUTTER_CONTROL_MASK (1u << 2)
#define CLUTTER_MOD1_MASK (1u << 3)
#define META_KEYBINDING_ACTION_NONE 0

#include "production-types.inc"

/* ---- What the stubs record. ---- */
static GPtrArray* released;       /* controls handed to the final free */
static GPtrArray* runtime_events; /* "event id session_id reason" per event sent to Lua */
static GPtrArray* socket_lines;   /* lines written to socket clients */
static guint64 config_generation;
static guint next_action;
static guint grabs;
static guint ungrabs;
static guint key_capture_stops;
static guint completions;

/* ---- Prototypes the extracted code calls before it is defined. ---- */
guint64 native_config_generation(GnoblinNativeControl* control);
void dynamic_shortcut_end_session(GnoblinNativeControl* control, NativeDynamicShortcut* shortcut,
                                  const char* reason);
void dynamic_shortcut_publish_event(GnoblinNativeControl* control, NativeDynamicShortcut* shortcut,
                                    const char* event, GVariant* payload);
gboolean native_shortcut_session_timeout(gpointer user_data);
void gnoblin_control_dynamic_shortcut_end_session(GnoblinNativeControl* control,
                                                  NativeDynamicShortcut* shortcut,
                                                  const char* reason);
void gnoblin_control_dynamic_shortcut_free(gpointer data);
guint gnoblin_control_dynamic_shortcut_count(GnoblinNativeControl* control, Client* owner);
NativeDynamicShortcut* gnoblin_control_find_dynamic_shortcut(Client* owner, const char* id);
NativeDynamicShortcut* gnoblin_control_find_runtime_dynamic_shortcut(GnoblinNativeControl* control,
                                                                     const char* owner_id,
                                                                     const char* id);
gboolean gnoblin_control_dynamic_shortcut_id_valid(const char* id);
NativeDynamicShortcut* gnoblin_control_arm_bare_super_shortcut(GnoblinNativeControl* control,
                                                               const char* id, const char* owner_id,
                                                               Client* client, guint64 session_id,
                                                               GError** error);
char* gnoblin_control_dynamic_shortcut_chord(const char* accelerator, char** error);
char* gnoblin_control_encode_response(const char* id, JsonNode* result, const char* message);
JsonNode* json_from_variant(GVariant* value);
void native_control_maybe_free_stopped(GnoblinNativeControl* control);
void native_control_finish_teardown(GnoblinNativeControl* control);
void native_runtime_abort(GnoblinNativeControl* control);
void gnoblin_control_maybe_free_stopped(GnoblinNativeControl* control);
void gnoblin_control_clear_runtime_dynamic_shortcuts(GnoblinNativeControl* control,
                                                     const char* reason);
void gnoblin_control_remove_dynamic_shortcut(GnoblinNativeControl* control,
                                             NativeDynamicShortcut* shortcut);
void gnoblin_control_input_grab_end(GnoblinNativeControl* control, const char* reason,
                                    gboolean publish);

/* ---- Boundary stubs. ---- */
void gnoblin_control_input_sources_release(GnoblinNativeControl* control) {
    g_ptr_array_add(released, control);
}

void gnoblin_control_clear_pending_location_authorizations(GnoblinNativeControl* control) {}
void gnoblin_control_corner_toolkit_cache_shutdown(GnoblinNativeControl* control) {}
void native_runtime_fail_pending_requests(GnoblinNativeControl* control, const char* reason) {}
void stop_native_shortcut_capture(GnoblinNativeControl* control, gboolean complete, gboolean ok,
                                  const char* accelerator, const char* error_code,
                                  const char* message) {}
void gnoblin_control_cancel_window_drags(GnoblinNativeControl* control, const char* reason) {}
void revoke_text_targets(GnoblinNativeControl* control) {}
void revoke_focus_contexts(GnoblinNativeControl* control) {}
void revoke_menu_contexts(GnoblinNativeControl* control) {}

const char* gnoblin_control_operation_error_code(const GError* error) {
    return "internal";
}

void gnoblin_control_dispatch_operation_completion_full(GnoblinNativeControl* control,
                                                        gint64 request_id, const char* method,
                                                        gboolean ok, JsonNode* result,
                                                        const char* error_code, const char* message,
                                                        gboolean dispatch_lua) {
    completions++;
}

guint64 native_config_generation(GnoblinNativeControl* control) {
    return config_generation;
}

gboolean meta_wayland_session_lock_is_active(MetaWaylandCompositor* compositor) {
    return FALSE;
}

guint meta_display_grab_accelerator(MetaDisplay* display, const char* accelerator, guint flags) {
    grabs++;
    return next_action++;
}

void meta_display_ungrab_accelerator(MetaDisplay* display, guint action) {
    ungrabs++;
}

void meta_display_stop_native_key_capture(MetaDisplay* display) {
    key_capture_stops++;
}

void meta_display_unregister_native_key_capture_handler(MetaDisplay* display, gpointer data) {}

gboolean meta_display_can_start_native_key_capture(MetaDisplay* display) {
    return TRUE;
}

gboolean meta_display_register_native_key_capture_handler(MetaDisplay* display, gpointer handler,
                                                          gpointer data, GDestroyNotify destroy) {
    return TRUE;
}

gboolean meta_display_start_native_key_capture(MetaDisplay* display) {
    return TRUE;
}

void native_shortcut_capture_key(const ClutterEvent* event, gpointer user_data) {}
void prune_focus_contexts(GnoblinNativeControl* control, gint64 now) {}
void capture_focus_identity(GnoblinNativeControl* control, NativeFocusContext* context,
                            const ClutterEvent* event) {}

gboolean focus_token_random(char token[65]) {
    static guint serial;
    g_snprintf(token, 65, "token-%u", ++serial);
    return TRUE;
}

gboolean focus_token_exists(GnoblinNativeControl* control, const char* token) {
    return FALSE;
}

guint32 clutter_event_get_time(const ClutterEvent* event) {
    return event->time;
}

guint32 clutter_event_get_state(const ClutterEvent* event) {
    return event->state;
}

ClutterEventType clutter_event_type(const ClutterEvent* event) {
    return event->type;
}

ClutterEventFlags clutter_event_get_flags(const ClutterEvent* event) {
    return event->flags;
}

ClutterInputDevice* clutter_event_get_source_device(const ClutterEvent* event) {
    return event->source;
}

guint32 clutter_event_get_key_code(const ClutterEvent* event) {
    return event->code;
}

guint clutter_event_get_key_symbol(const ClutterEvent* event) {
    return event->keysym;
}

ClutterEvent* clutter_event_copy(const ClutterEvent* event) {
    return g_memdup2(event, sizeof *event);
}

void clutter_event_free(ClutterEvent* event) {
    g_free(event);
}

void send_response(Client* client, char* response) {
    g_ptr_array_add(socket_lines, response);
}

gboolean native_runtime_dispatch_event(GnoblinNativeControl* control, const char* event,
                                       GVariant* payload) {
    const char* id = "";
    const char* reason = "";
    guint64 session_id = 0;
    g_variant_lookup(payload, "id", "&s", &id);
    g_variant_lookup(payload, "reason", "&s", &reason);
    g_variant_lookup(payload, "session_id", "t", &session_id);
    g_ptr_array_add(runtime_events, g_strdup_printf("%s %s %" G_GUINT64_FORMAT " %s", event, id,
                                                    session_id, reason));
    return TRUE;
}

#include "production.inc"

/* ---- Helpers. ---- */
static void reset(void) {
    g_ptr_array_set_size(released, 0);
    g_ptr_array_set_size(runtime_events, 0);
    g_ptr_array_set_size(socket_lines, 0);
    config_generation = 5;
    next_action = 100;
    grabs = ungrabs = key_capture_stops = completions = 0;
}

static GnoblinNativeControl* new_control(void) {
    GnoblinNativeControl* control = g_new0(GnoblinNativeControl, 1);
    control->dynamic_shortcuts = g_hash_table_new_full(g_direct_hash, g_direct_equal, NULL,
                                                       gnoblin_control_dynamic_shortcut_free);
    control->focus_contexts = g_hash_table_new_full(g_int64_hash, g_int64_equal, g_free, g_free);
    return control;
}

static void free_control(GnoblinNativeControl* control) {
    g_clear_pointer(&control->dynamic_shortcuts, g_hash_table_unref);
    g_clear_pointer(&control->focus_contexts, g_hash_table_unref);
    g_free(control);
}

static guint count_events(const char* prefix) {
    guint count = 0;
    for (guint i = 0; i < runtime_events->len; i++)
        count += g_str_has_prefix(g_ptr_array_index(runtime_events, i), prefix);
    return count;
}

static void expect_event(const char* expected) {
    for (guint i = 0; i < runtime_events->len; i++)
        if (g_str_equal(g_ptr_array_index(runtime_events, i), expected))
            return;
    g_error("event \"%s\" was not sent to the runtime", expected);
}

static ClutterEvent key_event = {.time = 1000, .state = 0};

/* ---- Contract 1: teardown ordering. ---- */
static PowerCall* start_power_call(GnoblinNativeControl* control) {
    PowerCall* call = g_new0(PowerCall, 1);
    call->control = control;
    call->operation_id = 9;
    call->method = "session.poweroff";
    control->pending_power_calls++;
    return call;
}

static void test_abort_alone_never_frees(void) {
    reset();
    GnoblinNativeControl* control = g_new0(GnoblinNativeControl, 1);
    native_runtime_abort(control);
    g_assert_true(control->stopping);
    g_assert_false(control->teardown_complete);
    g_assert_cmpuint(released->len, ==, 0);
    native_control_maybe_free_stopped(control);
    g_assert_cmpuint(released->len, ==, 0);
    native_control_finish_teardown(control);
    g_assert_cmpuint(released->len, ==, 1);
    g_assert_true(g_ptr_array_index(released, 0) == control);
}

static void test_async_callback_after_abort_does_not_free_before_teardown(void) {
    reset();
    GnoblinNativeControl* control = g_new0(GnoblinNativeControl, 1);
    PowerCall* call = start_power_call(control);

    native_runtime_abort(control);
    g_assert_cmpuint(released->len, ==, 0);

    /* logind answers after the runtime died and before the compositor finished tearing down. */
    power_finish(call, NULL);
    g_assert_cmpuint(control->pending_power_calls, ==, 0);
    g_assert_cmpuint(released->len, ==, 0);
    g_assert_cmpuint(completions, ==, 0);

    /* Teardown reads and clears the control after the callback; it must still be there. */
    g_assert_true(control->stopping);
    control->path = g_strdup("still readable");
    g_clear_pointer(&control->path, g_free);
    native_control_finish_teardown(control);
    g_assert_cmpuint(released->len, ==, 1);
    g_assert_true(g_ptr_array_index(released, 0) == control);
}

static void test_teardown_waits_for_the_last_async_callback(void) {
    reset();
    GnoblinNativeControl* control = g_new0(GnoblinNativeControl, 1);
    PowerCall* call = start_power_call(control);

    native_runtime_abort(control);
    native_control_finish_teardown(control);
    g_assert_true(control->teardown_complete);
    g_assert_cmpuint(released->len, ==, 0);

    power_finish(call, NULL);
    g_assert_cmpuint(released->len, ==, 1);
    g_assert_true(g_ptr_array_index(released, 0) == control);
    g_assert_cmpuint(completions, ==, 0);
}

static void test_every_kind_of_work_in_flight_holds_the_control(void) {
    const gsize offsets[] = {
        G_STRUCT_OFFSET(GnoblinNativeControl, pending_input_source_ops),
        G_STRUCT_OFFSET(GnoblinNativeControl, pending_ibus_queries),
        G_STRUCT_OFFSET(GnoblinNativeControl, pending_thumbnail_count),
        G_STRUCT_OFFSET(GnoblinNativeControl, pending_power_calls),
    };
    for (guint i = 0; i < G_N_ELEMENTS(offsets); i++) {
        reset();
        GnoblinNativeControl* control = g_new0(GnoblinNativeControl, 1);
        guint* pending = (guint*)((guchar*)control + offsets[i]);
        *pending = 1;
        native_runtime_abort(control);
        native_control_finish_teardown(control);
        g_assert_cmpuint(released->len, ==, 0);
        *pending = 0;
        native_control_maybe_free_stopped(control);
        g_assert_cmpuint(released->len, ==, 1);
    }
}

static void test_running_control_is_never_freed_by_teardown_bookkeeping(void) {
    reset();
    GnoblinNativeControl* control = g_new0(GnoblinNativeControl, 1);
    native_control_maybe_free_stopped(control);
    g_assert_cmpuint(released->len, ==, 0);
    g_free(control);
}

/* ---- Contract 2: ending a session keeps the binding with its owner. ---- */
static GVariant* lua_call(GnoblinNativeControl* control, const char* method, GVariant* arguments,
                          GError** error) {
    const GnoblinApiOperation* operation = gnoblin_api_operation_find(method);
    g_assert_nonnull(operation);
    return native_runtime_shortcut_operation(control, operation, arguments, 31, error);
}

static GVariant* lua_bind(GnoblinNativeControl* control, const char* id, const char* chord) {
    g_autoptr(GError) error = NULL;
    GVariant* result =
        lua_call(control, "shortcut.bind",
                 g_variant_new_parsed("@a{sv} {'id': <%s>, 'accelerator': <%s>, 'hold': <'alt'>}",
                                      id, chord),
                 &error);
    g_assert_no_error(error);
    g_assert_nonnull(result);
    return result;
}

static GVariant* lua_end_session(GnoblinNativeControl* control, const char* id, guint64 session_id,
                                 GError** error) {
    return lua_call(
        control, "shortcut.session.end",
        g_variant_new_parsed("@a{sv} {'id': <%s>, 'session_id': <%x>}", id, (gint64)session_id),
        error);
}

static void test_lua_binding_survives_session_end_and_activates_again(void) {
    reset();
    GnoblinNativeControl* control = new_control();
    g_autoptr(GVariant) bound = lua_bind(control, "hud", "<Control>F9");
    g_assert_cmpuint(grabs, ==, 1);
    NativeDynamicShortcut* shortcut =
        gnoblin_control_find_runtime_dynamic_shortcut(control, "lua:5", "hud");
    g_assert_nonnull(shortcut);
    guint action = shortcut->action;

    dispatch_dynamic_shortcut_activated(control, action, &key_event);
    g_assert_true(shortcut->active);
    g_assert_true(control->active_shortcut_session == shortcut);
    guint64 first = shortcut->session_id;
    g_assert_cmpuint(first, >, 0);
    g_autofree char* activated =
        g_strdup_printf("gnoblin.shortcut.binding-activated hud %" G_GUINT64_FORMAT " ", first);
    expect_event(activated);

    g_autoptr(GError) error = NULL;
    g_autoptr(GVariant) ended = lua_end_session(control, "hud", first, &error);
    g_assert_no_error(error);
    g_assert_nonnull(ended);
    g_autofree char* ended_event = g_strdup_printf(
        "gnoblin.shortcut.session.ended hud %" G_GUINT64_FORMAT " cancelled", first);
    expect_event(ended_event);

    /* The session is over... */
    g_assert_false(shortcut->active);
    g_assert_null(control->active_shortcut_session);
    /* ...and nothing else: same binding, same owner, still armed, accelerator still held. */
    g_assert_true(gnoblin_control_find_runtime_dynamic_shortcut(control, "lua:5", "hud") ==
                  shortcut);
    g_assert_true(g_hash_table_lookup(control->dynamic_shortcuts, GUINT_TO_POINTER(action)) ==
                  shortcut);
    g_assert_cmpstr(shortcut->owner_id, ==, "lua:5");
    g_assert_null(shortcut->client);
    g_assert_true(shortcut->armed);
    g_assert_cmpuint(ungrabs, ==, 0);
    g_assert_cmpuint(shortcut->timeout_id, ==, 0);

    /* Ending the same session again is refused, and still does not touch the binding. */
    g_clear_pointer(&error, g_error_free);
    g_assert_null(lua_end_session(control, "hud", first, &error));
    g_assert_error(error, G_IO_ERROR, G_IO_ERROR_NOT_FOUND);
    g_assert_true(gnoblin_control_find_runtime_dynamic_shortcut(control, "lua:5", "hud") ==
                  shortcut);

    /* The binding activates for its owner again, in a new session. */
    guint before = count_events("gnoblin.shortcut.binding-activated hud");
    dispatch_dynamic_shortcut_activated(control, action, &key_event);
    g_assert_cmpuint(count_events("gnoblin.shortcut.binding-activated hud"), ==, before + 1);
    g_assert_true(shortcut->active);
    g_assert_cmpuint(shortcut->session_id, >, first);

    /* A stale session ID does not end the new session. */
    g_clear_pointer(&error, g_error_free);
    g_assert_null(lua_end_session(control, "hud", first, &error));
    g_assert_error(error, G_IO_ERROR, G_IO_ERROR_NOT_FOUND);
    g_assert_true(shortcut->active);

    free_control(control);
}

static void test_session_end_reaches_only_the_binding_of_its_own_generation(void) {
    reset();
    GnoblinNativeControl* control = new_control();
    g_autoptr(GVariant) bound = lua_bind(control, "hud", "<Control>F9");
    NativeDynamicShortcut* old_shortcut =
        gnoblin_control_find_runtime_dynamic_shortcut(control, "lua:5", "hud");
    dispatch_dynamic_shortcut_activated(control, old_shortcut->action, &key_event);
    guint64 session = old_shortcut->session_id;

    /* A reload: the runtime of generation 6 binds its own "hud" and cannot end generation 5's. */
    config_generation = 6;
    g_autoptr(GVariant) rebound = lua_bind(control, "hud", "<Control>F10");
    NativeDynamicShortcut* new_shortcut =
        gnoblin_control_find_runtime_dynamic_shortcut(control, "lua:6", "hud");
    g_assert_nonnull(new_shortcut);
    g_assert_true(new_shortcut != old_shortcut);
    g_autoptr(GError) error = NULL;
    g_assert_null(lua_end_session(control, "hud", session, &error));
    g_assert_error(error, G_IO_ERROR, G_IO_ERROR_NOT_FOUND);
    g_assert_true(old_shortcut->active);
    g_assert_true(control->active_shortcut_session == old_shortcut);
    g_assert_true(gnoblin_control_find_runtime_dynamic_shortcut(control, "lua:5", "hud") ==
                  old_shortcut);
    free_control(control);
}

static void test_runtime_abort_ends_the_session_and_releases_only_lua_bindings(void) {
    reset();
    GnoblinNativeControl* control = new_control();
    g_autoptr(GVariant) bound = lua_bind(control, "hud", "<Control>F9");
    NativeDynamicShortcut* shortcut =
        gnoblin_control_find_runtime_dynamic_shortcut(control, "lua:5", "hud");
    dispatch_dynamic_shortcut_activated(control, shortcut->action, &key_event);
    guint64 session = shortcut->session_id;
    g_autofree char* stopped = g_strdup_printf(
        "gnoblin.shortcut.session.ended hud %" G_GUINT64_FORMAT " runtime_stopped", session);

    native_runtime_abort(control);
    expect_event(stopped);
    g_assert_null(control->active_shortcut_session);
    g_assert_null(gnoblin_control_find_runtime_dynamic_shortcut(control, "lua:5", "hud"));
    g_assert_cmpuint(ungrabs, ==, 1);
    free_control(control);
}

static Client* new_client(GnoblinNativeControl* control, guint64 client_id) {
    Client* client = g_new0(Client, 1);
    client->control = control;
    client->client_id = client_id;
    client->api_minor = 100;
    client->event_api_minor = 100;
    client->event_subscriptions = g_hash_table_new_full(g_str_hash, g_str_equal, g_free, NULL);
    g_hash_table_add(client->event_subscriptions, g_strdup("gnoblin.shortcut.binding-activated"));
    g_hash_table_add(client->event_subscriptions, g_strdup("gnoblin.shortcut.session.activated"));
    g_hash_table_add(client->event_subscriptions, g_strdup("gnoblin.shortcut.session.ended"));
    client->focus_grants = g_hash_table_new_full(g_str_hash, g_str_equal, g_free, g_free);
    return client;
}

static void free_client(Client* client) {
    g_clear_pointer(&client->event_subscriptions, g_hash_table_unref);
    g_clear_pointer(&client->focus_grants, g_hash_table_unref);
    g_free(client);
}

static JsonObject* parse_line(const char* line) {
    g_autoptr(JsonParser) parser = json_parser_new();
    g_assert_true(json_parser_load_from_data(parser, line, -1, NULL));
    return json_object_ref(json_node_get_object(json_parser_get_root(parser)));
}

/* The newest line sent to a socket client with this "event" member. */
static JsonObject* last_line(const char* event) {
    for (guint i = socket_lines->len; i > 0; i--) {
        g_autoptr(JsonObject) object = parse_line(g_ptr_array_index(socket_lines, i - 1));
        if (g_str_equal(json_object_get_string_member(object, "event"), event))
            return g_steal_pointer(&object);
    }
    g_error("no \"%s\" line was written to the client", event);
}

static char* socket_call(char* (*handler)(Client*, const char*, JsonObject*), Client* client,
                         JsonObject* arguments) {
    char* reply = handler(client, "1", arguments);
    json_object_unref(arguments);
    return reply;
}

static JsonObject* end_arguments(const char* id, gint64 session_id) {
    JsonObject* arguments = json_object_new();
    json_object_set_string_member(arguments, "id", id);
    json_object_set_int_member(arguments, "session_id", session_id);
    return arguments;
}

static void test_socket_binding_survives_session_end_and_activates_again(void) {
    reset();
    GnoblinNativeControl* control = new_control();
    Client* owner = new_client(control, 7);
    Client* other = new_client(control, 8);

    JsonObject* bind_arguments = json_object_new();
    json_object_set_string_member(bind_arguments, "id", "hud");
    json_object_set_string_member(bind_arguments, "accelerator", "<Control>F9");
    json_object_set_string_member(bind_arguments, "hold", "control");
    g_autofree char* bind_reply =
        socket_call(gnoblin_control_dynamic_shortcut_bind, owner, bind_arguments);
    g_autoptr(JsonObject) bound = parse_line(bind_reply);
    g_assert_cmpstr(json_object_get_string_member(bound, "event"), ==, "reply");
    NativeDynamicShortcut* shortcut = gnoblin_control_find_dynamic_shortcut(owner, "hud");
    g_assert_nonnull(shortcut);
    guint action = shortcut->action;

    dispatch_dynamic_shortcut_activated(control, action, &key_event);
    g_assert_true(shortcut->active);
    guint64 first = shortcut->session_id;
    g_autoptr(JsonObject) activated = last_line("gnoblin.shortcut.binding-activated");
    g_assert_cmpstr(json_object_get_string_member(activated, "id"), ==, "hud");
    g_assert_cmpint(json_object_get_int_member(activated, "session_id"), ==, (gint64)first);

    /* Another connection cannot end the session: the binding is not its own. */
    g_autofree char* foreign = socket_call(gnoblin_control_dynamic_shortcut_request_end_session,
                                           other, end_arguments("hud", (gint64)first));
    g_autoptr(JsonObject) refused = parse_line(foreign);
    g_assert_cmpstr(json_object_get_string_member(refused, "event"), ==, "error");
    g_assert_true(shortcut->active);

    g_autofree char* reply = socket_call(gnoblin_control_dynamic_shortcut_request_end_session,
                                         owner, end_arguments("hud", (gint64)first));
    g_autoptr(JsonObject) ended = parse_line(reply);
    g_assert_cmpstr(json_object_get_string_member(ended, "event"), ==, "reply");
    g_assert_true(
        json_object_get_boolean_member(json_object_get_object_member(ended, "result"), "ended"));
    g_autoptr(JsonObject) ended_event = last_line("gnoblin.shortcut.session.ended");
    g_assert_cmpstr(json_object_get_string_member(ended_event, "reason"), ==, "cancelled");
    g_assert_cmpint(json_object_get_int_member(ended_event, "session_id"), ==, (gint64)first);

    g_assert_false(shortcut->active);
    g_assert_null(control->active_shortcut_session);
    g_assert_true(gnoblin_control_find_dynamic_shortcut(owner, "hud") == shortcut);
    g_assert_true(shortcut->client == owner);
    g_assert_cmpuint(shortcut->client_id, ==, 7);
    g_assert_cmpstr(shortcut->owner_id, ==, "socket:7");
    g_assert_true(shortcut->armed);
    g_assert_true(g_hash_table_lookup(control->dynamic_shortcuts, GUINT_TO_POINTER(action)) ==
                  shortcut);
    g_assert_cmpuint(ungrabs, ==, 0);
    g_assert_null(gnoblin_control_find_dynamic_shortcut(other, "hud"));

    /* Ending it again is refused. */
    g_autofree char* again = socket_call(gnoblin_control_dynamic_shortcut_request_end_session,
                                         owner, end_arguments("hud", (gint64)first));
    g_autoptr(JsonObject) again_object = parse_line(again);
    g_assert_cmpstr(json_object_get_string_member(again_object, "event"), ==, "error");

    /* The binding still activates for its owner, with a new session. */
    guint lines = socket_lines->len;
    dispatch_dynamic_shortcut_activated(control, action, &key_event);
    g_assert_cmpuint(socket_lines->len, >, lines);
    g_autoptr(JsonObject) second = last_line("gnoblin.shortcut.binding-activated");
    g_assert_cmpstr(json_object_get_string_member(second, "id"), ==, "hud");
    g_assert_true(json_object_get_boolean_member(second, "first"));
    g_assert_cmpint(json_object_get_int_member(second, "session_id"), >, (gint64)first);
    g_assert_true(shortcut->active);

    g_clear_pointer(&control->dynamic_shortcuts, g_hash_table_unref);
    free_client(owner);
    free_client(other);
    g_clear_pointer(&control->focus_contexts, g_hash_table_unref);
    g_free(control);
}

/* ---- Contract 3: a grab is refused while a key is held, and the set of held keys does not keep
 * a key that can no longer lift. The production entry point is gnoblin_control_input_grab. ---- */
static int display_token;
static int keyboard_a_token;
static int keyboard_b_token;
#define DISPLAY ((MetaDisplay*)&display_token)
#define KEYBOARD_A ((ClutterInputDevice*)&keyboard_a_token)
#define KEYBOARD_B ((ClutterInputDevice*)&keyboard_b_token)
#define EVDEV_A 30
#define EVDEV_B 48

static GnoblinNativeControl* new_grab_control(void) {
    GnoblinNativeControl* control = g_new0(GnoblinNativeControl, 1);
    control->display = DISPLAY;
    control->runtime_generation = 7;
    control->pressed_keys = gnoblin_control_pressed_keys_new();
    /* A table of the test's own: the grab then does not look for the compose file of the locale. */
    struct xkb_context* context = xkb_context_new(XKB_CONTEXT_NO_FLAGS);
    static const char compose[] = "<Multi_key> <a> <b> : \"x\"\n";
    control->compose_table =
        xkb_compose_table_new_from_buffer(context, compose, sizeof compose - 1, "C",
                                          XKB_COMPOSE_FORMAT_TEXT_V1, XKB_COMPOSE_COMPILE_NO_FLAGS);
    xkb_context_unref(context);
    g_assert_nonnull(control->compose_table);
    return control;
}

static void free_grab_control(GnoblinNativeControl* control) {
    gnoblin_control_input_grab_end(control, "runtime_stopped", FALSE);
    g_clear_pointer((struct xkb_compose_table**)&control->compose_table, xkb_compose_table_unref);
    g_clear_pointer(&control->pressed_keys, g_hash_table_unref);
    g_free(control);
}

static void feed_key(GnoblinNativeControl* control, ClutterEventType type,
                     ClutterInputDevice* device, guint32 code, xkb_keysym_t keysym,
                     ClutterEventFlags flags) {
    ClutterEvent event = {
        .type = type, .flags = flags, .source = device, .code = code, .keysym = keysym};
    gnoblin_control_track_pressed_key(control, &event);
}

static void press_key(GnoblinNativeControl* control, ClutterInputDevice* device, guint32 code,
                      xkb_keysym_t keysym) {
    feed_key(control, CLUTTER_KEY_PRESS, device, code, keysym, CLUTTER_EVENT_NONE);
}

static void release_key(GnoblinNativeControl* control, ClutterInputDevice* device, guint32 code,
                        xkb_keysym_t keysym) {
    feed_key(control, CLUTTER_KEY_RELEASE, device, code, keysym, CLUTTER_EVENT_NONE);
}

static GVariant* start_grab(GnoblinNativeControl* control, GError** error) {
    g_autoptr(GVariant) arguments =
        g_variant_ref_sink(g_variant_new_array(G_VARIANT_TYPE("{sv}"), NULL, 0));
    return gnoblin_control_input_grab(control, 1, arguments, 7, error);
}

/* The grab is refused with the message of 1b5934d9, which names a held key. With several keys
 * down the hash table picks which one, so `alternative` (when given) is an accepted name too. */
static void assert_grab_refused_naming_one_of(GnoblinNativeControl* control, const char* key,
                                              const char* alternative) {
    g_autoptr(GError) error = NULL;
    g_autoptr(GVariant) result = start_grab(control, &error);
    g_assert_null(result);
    g_assert_error(error, G_IO_ERROR, G_IO_ERROR_BUSY);
    g_autofree char* named = g_strdup_printf("while a key is held (%s); its release would", key);
    g_autofree char* named_alternative =
        alternative ? g_strdup_printf("while a key is held (%s); its release would", alternative)
                    : NULL;
    g_assert_true(strstr(error->message, named) ||
                  (named_alternative && strstr(error->message, named_alternative)));
    g_assert_null(control->input_grab);
}

static void assert_grab_refused_naming(GnoblinNativeControl* control, const char* key) {
    assert_grab_refused_naming_one_of(control, key, NULL);
}

static void assert_grab_granted(GnoblinNativeControl* control) {
    g_autoptr(GError) error = NULL;
    g_autoptr(GVariant) result = start_grab(control, &error);
    g_assert_no_error(error);
    g_assert_nonnull(result);
    g_assert_nonnull(control->input_grab);
    gnoblin_control_input_grab_end(control, "released", FALSE);
    g_assert_null(control->input_grab);
}

/* The rule of 1b5934d9 still holds: a key that is down refuses the grab, and its release ends the
 * refusal. */
static void test_grab_is_refused_while_a_key_is_held_and_granted_once_it_lifts(void) {
    GnoblinNativeControl* control = new_grab_control();
    assert_grab_granted(control);

    press_key(control, KEYBOARD_A, EVDEV_A, XKB_KEY_a);
    assert_grab_refused_naming(control, "a");
    assert_grab_refused_naming(control, "a");
    release_key(control, KEYBOARD_A, EVDEV_A, XKB_KEY_a);
    assert_grab_granted(control);

    /* A keysym without a name of its own is named by its number. */
    press_key(control, KEYBOARD_A, EVDEV_B, 0x10203040);
    assert_grab_refused_naming(control, "0x10203040");
    release_key(control, KEYBOARD_A, EVDEV_B, 0x10203040);
    assert_grab_granted(control);
    free_grab_control(control);
}

/* A release that never comes (the keyboard is unplugged, or a virtual device is removed with its
 * key down) must not keep the grab refused. The seat tells that the keyboard is gone. */
static void test_a_removed_keyboard_drops_the_keys_it_held(void) {
    GnoblinNativeControl* control = new_grab_control();
    press_key(control, KEYBOARD_A, EVDEV_A, XKB_KEY_a);
    press_key(control, KEYBOARD_A, EVDEV_B, XKB_KEY_b);
    assert_grab_refused_naming_one_of(control, "a", "b");

    gnoblin_control_forget_pressed_keys(control, KEYBOARD_A);
    assert_grab_granted(control);

    /* Forgetting a keyboard that holds nothing, or that is already gone, changes nothing. */
    gnoblin_control_forget_pressed_keys(control, KEYBOARD_A);
    assert_grab_granted(control);
    free_grab_control(control);
}

static void test_removing_one_keyboard_keeps_the_keys_of_another(void) {
    GnoblinNativeControl* control = new_grab_control();
    press_key(control, KEYBOARD_A, EVDEV_A, XKB_KEY_a);
    press_key(control, KEYBOARD_B, EVDEV_B, XKB_KEY_b);

    gnoblin_control_forget_pressed_keys(control, KEYBOARD_A);
    assert_grab_refused_naming(control, "b");
    release_key(control, KEYBOARD_B, EVDEV_B, XKB_KEY_b);
    assert_grab_granted(control);
    free_grab_control(control);
}

static void test_the_same_key_on_two_keyboards_is_two_held_keys(void) {
    GnoblinNativeControl* control = new_grab_control();
    press_key(control, KEYBOARD_A, EVDEV_A, XKB_KEY_a);
    press_key(control, KEYBOARD_B, EVDEV_A, XKB_KEY_a);

    release_key(control, KEYBOARD_A, EVDEV_A, XKB_KEY_a);
    assert_grab_refused_naming(control, "a");
    release_key(control, KEYBOARD_B, EVDEV_A, XKB_KEY_a);
    assert_grab_granted(control);

    /* The second keyboard goes away with the key down: only the first lifts it. */
    press_key(control, KEYBOARD_A, EVDEV_A, XKB_KEY_a);
    press_key(control, KEYBOARD_B, EVDEV_A, XKB_KEY_a);
    gnoblin_control_forget_pressed_keys(control, KEYBOARD_B);
    assert_grab_refused_naming(control, "a");
    release_key(control, KEYBOARD_A, EVDEV_A, XKB_KEY_a);
    assert_grab_granted(control);
    free_grab_control(control);
}

static void test_repeats_count_once_and_synthetic_or_unseen_events_hold_nothing(void) {
    GnoblinNativeControl* control = new_grab_control();

    /* Autorepeat sends more presses of a key that is already down; one release lifts it. */
    press_key(control, KEYBOARD_A, EVDEV_A, XKB_KEY_a);
    feed_key(control, CLUTTER_KEY_PRESS, KEYBOARD_A, EVDEV_A, XKB_KEY_a,
             CLUTTER_EVENT_FLAG_REPEATED);
    feed_key(control, CLUTTER_KEY_PRESS, KEYBOARD_A, EVDEV_A, XKB_KEY_a,
             CLUTTER_EVENT_FLAG_REPEATED);
    assert_grab_refused_naming(control, "a");
    release_key(control, KEYBOARD_A, EVDEV_A, XKB_KEY_a);
    assert_grab_granted(control);

    /* A key that is not a key of a keyboard (an input method, a synthetic press) is not held. */
    feed_key(control, CLUTTER_KEY_PRESS, KEYBOARD_A, EVDEV_A, XKB_KEY_a,
             CLUTTER_EVENT_FLAG_SYNTHETIC);
    feed_key(control, CLUTTER_KEY_PRESS, KEYBOARD_A, EVDEV_B, XKB_KEY_b,
             CLUTTER_EVENT_FLAG_INPUT_METHOD);
    assert_grab_granted(control);

    /* The release of a key whose press was never seen, and an event that is not a key, hold
     * nothing either. */
    release_key(control, KEYBOARD_A, EVDEV_A, XKB_KEY_a);
    feed_key(control, CLUTTER_MOTION, KEYBOARD_A, EVDEV_A, XKB_KEY_a, CLUTTER_EVENT_NONE);
    assert_grab_granted(control);
    free_grab_control(control);
}

int main(int argc, char** argv) {
    g_test_init(&argc, &argv, NULL);
    released = g_ptr_array_new();
    runtime_events = g_ptr_array_new_with_free_func(g_free);
    socket_lines = g_ptr_array_new_with_free_func(g_free);

    g_test_add_func("/teardown/abort-alone-never-frees", test_abort_alone_never_frees);
    g_test_add_func("/teardown/callback-after-abort-waits-for-teardown",
                    test_async_callback_after_abort_does_not_free_before_teardown);
    g_test_add_func("/teardown/teardown-waits-for-last-callback",
                    test_teardown_waits_for_the_last_async_callback);
    g_test_add_func("/teardown/every-kind-of-work-holds-the-control",
                    test_every_kind_of_work_in_flight_holds_the_control);
    g_test_add_func("/teardown/running-control-is-kept",
                    test_running_control_is_never_freed_by_teardown_bookkeeping);
    g_test_add_func("/shortcut/lua-binding-survives-session-end",
                    test_lua_binding_survives_session_end_and_activates_again);
    g_test_add_func("/shortcut/session-end-is-scoped-to-its-generation",
                    test_session_end_reaches_only_the_binding_of_its_own_generation);
    g_test_add_func("/shortcut/runtime-abort-ends-session-and-lua-bindings",
                    test_runtime_abort_ends_the_session_and_releases_only_lua_bindings);
    g_test_add_func("/shortcut/socket-binding-survives-session-end",
                    test_socket_binding_survives_session_end_and_activates_again);
    g_test_add_func("/held-keys/grab-is-refused-while-a-key-is-held",
                    test_grab_is_refused_while_a_key_is_held_and_granted_once_it_lifts);
    g_test_add_func("/held-keys/removed-keyboard-drops-its-keys",
                    test_a_removed_keyboard_drops_the_keys_it_held);
    g_test_add_func("/held-keys/other-keyboards-keep-their-keys",
                    test_removing_one_keyboard_keeps_the_keys_of_another);
    g_test_add_func("/held-keys/same-key-on-two-keyboards",
                    test_the_same_key_on_two_keyboards_is_two_held_keys);
    g_test_add_func("/held-keys/repeat-synthetic-and-unseen",
                    test_repeats_count_once_and_synthetic_or_unseen_events_hold_nothing);
    return g_test_run();
}
