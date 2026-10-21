/* Built by native-window-publish.py using verbatim production functions.
 * Failure modes: legacy windows subscribers lost; API 1.0 receiving events;
 * explicit subscriptions/version gates bypassed; double delivery/encoding;
 * altered shared JSON mutations; Lua dispatch suppressed; snapshots lost.
 * These boundary stubs intentionally do not model a desktop or Lua VM. */
#include <gio/gio.h>
#include <json-glib/json-glib.h>

typedef struct {
    gboolean track_windows, track_monitors, track_input_sources, track_shortcut_capture;
    gboolean track_input_devices, track_launches;
    guint api_minor, event_api_minor, windows_api_minor, monitors_api_minor;
    guint input_sources_api_minor, shortcut_capture_api_minor, input_devices_api_minor;
    guint launch_api_minor;
    guint64 client_id;
    GHashTable* event_subscriptions;
    GPtrArray* messages;
} Client;

typedef struct {
    GHashTable* recipient_client_ids;
} PendingLocationAuthorization;

typedef struct {
    GHashTable *clients, *window_state, *pending_location_authorizations;
    gpointer pending_auth, pending_prompts, display;
    guint64 state_revision, event_sequence;
    guint publish_id;
    gboolean window_state_initialized;
} GnoblinNativeControl;

typedef struct {
    const char *native_name, *lua_name;
} WindowPropertyName;

typedef struct {
    char *json, *comparable_json;
    gboolean focused;
} NativeWindowState;

/* Only modal classification needs this enum value; no Mutter ABI is exercised. */
#define META_WINDOW_MODAL_DIALOG 4

static guint socket_encodes, socket_copies, lua_events, lua_snapshots;
static guint auth_recipients, prompt_recipients;
static guint64 encoded_bytes, delivered_bytes;
static GVariant* last_lua_payload;
static char* last_lua_event;
static const char* snapshot_fixture = "{\"windows\":[]}";
static gboolean retain_messages = TRUE;

static char* counted_json_to_string(JsonNode* node, gboolean pretty) {
    char* text = json_to_string(node, pretty);
    socket_encodes++;
    encoded_bytes += strlen(text);
    return text;
}

static JsonNode* counted_json_node_copy(JsonNode* node) {
    socket_copies++;
    return json_node_copy(node);
}

static void send_response(Client* client, char* response) {
    delivered_bytes += strlen(response);
    if (retain_messages)
        g_ptr_array_add(client->messages, response);
    else
        g_free(response);
}

static gboolean native_runtime_dispatch_event(GnoblinNativeControl* control, const char* event,
                                              GVariant* payload) {
    lua_events++;
    g_clear_pointer(&last_lua_payload, g_variant_unref);
    g_free(last_lua_event);
    last_lua_payload = g_variant_ref(payload);
    last_lua_event = g_strdup(event);
    return TRUE;
}

static void native_publish_runtime_snapshot(GnoblinNativeControl* control, const char* name,
                                            GVariant* snapshot, guint64 revision) {
    if (g_str_equal(name, "windows")) {
        g_assert_nonnull(snapshot);
        lua_snapshots++;
    }
}

static void gnoblin_control_auth_note_recipient(GnoblinNativeControl* control, gint64 request,
                                                guint64 client) {
    auth_recipients++;
}

static void gnoblin_control_prompts_note_recipient(GnoblinNativeControl* control, gint64 request,
                                                   guint64 client) {
    prompt_recipients++;
}

GVariant* variant_from_json(JsonNode* node);
void publish_window_changes(GnoblinNativeControl* control, JsonNode* snapshot, guint64 revision);

static GVariant* meta_gnoblin_dispatch_native_api(gpointer display, const char* method,
                                                  GVariant* arguments, GError** error) {
    g_assert_cmpstr(method, ==, "window.list");
    g_autoptr(JsonNode) root = json_from_string(snapshot_fixture, NULL);
    return variant_from_json(root);
}

static char* encode_response(const char* id, JsonNode* result, const char* error) {
    g_error("Unexpected snapshot error: %s", error);
    return NULL;
}

static JsonNode* gnoblin_control_workspace_snapshot_json(GnoblinNativeControl* control,
                                                         GError** error) {
    return json_from_string("{}", NULL);
}

static JsonNode* gnoblin_control_monitor_snapshot_json(GnoblinNativeControl* control,
                                                       gboolean update, GError** error) {
    return json_from_string("{}", NULL);
}

static JsonNode* gnoblin_control_layer_snapshot_json(GnoblinNativeControl* control, gboolean update,
                                                     GError** error) {
    return json_from_string("{}", NULL);
}

