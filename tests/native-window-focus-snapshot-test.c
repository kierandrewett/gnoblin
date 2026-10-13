/* Exercise the production diff/dispatch functions with real JSON-GLib objects.
 * Before changing production, protect focus transfer ordering, baseline/revision
 * silence, immutable snapshots and nested fields, and normal lifecycle changes. */
#include <gio/gio.h>
#include <json-glib/json-glib.h>

typedef struct {
    char* json;
    char* comparable_json;
    gboolean focused;
} NativeWindowState;

typedef struct {
    const char* native_name;
    const char* lua_name;
} WindowPropertyName;

typedef struct {
    GHashTable* window_state;
    gboolean window_state_initialized;
    guint64 event_sequence;
    GPtrArray *socket_events, *lua_events;
} GnoblinNativeControl;

#define META_WINDOW_MODAL_DIALOG 4

static void publish_native_socket_event(GnoblinNativeControl* control, JsonNode* payload) {
    g_ptr_array_add(control->socket_events, json_to_string(payload, FALSE));
}

static gboolean native_runtime_dispatch_event(GnoblinNativeControl* control, const char* event,
                                              GVariant* payload) {
    const char* name = NULL;
    g_assert_true(g_variant_lookup(payload, "name", "&s", &name));
    g_assert_cmpstr(name, ==, event);
    g_ptr_array_add(control->lua_events, g_variant_ref(payload));
    return TRUE;
}

#include "lifecycle.inc"

static GnoblinNativeControl control_new(void) {
    return (GnoblinNativeControl){
        .window_state =
            g_hash_table_new_full(g_str_hash, g_str_equal, g_free, native_window_state_free),
        .socket_events = g_ptr_array_new_with_free_func(g_free),
        .lua_events = g_ptr_array_new_with_free_func((GDestroyNotify)g_variant_unref),
    };
}

static void control_clear(GnoblinNativeControl* control) {
    g_hash_table_unref(control->window_state);
    g_ptr_array_unref(control->socket_events);
    g_ptr_array_unref(control->lua_events);
}

static void publish(GnoblinNativeControl* control, const char* text, guint64 revision) {
    g_autoptr(JsonNode) snapshot = json_from_string(text, NULL);
    g_assert_nonnull(snapshot);
    publish_window_changes(control, snapshot, revision);
}

static void assert_event(GnoblinNativeControl* control, guint index, const char* event,
                         const char* id, gboolean focused, guint64 revision) {
    g_assert_cmpuint(control->socket_events->len, >, index);
    g_assert_cmpuint(control->lua_events->len, ==, control->socket_events->len);
    g_autoptr(JsonNode) root =
        json_from_string(g_ptr_array_index(control->socket_events, index), NULL);
    g_autoptr(JsonNode) lua = json_from_variant(g_ptr_array_index(control->lua_events, index));
    g_assert_true(json_node_equal(root, lua));
    JsonObject* object = json_node_get_object(root);
    g_assert_cmpstr(json_object_get_string_member(object, "name"), ==, event);
    g_assert_cmpint(json_object_get_int_member(object, "sequence"), ==, index + 1);
    g_assert_cmpint(json_object_get_int_member(object, "revision"), ==, revision);
    g_assert_cmpint(json_object_get_int_member(object, "time"), >, 0);
    const char* record_name = g_str_equal(event, "gnoblin.window.closed") ? "last" : "window";
    JsonObject* record = json_object_get_object_member(object, record_name);
    g_assert_cmpstr(json_object_get_string_member(record, "id"), ==, id);
    g_assert_cmpint(json_object_get_boolean_member(record, "focused"), ==, focused);
    g_assert_cmpint(json_object_get_int_member(record, "revision"), ==, revision);
    if (!g_str_equal(event, "gnoblin.window.created"))
        g_assert_cmpstr(json_object_get_string_member(object, "window_id"), ==, id);
}

static void test_focus_transfer(void) {
    GnoblinNativeControl control = control_new();
    publish(&control,
            "{\"windows\":[{\"id\":\"a\",\"focused\":true,\"revision\":1},"
            "{\"id\":\"b\",\"focused\":false,\"revision\":1}]}",
            1);
    g_assert_cmpuint(control.socket_events->len, ==, 0);
    /* Only focus and transport revision change: no generic changed events. */
    publish(&control,
            "{\"windows\":[{\"id\":\"a\",\"focused\":false,\"revision\":2},"
            "{\"id\":\"b\",\"focused\":true,\"revision\":2}]}",
            2);
    g_assert_cmpuint(control.socket_events->len, ==, 2);
    assert_event(&control, 0, "gnoblin.window.unfocused", "a", FALSE, 2);
    assert_event(&control, 1, "gnoblin.window.focused", "b", TRUE, 2);
    publish(&control,
            "{\"windows\":[{\"id\":\"a\",\"focused\":false,\"revision\":3},"
            "{\"id\":\"b\",\"focused\":false,\"revision\":3}]}",
            3);
    g_assert_cmpuint(control.socket_events->len, ==, 3);
    assert_event(&control, 2, "gnoblin.window.unfocused", "b", FALSE, 3);
    control_clear(&control);
}

