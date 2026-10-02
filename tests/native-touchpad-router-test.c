#include <gio/gio.h>

#include "../src/native-control/gnoblin-touchpad-router.h"

static GVariant* variant(const char* text) {
    g_autoptr(GError) error = NULL;
    GVariant* value = g_variant_parse(NULL, text, NULL, NULL, &error);
    g_assert_no_error(error);
    return g_variant_ref_sink(value);
}

static GVariant* swipe_gestures(const char* when, double threshold, double tolerance) {
    g_autofree char* text =
        g_strdup_printf("[<{'name': <'right-down'>, 'gesture': <'swipe'>, 'fingers': <int64 3>, "
                        "'path': <[<{'x': <0.0>, 'y': <0.0>}>, <{'x': <1.0>, 'y': <0.0>}>, "
                        "<{'x': <1.0>, 'y': <1.0>}>]>, 'threshold': <%g>, 'tolerance': <%g>, "
                        "'when': <'%s'>, 'command': <[<'true'>]>}>]",
                        threshold, tolerance, when);
    return variant(text);
}

static GVariant* payload(const char* text) {
    return variant(text);
}

static gboolean handle(GnoblinTouchpadRouter* router, GVariant* gestures, const char* event,
                       const char* context, GVariant** matched) {
    g_autoptr(GVariant) value = payload(event);
    return gnoblin_touchpad_router_handle(router, gestures, value, context, matched);
}

static void assert_no_match(GVariant* matched) {
    g_assert_null(matched);
}

static void test_matching_swipe(void) {
    g_autoptr(GnoblinTouchpadRouter) router = gnoblin_touchpad_router_new();
    g_autoptr(GVariant) gestures = swipe_gestures("normal", 16, .2);
    GVariant* matched = NULL;

    g_assert_true(handle(router, gestures,
                         "{'gesture': <'swipe'>, 'phase': <'begin'>, 'fingers': <int64 3>}",
                         "normal", &matched));
    assert_no_match(matched);
    g_assert_true(handle(router, gestures,
                         "{'gesture': <'swipe'>, 'phase': <'update'>, 'dx': <20.0>, 'dy': <0.0>}",
                         "normal", &matched));
    assert_no_match(matched);
    g_assert_true(handle(router, gestures,
                         "{'gesture': <'swipe'>, 'phase': <'update'>, 'dx': <0.0>, 'dy': <20.0>}",
                         "normal", &matched));
    assert_no_match(matched);
    g_assert_true(
        handle(router, gestures, "{'gesture': <'swipe'>, 'phase': <'end'>}", "normal", &matched));
    g_assert_nonnull(matched);
    g_autoptr(GVariant) name = g_variant_lookup_value(matched, "name", G_VARIANT_TYPE_STRING);
    g_assert_cmpstr(g_variant_get_string(name, NULL), ==, "right-down");
    g_variant_unref(matched);
}

static void test_path_normalization_and_tolerance(void) {
    g_autoptr(GnoblinTouchpadRouter) router = gnoblin_touchpad_router_new();
    g_autoptr(GVariant) gestures = swipe_gestures("normal", 16, .1);
    GVariant* matched = NULL;
    g_assert_true(handle(router, gestures,
                         "{'gesture': <'swipe'>, 'phase': <'begin'>, 'fingers': <int64 3>}",
                         "normal", &matched));
    g_assert_true(handle(router, gestures,
                         "{'gesture': <'swipe'>, 'phase': <'update'>, 'dx': <12.0>, 'dy': <0.0>}",
                         "normal", &matched));
    g_assert_true(handle(router, gestures,
                         "{'gesture': <'swipe'>, 'phase': <'update'>, 'dx': <13.0>, 'dy': <0.0>}",
                         "normal", &matched));
    g_assert_true(handle(router, gestures,
                         "{'gesture': <'swipe'>, 'phase': <'update'>, 'dx': <0.0>, 'dy': <10.0>}",
                         "normal", &matched));
    g_assert_true(handle(router, gestures,
                         "{'gesture': <'swipe'>, 'phase': <'update'>, 'dx': <0.0>, 'dy': <15.0>}",
                         "normal", &matched));
    g_assert_true(
        handle(router, gestures, "{'gesture': <'swipe'>, 'phase': <'end'>}", "normal", &matched));
    g_assert_nonnull(matched);
    g_variant_unref(matched);
}