static GVariant* capability_snapshot(GnoblinNativeControl* control) {
    return g_variant_ref_sink(g_variant_new_array(G_VARIANT_TYPE("{sv}"), NULL, 0));
}

static GVariant* input_device_snapshot(GnoblinNativeControl* control) {
    return capability_snapshot(control);
}

static void gnoblin_control_publish_input_source_changes(GnoblinNativeControl* control,
                                                         guint64 revision) {}
static void gnoblin_control_publish_input_device_changes(GnoblinNativeControl* control,
                                                         JsonNode* snapshot, guint64 revision) {}
static void gnoblin_control_publish_workspace_changes(GnoblinNativeControl* control,
                                                      JsonNode* workspace, JsonNode* windows,
                                                      guint64 revision) {}
static gboolean gnoblin_control_publish_monitor_changes(GnoblinNativeControl* control,
                                                        JsonNode* snapshot, guint64 revision) {
    return FALSE;
}
static void publish_layer_changes(GnoblinNativeControl* control, JsonNode* snapshot,
                                  guint64 revision) {}

#include "production.inc"

static GnoblinNativeControl control_new(void) {
    socket_encodes = socket_copies = lua_events = lua_snapshots = 0;
    encoded_bytes = delivered_bytes = 0;
    auth_recipients = prompt_recipients = 0;
    return (GnoblinNativeControl){
        .clients = g_hash_table_new(g_direct_hash, g_direct_equal),
        .window_state =
            g_hash_table_new_full(g_str_hash, g_str_equal, g_free, native_window_state_free),
        .state_revision = 42,
    };
}

static void control_clear(GnoblinNativeControl* control) {
    g_hash_table_unref(control->clients);
    g_hash_table_unref(control->window_state);
    g_clear_pointer(&last_lua_payload, g_variant_unref);
    g_clear_pointer(&last_lua_event, g_free);
}

static Client client_new(GnoblinNativeControl* control, gboolean windows, guint minor) {
    return (Client){.track_windows = windows,
                    .windows_api_minor = minor,
                    .api_minor = minor,
                    .messages = g_ptr_array_new_with_free_func(g_free),
                    .event_subscriptions = g_hash_table_new(g_str_hash, g_str_equal)};
}

static void client_clear(Client* client) {
    g_ptr_array_unref(client->messages);
    g_hash_table_unref(client->event_subscriptions);
}

static const char* lifecycle_events[] = {
    "gnoblin.window.created",   "gnoblin.window.changed",           "gnoblin.window.focused",
    "gnoblin.window.unfocused", "gnoblin.window.attention-changed", "gnoblin.window.closed",
};

static void dispatch(GnoblinNativeControl* control, const char* event) {
    g_autoptr(JsonNode) record = json_from_string(
        "{\"id\":\"w1\",\"title\":\"sample\",\"appId\":\"org.example.Test\","
        "\"workspaceId\":\"ws1\",\"demandsAttention\":true,\"geometry\":{\"x\":12,\"width\":800}}",
        NULL);
    g_autoptr(JsonArray) changed = json_array_new();
    json_array_add_string_element(changed, "app_id");
    json_array_add_string_element(changed, "frame");
    dispatch_lua_window_event(control, 42, event, "w1", record, record, changed);
}

static JsonNode* message(Client* client, guint index) {
    const char* text = g_ptr_array_index(client->messages, index);
    g_assert_true(g_str_has_suffix(text, "\n"));
    g_assert_true(strchr(text, '\n') == text + strlen(text) - 1);
    return json_from_string(text, NULL);
}

static void assert_lua(const char* event, guint sequence) {
    g_assert_cmpstr(last_lua_event, ==, event);
    g_autoptr(JsonNode) lua = json_from_variant(last_lua_payload);
    JsonObject* object = json_node_get_object(lua);
    /* Preserve the existing shared-object mutation, even without recipients. */
    g_assert_false(json_object_has_member(object, "name"));
    g_assert_cmpstr(json_object_get_string_member(object, "event"), ==, event);
    g_assert_cmpint(json_object_get_int_member(object, "revision"), ==, 42);
    g_assert_cmpint(json_object_get_int_member(object, "sequence"), ==, sequence);
    g_assert_cmpint(json_object_get_int_member(object, "time"), >, 0);
    if (g_str_equal(event, "gnoblin.window.changed")) {
        JsonArray* changed = json_object_get_array_member(object, "changed");
        g_assert_cmpuint(json_array_get_length(changed), ==, 4);
        g_assert_cmpstr(json_array_get_string_element(changed, 0), ==, "app_id");
        g_assert_cmpstr(json_array_get_string_element(changed, 1), ==, "appId");
        g_assert_cmpstr(json_array_get_string_element(changed, 2), ==, "frame");
        g_assert_cmpstr(json_array_get_string_element(changed, 3), ==, "geometry");
    }
}

