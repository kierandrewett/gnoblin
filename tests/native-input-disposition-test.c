#include <gio/gio.h>

#include "../src/native-control/gnoblin-input-disposition.h"

static GHashTable* dispositions(void) {
    return g_hash_table_new_full(g_str_hash, g_str_equal, g_free, NULL);
}

static void test_fresh_press_retires_lost_terminal_state(void) {
    g_autoptr(GHashTable) held = dispositions();
    g_hash_table_insert(held, g_strdup("button:mouse:1"), GINT_TO_POINTER(2));

    g_assert_true(gnoblin_input_disposition_discard_stale(
        held, "button:mouse:1", TRUE, FALSE));
    g_assert_false(g_hash_table_contains(held, "button:mouse:1"));
}

static void test_key_repeat_keeps_consumed_stream_state(void) {
    g_autoptr(GHashTable) held = dispositions();
    g_hash_table_insert(held, g_strdup("key:keyboard:37"), GINT_TO_POINTER(2));

    g_assert_false(gnoblin_input_disposition_discard_stale(
        held, "key:keyboard:37", TRUE, TRUE));
    g_assert_true(g_hash_table_contains(held, "key:keyboard:37"));
}

static void test_non_begin_event_keeps_stream_ownership(void) {
    g_autoptr(GHashTable) held = dispositions();
    g_hash_table_insert(held, g_strdup("touch:touchscreen:0"), GINT_TO_POINTER(2));

    g_assert_false(gnoblin_input_disposition_discard_stale(
        held, "touch:touchscreen:0", FALSE, FALSE));
    g_assert_true(g_hash_table_contains(held, "touch:touchscreen:0"));
}

int main(int argc, char** argv) {
    g_test_init(&argc, &argv, NULL);
    g_test_add_func("/input-disposition/fresh-press-retires-lost-terminal",
                    test_fresh_press_retires_lost_terminal_state);
    g_test_add_func("/input-disposition/key-repeat-keeps-stream",
                    test_key_repeat_keeps_consumed_stream_state);
    g_test_add_func("/input-disposition/non-begin-keeps-stream",
                    test_non_begin_event_keeps_stream_ownership);
    return g_test_run();
}