static void test_threshold_and_claimed_nonmatch(void) {
    g_autoptr(GnoblinTouchpadRouter) router = gnoblin_touchpad_router_new();
    g_autoptr(GVariant) gestures = swipe_gestures("normal", 48, .2);
    GVariant* matched = NULL;
    g_assert_true(handle(router, gestures,
                         "{'gesture': <'swipe'>, 'phase': <'begin'>, 'fingers': <int64 3>}",
                         "normal", &matched));
    g_assert_true(handle(router, gestures,
                         "{'gesture': <'swipe'>, 'phase': <'update'>, 'dx': <12.0>, 'dy': <0.0>}",
                         "normal", &matched));
    g_assert_true(
        handle(router, gestures, "{'gesture': <'swipe'>, 'phase': <'end'>}", "normal", &matched));
    assert_no_match(matched);

    g_assert_true(handle(router, gestures,
                         "{'gesture': <'swipe'>, 'phase': <'begin'>, 'fingers': <int64 3>}",
                         "normal", &matched));
    g_assert_true(handle(router, gestures,
                         "{'gesture': <'swipe'>, 'phase': <'update'>, 'dx': <48.0>, 'dy': <0.0>}",
                         "normal", &matched));
    g_assert_true(
        handle(router, gestures, "{'gesture': <'swipe'>, 'phase': <'end'>}", "normal", &matched));
    assert_no_match(matched);
}

static GVariant* pinch_gestures(void) {
    return variant("[<{'name': <'in'>, 'gesture': <'pinch'>, 'fingers': <int64 2>, "
                   "'direction': <'in'>, 'threshold': <0.1>, 'when': <'normal'>, 'action': "
                   "<'window.close'>}>, "
                   "<{'name': <'out'>, 'gesture': <'pinch'>, 'fingers': <int64 2>, "
                   "'direction': <'out'>, 'threshold': <0.1>, 'when': <'normal'>, 'action': "
                   "<'window.minimize'>}>]");
}

static void test_pinch_directions(void) {
    g_autoptr(GnoblinTouchpadRouter) router = gnoblin_touchpad_router_new();
    g_autoptr(GVariant) gestures = pinch_gestures();
    GVariant* matched = NULL;
    g_assert_true(
        handle(router, gestures,
               "{'gesture': <'pinch'>, 'phase': <'begin'>, 'fingers': <int64 2>, 'scale': <1.0>}",
               "normal", &matched));
    g_assert_true(handle(router, gestures,
                         "{'gesture': <'pinch'>, 'phase': <'end'>, 'scale': <1.2>}", "normal",
                         &matched));
    g_assert_nonnull(matched);
    g_variant_unref(matched);
    matched = NULL;
    g_assert_true(
        handle(router, gestures,
               "{'gesture': <'pinch'>, 'phase': <'begin'>, 'fingers': <int64 2>, 'scale': <1.0>}",
               "normal", &matched));
    g_assert_true(handle(router, gestures,
                         "{'gesture': <'pinch'>, 'phase': <'end'>, 'scale': <0.8>}", "normal",
                         &matched));
    g_assert_nonnull(matched);
    g_variant_unref(matched);
}

static void test_context_cancel_and_config_change(void) {
    g_autoptr(GnoblinTouchpadRouter) router = gnoblin_touchpad_router_new();
    g_autoptr(GVariant) normal = swipe_gestures("normal", 16, .2);
    g_autoptr(GVariant) unlock = swipe_gestures("unlock-screen", 16, .2);
    g_autoptr(GVariant) any = swipe_gestures("any", 16, .2);
    GVariant* matched = NULL;
    g_assert_false(handle(router, unlock,
                          "{'gesture': <'swipe'>, 'phase': <'begin'>, 'fingers': <int64 3>}",
                          "normal", &matched));
    assert_no_match(matched);
    g_assert_true(handle(router, any,
                         "{'gesture': <'swipe'>, 'phase': <'begin'>, 'fingers': <int64 3>}",
                         "emoji-picker", &matched));
    g_assert_true(handle(router, any, "{'gesture': <'swipe'>, 'phase': <'cancel'>}", "emoji-picker",
                         &matched));
    g_assert_true(handle(router, normal,
                         "{'gesture': <'swipe'>, 'phase': <'begin'>, 'fingers': <int64 3>}",
                         "normal", &matched));
    g_assert_true(
        handle(router, normal, "{'gesture': <'swipe'>, 'phase': <'cancel'>}", "normal", &matched));
    assert_no_match(matched);
    g_assert_true(handle(router, normal,
                         "{'gesture': <'swipe'>, 'phase': <'begin'>, 'fingers': <int64 3>}",
                         "normal", &matched));
    g_assert_false(handle(router, unlock,
                          "{'gesture': <'swipe'>, 'phase': <'update'>, 'dx': <20.0>, 'dy': <0.0>}",
                          "normal", &matched));
    assert_no_match(matched);
}

static void test_progress_and_malformed_events_do_not_claim(void) {
    g_autoptr(GnoblinTouchpadRouter) router = gnoblin_touchpad_router_new();
    g_autoptr(GVariant) gestures =
        variant("[<{'name': <'progress'>, 'gesture': <'swipe'>, 'fingers': <int64 3>, "
                "'path': <[<{'x': <0.0>, 'y': <0.0>}>, <{'x': <1.0>, 'y': <0.0>}>]>, "
                "'action': <'workspace.progress'>}>]");
    GVariant* matched = NULL;
    g_assert_false(handle(router, gestures,
                          "{'gesture': <'swipe'>, 'phase': <'begin'>, 'fingers': <int64 3>}",
                          "normal", &matched));
    g_assert_false(handle(router, gestures, "{'gesture': <'swipe'>}", "normal", &matched));
    assert_no_match(matched);
}