static void test_legacy_windows(void) {
    for (guint minor = 0; minor <= 9; minor++) {
        GnoblinNativeControl control = control_new();
        Client client = client_new(&control, TRUE, minor);
        g_hash_table_add(control.clients, &client);
        for (guint i = 0; i < G_N_ELEMENTS(lifecycle_events); i++) {
            dispatch(&control, lifecycle_events[i]);
            assert_lua(lifecycle_events[i], i + 1);
            if (minor > 0) {
                g_autoptr(JsonNode) received = message(&client, i);
                g_autoptr(JsonNode) lua = json_from_variant(last_lua_payload);
                g_assert_true(json_node_equal(received, lua));
            }
        }
        g_assert_cmpuint(client.messages->len, ==, minor ? 6 : 0);
        g_assert_cmpuint(lua_events, ==, 6);
        client_clear(&client);
        control_clear(&control);
    }
}

static void test_explicit_and_fanout(void) {
    GnoblinNativeControl control = control_new();
    Client legacy = client_new(&control, TRUE, 1);
    Client explicit = client_new(&control, FALSE, 9);
    Client old = client_new(&control, FALSE, 8);
    Client unrelated = client_new(&control, FALSE, 80);
    explicit.event_api_minor = 9;
    old.event_api_minor = 8;
    unrelated.event_api_minor = 80;
    g_hash_table_add(explicit.event_subscriptions, "gnoblin.window.changed");
    g_hash_table_add(old.event_subscriptions, "gnoblin.window.changed");
    g_hash_table_add(unrelated.event_subscriptions, "gnoblin.window.closed");
    Client* clients[] = {&legacy, &explicit, &old, &unrelated};
    for (guint i = 0; i < G_N_ELEMENTS(clients); i++)
        g_hash_table_add(control.clients, clients[i]);
    dispatch(&control, "gnoblin.window.changed");
    g_assert_cmpuint(socket_encodes, ==, 1);
    g_assert_cmpuint(legacy.messages->len, ==, 1);
    g_assert_cmpuint(explicit.messages->len, ==, 1);
    g_assert_cmpuint(old.messages->len, ==, 0);
    g_assert_cmpuint(unrelated.messages->len, ==, 0);
    g_assert_cmpstr(g_ptr_array_index(legacy.messages, 0), ==,
                    g_ptr_array_index(explicit.messages, 0));
    /* Explicit mode must work by itself, and both modes must not double-send. */
    g_hash_table_remove(control.clients, &legacy);
    dispatch(&control, "gnoblin.window.changed");
    explicit.track_windows = TRUE;
    explicit.windows_api_minor = 1;
    dispatch(&control, "gnoblin.window.changed");
    g_assert_cmpuint(explicit.messages->len, ==, 3);
    g_assert_cmpuint(socket_encodes, ==, 3);
    for (guint i = 0; i < G_N_ELEMENTS(clients); i++)
        client_clear(clients[i]);
    control_clear(&control);
}

static void test_mutations(void) {
    const char* input =
        "{\"name\":\"gnoblin.window.changed\",\"revision\":42,\"sequence\":7,\"time\":123,"
        "\"focus_context\":1,\"window\":{\"appId\":\"new\",\"app_id\":\"keep\","
        "\"geometry\":{\"x\":12},\"focus_context\":2},\"last\":{\"appId\":\"old\"},"
        "\"nested\":[{\"focus_context\":3,\"keep\":true}],\"changed\":[\"app_id\",\"title\"]}";
    const char* expected =
        "{\"event\":\"gnoblin.window.changed\",\"revision\":42,\"sequence\":7,\"time\":123,"
        "\"window\":{\"appId\":\"new\",\"app_id\":\"keep\",\"geometry\":{\"x\":12},"
        "\"frame\":{\"x\":12}},\"last\":{\"appId\":\"old\",\"app_id\":\"old\"},"
        "\"nested\":[{\"keep\":true}],\"changed\":[\"app_id\",\"appId\",\"title\"]}";
    for (guint subscribed = 0; subscribed <= 1; subscribed++) {
        GnoblinNativeControl control = control_new();
        Client client = client_new(&control, TRUE, 1);
        if (subscribed)
            g_hash_table_add(control.clients, &client);
        g_autoptr(JsonNode) root = json_from_string(input, NULL);
        g_autoptr(JsonNode) wanted = json_from_string(expected, NULL);
        publish_native_socket_event(&control, root);
        g_assert_true(json_node_equal(root, wanted));
        if (subscribed) {
            g_autoptr(JsonNode) received = message(&client, 0);
            g_assert_true(json_node_equal(received, wanted));
            g_assert_cmpstr(
                g_ptr_array_index(client.messages, 0), ==,
                "{\"revision\":42,\"sequence\":7,\"time\":123,"
                "\"window\":{\"appId\":\"new\",\"app_id\":\"keep\",\"geometry\":{\"x\":12},"
                "\"frame\":{\"x\":12}},\"last\":{\"appId\":\"old\",\"app_id\":\"old\"},"
                "\"nested\":[{\"keep\":true}],\"changed\":[\"app_id\",\"appId\",\"title\"],"
                "\"event\":\"gnoblin.window.changed\"}\n");
        }
        client_clear(&client);
        control_clear(&control);
    }
}