static void test_snapshot_preserved(gconstpointer data) {
    gboolean immutable = GPOINTER_TO_INT(data);
    GnoblinNativeControl control = control_new();
    g_autoptr(JsonNode) snapshot = json_from_string(
        "{\"event\":\"windows\",\"revision\":99,\"windows\":[{\"id\":\"a\",\"focused\":true,"
        "\"revision\":7,\"title\":\"keep\",\"geometry\":{\"x\":10,\"width\":800},"
        "\"extra\":{\"focused\":true,\"revision\":8,\"items\":[1,2]}}]}",
        NULL);
    if (immutable)
        json_node_seal(snapshot);
    g_autofree char* before = json_to_string(snapshot, FALSE);
    JsonArray* windows = json_object_get_array_member(json_node_get_object(snapshot), "windows");
    JsonNode* record = json_array_get_element(windows, 0);
    g_autofree char* record_before = json_to_string(record, FALSE);
    JsonNode* nested_before = json_object_get_member(json_node_get_object(record), "extra");
    publish_window_changes(&control, snapshot, 99);
    g_autofree char* after = json_to_string(snapshot, FALSE);
    g_assert_cmpstr(after, ==, before);
    g_assert_true(record == json_array_get_element(windows, 0));
    g_assert_true(nested_before == json_object_get_member(json_node_get_object(record), "extra"));
    NativeWindowState* state = g_hash_table_lookup(control.window_state, "a");
    g_assert_true(state->focused);
    g_assert_cmpstr(state->json, ==, record_before);
    g_autoptr(JsonNode) comparable = json_from_string(state->comparable_json, NULL);
    JsonObject* object = json_node_get_object(comparable);
    g_assert_false(json_object_has_member(object, "focused"));
    g_assert_false(json_object_has_member(object, "revision"));
    g_assert_cmpstr(json_object_get_string_member(object, "title"), ==, "keep");
    g_assert_true(json_node_equal(json_object_get_member(object, "extra"), nested_before));
    g_assert_true(
        json_node_equal(json_object_get_member(object, "geometry"),
                        json_object_get_member(json_node_get_object(record), "geometry")));
    /* Reusing the same (possibly sealed) snapshot must also be safe and silent. */
    publish_window_changes(&control, snapshot, 100);
    g_assert_cmpuint(control.socket_events->len, ==, 0);
    g_assert_cmpuint(control.lua_events->len, ==, 0);
    control_clear(&control);
}

static void test_revision_only(void) {
    GnoblinNativeControl control = control_new();
    publish(&control, "{\"windows\":[{\"id\":\"a\",\"focused\":true,\"revision\":1}]}", 1);
    publish(&control, "{\"windows\":[{\"id\":\"a\",\"focused\":true,\"revision\":2}]}", 2);
    g_assert_cmpuint(control.socket_events->len, ==, 0);
    g_assert_cmpuint(control.lua_events->len, ==, 0);
    NativeWindowState* state = g_hash_table_lookup(control.window_state, "a");
    g_assert_true(state->focused);
    control_clear(&control);
}

static void test_lifecycle(void) {
    GnoblinNativeControl control = control_new();
    publish(&control, "{\"windows\":[]}", 1);
    publish(&control,
            "{\"windows\":[{\"id\":\"a\",\"focused\":true,\"title\":\"before\","
            "\"demandsAttention\":false,\"geometry\":{\"x\":10}}]}",
            2);
    g_assert_cmpuint(control.socket_events->len, ==, 2);
    assert_event(&control, 0, "gnoblin.window.created", "a", TRUE, 2);
    assert_event(&control, 1, "gnoblin.window.focused", "a", TRUE, 2);
    publish(&control,
            "{\"windows\":[{\"id\":\"a\",\"focused\":true,\"title\":\"after\","
            "\"demandsAttention\":true,\"geometry\":{\"x\":20}}]}",
            3);
    g_assert_cmpuint(control.socket_events->len, ==, 4);
    assert_event(&control, 2, "gnoblin.window.changed", "a", TRUE, 3);
    assert_event(&control, 3, "gnoblin.window.attention-changed", "a", TRUE, 3);
    g_autoptr(JsonNode) changed_event =
        json_from_string(g_ptr_array_index(control.socket_events, 2), NULL);
    JsonArray* changed =
        json_object_get_array_member(json_node_get_object(changed_event), "changed");
    g_assert_cmpuint(json_array_get_length(changed), ==, 2);
    g_assert_cmpstr(json_array_get_string_element(changed, 0), ==, "title");
    g_assert_cmpstr(json_array_get_string_element(changed, 1), ==, "frame");
    publish(&control, "{\"windows\":[]}", 4);
    g_assert_cmpuint(control.socket_events->len, ==, 5);
    assert_event(&control, 4, "gnoblin.window.closed", "a", TRUE, 4);
    control_clear(&control);
}

static void test_absent_focus(void) {
    GnoblinNativeControl control = control_new();
    publish(&control, "{\"windows\":[{\"id\":\"a\"}]}", 1);
    NativeWindowState* state = g_hash_table_lookup(control.window_state, "a");
    g_assert_false(state->focused);
    publish(&control, "{\"windows\":[{\"id\":\"a\",\"focused\":null}]}", 2);
    g_assert_cmpuint(control.socket_events->len, ==, 0);
    publish(&control, "{\"windows\":[{\"id\":\"a\",\"focused\":true}]}", 3);
    g_assert_cmpuint(control.socket_events->len, ==, 1);
    assert_event(&control, 0, "gnoblin.window.focused", "a", TRUE, 3);
    control_clear(&control);
}

int main(int argc, char** argv) {
    g_test_init(&argc, &argv, NULL);
    g_test_add_func("/focus/transfer", test_focus_transfer);
    g_test_add_func("/focus/revision-only", test_revision_only);
    g_test_add_func("/focus/lifecycle", test_lifecycle);
    g_test_add_func("/focus/absent", test_absent_focus);
    g_test_add_data_func("/snapshot/preserved", GINT_TO_POINTER(FALSE), test_snapshot_preserved);
    g_test_add_data_func("/snapshot/immutable", GINT_TO_POINTER(TRUE), test_snapshot_preserved);
    return g_test_run();
}