static void test_native_any_binding_claims_only_its_context(void) {
    g_autoptr(GnoblinTouchpadRouter) router = gnoblin_touchpad_router_new();
    g_autoptr(GVariant) gestures =
        variant("[<{'name': <'close-window'>, 'gesture': <'pinch'>, 'fingers': <int64 2>, "
                "'direction': <'in'>, 'threshold': <0.1>, 'when': <'any'>, "
                "'action': <'window.close'>}>]");
    GVariant* matched = NULL;

    g_assert_true(
        handle(router, gestures,
               "{'gesture': <'pinch'>, 'phase': <'begin'>, 'fingers': <int64 2>, 'scale': <1.0>}",
               "normal", &matched));
    g_assert_true(handle(router, gestures,
                         "{'gesture': <'pinch'>, 'phase': <'end'>, 'scale': <0.8>}", "normal",
                         &matched));
    g_assert_nonnull(matched);
    g_autoptr(GVariant) action = g_variant_lookup_value(matched, "action", G_VARIANT_TYPE_STRING);
    g_assert_cmpstr(g_variant_get_string(action, NULL), ==, "window.close");
    g_variant_unref(matched);
}

static void test_normal_binding_matches_unlocked_session_context(void) {
    g_autoptr(GnoblinTouchpadRouter) router = gnoblin_touchpad_router_new();
    g_autoptr(GVariant) gestures =
        variant("[<{'name': <'workspace-next'>, 'gesture': <'swipe'>, 'fingers': <int64 3>, "
                "'path': <[<{'x': <0.0>, 'y': <0.0>}>, <{'x': <1.0>, 'y': <0.0>}>]>, "
                "'when': <'normal'>, 'action': <'workspace.next'>}>]");
    GVariant* matched = NULL;
    g_assert_true(handle(router, gestures,
                         "{'gesture': <'swipe'>, 'phase': <'begin'>, 'fingers': <int64 3>}",
                         "normal", &matched));
    g_assert_true(handle(router, gestures,
                         "{'gesture': <'swipe'>, 'phase': <'update'>, 'dx': <48.0>, 'dy': <0.0>}",
                         "normal", &matched));
    g_assert_true(
        handle(router, gestures, "{'gesture': <'swipe'>, 'phase': <'end'>}", "normal", &matched));
    g_assert_nonnull(matched);
    g_variant_unref(matched);
}

static void test_command_preserves_empty_argument(void) {
    g_autoptr(GnoblinTouchpadRouter) router = gnoblin_touchpad_router_new();
    g_autoptr(GVariant) gestures =
        variant("[<{'name': <'command'>, 'gesture': <'pinch'>, 'fingers': <int64 2>, "
                "'direction': <'in'>, 'when': <'any'>, "
                "'command': <[<'printf'>, <''>, <'value'>]>}>]");
    GVariant* matched = NULL;

    g_assert_true(
        handle(router, gestures,
               "{'gesture': <'pinch'>, 'phase': <'begin'>, 'fingers': <int64 2>, 'scale': <1.0>}",
               "normal", &matched));
    g_assert_true(handle(router, gestures,
                         "{'gesture': <'pinch'>, 'phase': <'end'>, 'scale': <0.8>}", "normal",
                         &matched));
    g_assert_nonnull(matched);
    g_autoptr(GVariant) command = g_variant_lookup_value(matched, "command", G_VARIANT_TYPE("av"));
    g_assert_nonnull(command);
    g_autoptr(GVariant) empty_boxed = g_variant_get_child_value(command, 1);
    g_autoptr(GVariant) empty = g_variant_get_variant(empty_boxed);
    g_assert_cmpstr(g_variant_get_string(empty, NULL), ==, "");
    g_variant_unref(matched);
}

int main(int argc, char** argv) {
    g_test_init(&argc, &argv, NULL);
    g_test_add_func("/native/touchpad-router/matching-swipe", test_matching_swipe);
    g_test_add_func("/native/touchpad-router/path-normalization",
                    test_path_normalization_and_tolerance);
    g_test_add_func("/native/touchpad-router/threshold-and-claimed-nonmatch",
                    test_threshold_and_claimed_nonmatch);
    g_test_add_func("/native/touchpad-router/pinch-directions", test_pinch_directions);
    g_test_add_func("/native/touchpad-router/context-cancel-config-change",
                    test_context_cancel_and_config_change);
    g_test_add_func("/native/touchpad-router/progress-and-malformed",
                    test_progress_and_malformed_events_do_not_claim);
    g_test_add_func("/native/touchpad-router/any-binding-normal-context",
                    test_native_any_binding_claims_only_its_context);
    g_test_add_func("/native/touchpad-router/normal-binding-unlocked-context",
                    test_normal_binding_matches_unlocked_session_context);
    g_test_add_func("/native/touchpad-router/empty-command-argument",
                    test_command_preserves_empty_argument);
    return g_test_run();
}