static void test_snapshots(void) {
    GnoblinNativeControl control = control_new();
    Client old = client_new(&control, TRUE, 0);
    Client legacy = client_new(&control, TRUE, 1);
    g_hash_table_add(control.clients, &old);
    g_hash_table_add(control.clients, &legacy);
    snapshot_fixture = "{\"windows\":[{\"id\":\"w1\",\"title\":\"before\",\"appId\":\"test\"}]}";
    g_assert_false(publish_windows(&control));
    g_assert_cmpuint(lua_events, ==, 0); /* Initial snapshot establishes baseline. */
    snapshot_fixture = "{\"windows\":[{\"id\":\"w1\",\"title\":\"after\",\"appId\":\"test\"}]}";
    control.state_revision++;
    g_assert_false(publish_windows(&control));
    g_assert_cmpuint(old.messages->len, ==, 2);
    g_assert_cmpuint(legacy.messages->len, ==, 3);
    g_assert_cmpuint(lua_events, ==, 1);
    g_assert_cmpuint(lua_snapshots, ==, 2);
    for (guint i = 0; i < 2; i++) {
        g_autoptr(JsonNode) snapshot = message(&old, i);
        JsonObject* object = json_node_get_object(snapshot);
        g_assert_cmpstr(json_object_get_string_member(object, "event"), ==, "windows");
        g_assert_cmpint(json_object_get_int_member(object, "revision"), ==, 42 + i);
        JsonObject* window =
            json_array_get_object_element(json_object_get_array_member(object, "windows"), 0);
        g_assert_cmpstr(json_object_get_string_member(window, "title"), ==, i ? "after" : "before");
        g_assert_cmpstr(json_object_get_string_member(window, "app_id"), ==, "test");
        g_assert_cmpstr(g_ptr_array_index(old.messages, i), ==,
                        g_ptr_array_index(legacy.messages, i ? 2 : 0));
    }
    client_clear(&old);
    client_clear(&legacy);
    control_clear(&control);
}

static void test_window_version_gates(void) {
    const char* events[] = {"gnoblin.window.menu-requested", "gnoblin.window.activation-denied"};
    guint required[] = {27, 69};
    for (guint i = 0; i < G_N_ELEMENTS(events); i++) {
        GnoblinNativeControl control = control_new();
        Client client = client_new(&control, TRUE, 1);
        g_hash_table_add(control.clients, &client);
        g_hash_table_add(client.event_subscriptions, (gpointer)events[i]);
        for (guint minor = required[i] - 1; minor <= required[i]; minor++) {
            client.event_api_minor = minor;
            g_autoptr(JsonNode) root = json_from_string("{}", NULL);
            json_object_set_string_member(json_node_get_object(root), "name", events[i]);
            publish_native_socket_event(&control, root);
            g_assert_cmpuint(client.messages->len, ==, minor == required[i] ? 1 : 0);
        }
        client_clear(&client);
        control_clear(&control);
    }
}

static void test_no_recipient_work(void) {
    for (guint mode = 0; mode < 4; mode++) {
        GnoblinNativeControl control = control_new();
        Client client = client_new(&control, mode == 1, 0);
        if (mode)
            g_hash_table_add(control.clients, &client);
        client.event_api_minor = mode == 2 ? 8 : 9;
        g_hash_table_add(client.event_subscriptions,
                         mode == 2 ? "gnoblin.window.changed" : "gnoblin.monitor.changed");
        for (guint i = 0; i < G_N_ELEMENTS(lifecycle_events); i++) {
            dispatch(&control, lifecycle_events[i]);
            assert_lua(lifecycle_events[i], i + 1);
        }
        g_assert_cmpuint(lua_events, ==, 6);
        g_assert_cmpuint(client.messages->len, ==, 0);
        g_assert_cmpuint(socket_copies, >, 0); /* Existing mutations still run. */
        g_assert_cmpuint(socket_encodes, ==, 0);
        client_clear(&client);
        control_clear(&control);
    }
}

static void test_other_event_side_effects(void) {
    GnoblinNativeControl control = control_new();
    Client client = client_new(&control, FALSE, 14);
    g_hash_table_add(control.clients, &client);
    for (guint minor = 13; minor <= 14; minor++) {
        client.api_minor = minor;
        g_autoptr(JsonNode) root = json_from_string(
            "{\"name\":\"gnoblin.operation.completed\",\"operation_id\":123}", NULL);
        publish_native_socket_event(&control, root);
        g_assert_cmpuint(client.messages->len, ==, minor == 14 ? 1 : 0);
    }
    const char* requests[] = {"gnoblin.auth.requested", "gnoblin.prompt.requested",
                              "gnoblin.location.authorization-requested"};
    client.event_api_minor = 80;
    client.client_id = 99;
    control.pending_auth = control.pending_prompts = &control;
    control.pending_location_authorizations = g_hash_table_new(g_int64_hash, g_int64_equal);
    gint64 request_id = 123;
    PendingLocationAuthorization pending = {
        .recipient_client_ids = g_hash_table_new_full(g_int64_hash, g_int64_equal, g_free, NULL)};
    g_hash_table_insert(control.pending_location_authorizations, &request_id, &pending);
    for (guint i = 0; i < G_N_ELEMENTS(requests); i++) {
        g_hash_table_add(client.event_subscriptions, (gpointer)requests[i]);
        g_autoptr(JsonNode) root = json_from_string("{\"request_id\":123}", NULL);
        json_object_set_string_member(json_node_get_object(root), "name", requests[i]);
        publish_native_socket_event(&control, root);
    }
    g_assert_cmpuint(auth_recipients, ==, 1);
    g_assert_cmpuint(prompt_recipients, ==, 1);
    g_assert_true(g_hash_table_contains(pending.recipient_client_ids, &client.client_id));
    g_assert_cmpuint(client.messages->len, ==, 4);
    g_hash_table_unref(pending.recipient_client_ids);
    g_hash_table_unref(control.pending_location_authorizations);
    client_clear(&client);
    control_clear(&control);
}

static void benchmark(guint iterations) {
    const char* modes[] = {"no-clients", "api-1.0", "api-1.1", "explicit-1.9", "unrelated-1.9"};
    retain_messages = FALSE;
    for (guint mode = 0; mode < G_N_ELEMENTS(modes); mode++) {
        GnoblinNativeControl control = control_new();
        Client client = client_new(&control, mode == 1 || mode == 2, mode == 2 ? 1 : 0);
        if (mode)
            g_hash_table_add(control.clients, &client);
        client.event_api_minor = 9;
        if (mode == 3)
            g_hash_table_add(client.event_subscriptions, "gnoblin.window.changed");
        else if (mode == 4)
            g_hash_table_add(client.event_subscriptions, "gnoblin.monitor.changed");
        gint64 start = g_get_monotonic_time();
        for (guint i = 0; i < iterations; i++)
            dispatch(&control, "gnoblin.window.changed");
        gint64 elapsed = g_get_monotonic_time() - start;
        g_print("%s events=%u lua=%u copies=%u encodes=%u encoded_bytes=%" G_GUINT64_FORMAT
                " delivered_bytes=%" G_GUINT64_FORMAT " elapsed_us=%" G_GINT64_FORMAT "\n",
                modes[mode], iterations, lua_events, socket_copies, socket_encodes, encoded_bytes,
                delivered_bytes, elapsed);
        client_clear(&client);
        control_clear(&control);
    }
}

int main(int argc, char** argv) {
    if (argc == 3 && g_str_equal(argv[1], "--benchmark")) {
        benchmark((guint)g_ascii_strtoull(argv[2], NULL, 10));
        return 0;
    }
    g_test_init(&argc, &argv, NULL);
    g_test_add_func("/compat/legacy-windows-and-lua", test_legacy_windows);
    g_test_add_func("/compat/explicit-and-fanout", test_explicit_and_fanout);
    g_test_add_func("/compat/shared-mutations-aliases-private-context", test_mutations);
    g_test_add_func("/compat/api-1.0-snapshots", test_snapshots);
    g_test_add_func("/compat/window-version-gates", test_window_version_gates);
    g_test_add_func("/compat/other-event-side-effects", test_other_event_side_effects);
    g_test_add_func("/optimization/no-recipient-work", test_no_recipient_work);
    return g_test_run();
}
