#include "gnoblin-config.h"
#include "gnoblin-portal-policy.h"
#include <glib/gstdio.h>
#include <string.h>

static void capture_animation_callback_warning(const gchar* domain, GLogLevelFlags level,
                                               const gchar* message, gpointer user_data) {
    gchar** warning = user_data;
    if (!*warning && strstr(message, "Lua event 'test.animation' failed:"))
        *warning = g_strdup(message);
    else
        g_log_default_handler(domain, level, message, NULL);
}

static GVariant* load(const char* path, GPtrArray** paths, GError** error) {
    return gnoblin_config_load_document(path, paths, NULL, error);
}

int main(void) {
    g_autoptr(GError) error = NULL;
    g_assert_true(gnoblin_config_window_pattern_match("dock", "prefix-dock-suffix", &error));
    g_assert_no_error(error);
    g_assert_true(gnoblin_config_window_pattern_match("^dock$", "dock", &error));
    g_assert_no_error(error);
    g_assert_false(gnoblin_config_window_pattern_match("^dock$", "prefix-dock", &error));
    g_assert_no_error(error);
    g_assert_true(gnoblin_config_window_pattern_match("^app%d+[%a_]+$", "app42_name", &error));
    g_assert_no_error(error);
    g_assert_true(gnoblin_config_window_pattern_match("^a%%b$", "a%b", &error));
    g_assert_no_error(error);
    g_assert_false(gnoblin_config_window_pattern_match("^a%%b$", "axb", &error));
    g_assert_no_error(error);
    g_assert_false(gnoblin_config_window_pattern_match("%", "value", &error));
    g_assert_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL);
    g_assert_nonnull(strstr(error->message, "invalid Lua pattern"));
    g_clear_error(&error);

    g_autofree char* dir = g_dir_make_tmp("gnoblin-lua-test-XXXXXX", &error);
    g_autofree char* conf = g_build_filename(dir, "conf.d", NULL);
    g_autofree char* root = g_build_filename(dir, "init.lua", NULL);
    g_autofree char* example_root = g_build_filename(dir, "example.lua", NULL);
    g_autofree char* explicit_root = g_build_filename(dir, "personal.lua", NULL);
    g_autofree char* malformed_patterns = g_build_filename(dir, "malformed-patterns.lua", NULL);
    g_autofree char* module = g_build_filename(dir, "module.lua", NULL);
    g_autofree char* nested = g_build_filename(dir, "nested.lua", NULL);
    g_autofree char* legacy_root = g_build_filename(dir, "legacy.toml", NULL);
    g_autofree char* missing_legacy_root = g_build_filename(dir, "missing.toml", NULL);
    g_autofree char* missing_legacy_conf = g_build_filename(dir, "missing.conf", NULL);
    g_autofree char* missing_root = g_build_filename(dir, "missing.lua", NULL);
    g_autofree char* legacy_include = g_build_filename(dir, "legacy-include.lua", NULL);
    g_autofree char* legacy_child = g_build_filename(dir, "legacy-child.conf", NULL);
    g_assert_no_error(error);
    g_autoptr(GVariant) defaults = load(missing_root, NULL, &error);
    g_assert_no_error(error);
    g_assert_nonnull(defaults);
    g_assert_cmpuint(g_variant_n_children(defaults), ==, 0);
    g_clear_pointer(&defaults, g_variant_unref);
    const char* legacy_roots[] = {missing_legacy_root, missing_legacy_conf, NULL};
    for (guint i = 0; legacy_roots[i]; i++) {
        g_autoptr(GVariant) rejected = load(legacy_roots[i], NULL, &error);
        g_assert_null(rejected);
        g_assert_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL);
        g_assert_nonnull(strstr(error->message, "configuration uses Lua only"));
        g_clear_error(&error);
    }
    g_assert_cmpint(g_mkdir(conf, 0700), ==, 0);
    g_assert_true(g_file_set_contents(module, "return { name = 'module' }\n", -1, &error));
    g_assert_true(g_file_set_contents(
        nested,
        "local g=require('gnoblin'); g.configure {compositor={['enable-animations']=true}}\n", -1,
        &error));
    g_autofree char* fragment = g_build_filename(conf, "10-bingux.lua", NULL);
    g_assert_true(g_file_set_contents(
        fragment, "return { shortcuts = {{ name = 'search', command = {'binguxctl'} }} }\n", -1,
        &error));
    g_assert_true(g_file_set_contents(
        root,
        "local g=require('gnoblin'); local a=require('module'); local b=require('module')\n"
        "if a~=b then error('require cache') end\n"
        "assert(type(g.animation)=='function', 'gnoblin.animation must declare animations')\n"
        "for _,name in ipairs({'list','get','surfaces','inspect','preview',\n"
        "    'seek','step','play','pause','stop'}) do\n"
        "  assert(type(g.animations[name])=='function', 'missing gnoblin.animations.'..name)\n"
        "end\n"
        "g.configure { compositor={['enable-animations']=false}, shortcuts={} }; "
        "g.config.autostart={}\n"
        "g.animation { name='test-open', event='open', duration=240, from={scale_x=0.8}, "
        "to={scale_x=1} }\n"
        "g.animation { name='test-open', duration=260 }\n"
        "g.animation { name='removed', event='close', from={opacity=1}, to={opacity=0} }\n"
        "g.animation { name='removed', enable=false }\n"
        "g.load('nested.lua'); g.load('conf.d/**/*.lua')\n",
        -1, &error));
    g_assert_no_error(error);
    g_autoptr(GPtrArray) paths = NULL;
    g_autoptr(GVariant) document = load(root, &paths, &error);
    g_assert_no_error(error);
    g_assert_nonnull(document);
    g_assert_false(g_variant_is_floating(document));
    g_autoptr(GVariant) compositor =
        g_variant_lookup_value(document, "compositor", G_VARIANT_TYPE_VARDICT);
    g_autoptr(GVariant) animations_enabled =
        g_variant_lookup_value(compositor, "enable-animations", G_VARIANT_TYPE_BOOLEAN);
    g_assert_true(g_variant_get_boolean(animations_enabled));
    g_autoptr(GVariant) shortcuts =
        g_variant_lookup_value(document, "shortcuts", G_VARIANT_TYPE("av"));
    g_assert_cmpuint(g_variant_n_children(shortcuts), ==, 1);
    g_autoptr(GVariant) animations =
        g_variant_lookup_value(document, "animations", G_VARIANT_TYPE("av"));
    g_assert_cmpuint(g_variant_n_children(animations), ==, 1);
    g_autoptr(GVariant) animation_box = g_variant_get_child_value(animations, 0);
    g_autoptr(GVariant) animation = g_variant_get_variant(animation_box);
    g_autoptr(GVariant) animation_name =
        g_variant_lookup_value(animation, "name", G_VARIANT_TYPE_STRING);
    g_autoptr(GVariant) animation_duration =
        g_variant_lookup_value(animation, "duration", G_VARIANT_TYPE_INT64);
    g_autoptr(GVariant) animation_from =
        g_variant_lookup_value(animation, "from", G_VARIANT_TYPE_VARDICT);
    g_autoptr(GVariant) animation_scale =
        g_variant_lookup_value(animation_from, "scale-x", G_VARIANT_TYPE_DOUBLE);
    g_assert_cmpstr(g_variant_get_string(animation_name, NULL), ==, "test-open");
    g_assert_cmpint(g_variant_get_int64(animation_duration), ==, 260);
    g_assert_cmpfloat(g_variant_get_double(animation_scale), ==, 0.8);
    g_assert_cmpuint(paths->len, >=, 4);

    g_assert_true(g_file_set_contents(malformed_patterns,
                                      "local g=require('gnoblin')\n"
                                      "g.window_rule { match={type='window', title='ok'} }\n"
                                      "g.window_rule { match={type='window', title='['} }\n",
                                      -1, &error));
    g_clear_pointer(&document, g_variant_unref);
    document = load(malformed_patterns, NULL, &error);
    g_assert_null(document);
    g_assert_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL);
    g_assert_nonnull(strstr(error->message, "window-rules[2].match.title"));
    g_clear_error(&error);

    const char* source_root = g_getenv("GNOBLIN_TEST_SOURCE_ROOT");
    if (source_root) {
        g_autofree char* example =
            g_build_filename(source_root, "src", "data", "init.lua.example", NULL);
        g_autofree char* example_source = NULL;
        g_assert_true(g_file_get_contents(example, &example_source, NULL, &error));
        g_assert_no_error(error);
        g_assert_true(g_file_set_contents(example_root, example_source, -1, &error));
        g_assert_no_error(error);
        g_clear_pointer(&document, g_variant_unref);
        document = load(example_root, NULL, &error);
        g_assert_no_error(error);
        g_assert_nonnull(document);
        g_autoptr(GVariant) example_shortcuts =
            g_variant_lookup_value(document, "shortcuts", G_VARIANT_TYPE("av"));
        g_assert_nonnull(example_shortcuts);
        /* The seed includes ten command shortcuts and one compositor action. */
        g_assert_cmpuint(g_variant_n_children(example_shortcuts), ==, 11);
        g_autoptr(GVariant) example_window_management =
            g_variant_lookup_value(document, "window-management", G_VARIANT_TYPE_VARDICT);
        g_assert_nonnull(example_window_management);
        g_autoptr(GVariant) example_focus_new_windows = g_variant_lookup_value(
            example_window_management, "focus-new-windows", G_VARIANT_TYPE_STRING);
        g_assert_nonnull(example_focus_new_windows);
        g_assert_cmpstr(g_variant_get_string(example_focus_new_windows, NULL), ==, "strict");
    }

    g_assert_true(g_file_set_contents(explicit_root, "return {}\n", -1, &error));
    g_clear_pointer(&document, g_variant_unref);
    document = load(explicit_root, NULL, &error);
    g_assert_no_error(error);
    g_assert_nonnull(document);

    /* A detected old root gives a conversion error instead of being parsed. */
    g_assert_true(g_file_set_contents(legacy_root, "legacy = true\n", -1, &error));
    g_autoptr(GVariant) rejected_root = load(legacy_root, NULL, &error);
    g_assert_null(rejected_root);
    g_assert_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL);
    g_assert_nonnull(strstr(error->message, "Convert this file to Lua"));
    g_clear_error(&error);

    /* Includes follow the same Lua-only rule and name the file to convert. */
    g_assert_true(g_file_set_contents(legacy_child, "legacy = true\n", -1, &error));
    g_assert_true(
        g_file_set_contents(legacy_include, "gnoblin.load('legacy-child.conf')\n", -1, &error));
    g_autoptr(GVariant) rejected_include = load(legacy_include, NULL, &error);
    g_assert_null(rejected_include);
    g_assert_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL);
    g_assert_nonnull(strstr(error->message, "legacy-child.conf"));
    g_assert_nonnull(strstr(error->message, "Convert this file to Lua"));
    g_clear_error(&error);
    g_autoptr(GVariant) absent_input = gnoblin_config_normalize_input(document, &error);
    g_assert_null(absent_input);
    g_assert_no_error(error);

    const char* input_source =
        "gnoblin.configure { input = {"
        "  mouse = { speed = 0.25, accel_profile = 'custom',"
        "    accel_curve = { step = 1, points = {0, 0.5, 1} },"
        "    left_handed = 'inherit' },"
        "  touchpad = { scroll_speed = 2, left_handed = 'mouse', click_method = 'fingers' },"
        "  keyboard = { delay = 10000, repeat_interval = 30, ['repeat'] = true,"
        "    xkb_options = {'caps:escape'},"
        "    numlock_state = 'inherit' },"
        "  tablets = { ['056a:00b9'] = { mapping = 'relative', keep_aspect = true },"
        "    ['1234:abcd'] = { mapping = 'inherit' } },"
        "  styluses = { ['123'] = { button_action = 'keybinding',"
        "    button_keybinding = 'XF86AudioMute' } },"
        "  orientation_lock = false } }\n";
    g_assert_true(g_file_set_contents(explicit_root, input_source, -1, &error));
    g_clear_pointer(&document, g_variant_unref);
    document = load(explicit_root, NULL, &error);
    g_assert_no_error(error);
    g_assert_nonnull(document);
    g_autoptr(GVariant) input_overlay = gnoblin_config_normalize_input(document, &error);
    g_assert_no_error(error);
    g_assert_nonnull(input_overlay);
    g_assert_cmpstr(g_variant_get_type_string(input_overlay), ==, "a{sv}");
    g_autoptr(GVariant) mouse =
        g_variant_lookup_value(input_overlay, "mouse", G_VARIANT_TYPE_VARDICT);
    g_assert_nonnull(mouse);
    g_assert_null(g_variant_lookup_value(mouse, "left-handed", NULL));
    g_autoptr(GVariant) speed = g_variant_lookup_value(mouse, "speed", NULL);
    g_assert_true(g_variant_is_of_type(speed, G_VARIANT_TYPE_DOUBLE));
    g_assert_cmpfloat(g_variant_get_double(speed), ==, 0.25);
    g_autoptr(GVariant) curve = g_variant_lookup_value(mouse, "accel-curve", NULL);
    g_autoptr(GVariant) step = g_variant_lookup_value(curve, "step", NULL);
    g_autoptr(GVariant) points = g_variant_lookup_value(curve, "points", NULL);
    g_assert_true(g_variant_is_of_type(step, G_VARIANT_TYPE_DOUBLE));
    g_assert_cmpfloat(g_variant_get_double(step), ==, 1.0);
    g_assert_true(g_variant_is_of_type(points, G_VARIANT_TYPE("ad")));
    g_assert_cmpuint(g_variant_n_children(points), ==, 3);
    g_autoptr(GVariant) first_point = g_variant_get_child_value(points, 0);
    g_assert_cmpfloat(g_variant_get_double(first_point), ==, 0.0);
    g_autoptr(GVariant) touchpad =
        g_variant_lookup_value(input_overlay, "touchpad", G_VARIANT_TYPE_VARDICT);
    g_autoptr(GVariant) scroll_speed =
        g_variant_lookup_value(touchpad, "scroll-speed", G_VARIANT_TYPE_DOUBLE);
    g_assert_cmpfloat(g_variant_get_double(scroll_speed), ==, 2.0);
    g_autoptr(GVariant) keyboard =
        g_variant_lookup_value(input_overlay, "keyboard", G_VARIANT_TYPE_VARDICT);
    g_assert_null(g_variant_lookup_value(keyboard, "numlock-state", NULL));
    g_autoptr(GVariant) delay = g_variant_lookup_value(keyboard, "delay", G_VARIANT_TYPE_UINT32);
    g_assert_cmpuint(g_variant_get_uint32(delay), ==, 10000);
    g_autoptr(GVariant) repeat_interval =
        g_variant_lookup_value(keyboard, "repeat-interval", G_VARIANT_TYPE_UINT32);
    g_assert_cmpuint(g_variant_get_uint32(repeat_interval), ==, 30);
    g_autoptr(GVariant) xkb_options =
        g_variant_lookup_value(keyboard, "xkb-options", G_VARIANT_TYPE_STRING_ARRAY);
    g_assert_cmpuint(g_variant_n_children(xkb_options), ==, 1);
    g_autoptr(GVariant) xkb_option = g_variant_get_child_value(xkb_options, 0);
    g_assert_cmpstr(g_variant_get_string(xkb_option, NULL), ==, "caps:escape");
    g_autoptr(GVariant) orientation_lock =
        g_variant_lookup_value(input_overlay, "orientation-lock", G_VARIANT_TYPE_BOOLEAN);
    g_assert_false(g_variant_get_boolean(orientation_lock));
    g_autoptr(GVariant) tablets =
        g_variant_lookup_value(input_overlay, "tablets", G_VARIANT_TYPE_VARDICT);
    g_assert_null(g_variant_lookup_value(tablets, "1234:abcd", NULL));
    g_autoptr(GVariant) mapped_tablet =
        g_variant_lookup_value(tablets, "056a:00b9", G_VARIANT_TYPE_VARDICT);
    g_assert_nonnull(mapped_tablet);

    g_assert_true(g_file_set_contents(explicit_root,
                                      "gnoblin.configure {input = {mouse = {speed = 'inherit'},"
                                      " tablets = {['1234:abcd'] = {mapping = 'inherit'}},"
                                      " orientation_lock = 'inherit'}}\n",
                                      -1, &error));
    g_clear_pointer(&document, g_variant_unref);
    document = load(explicit_root, NULL, &error);
    g_assert_no_error(error);
    g_clear_pointer(&input_overlay, g_variant_unref);
    input_overlay = gnoblin_config_normalize_input(document, &error);
    g_assert_no_error(error);
    g_assert_cmpuint(g_variant_n_children(input_overlay), ==, 0);

    const char* invalid_input_sources[] = {
        "gnoblin.configure {input = {mouse = {unknown = true}}}\n",
        "gnoblin.configure {input = {mouse = {speed = '0.5'}}}\n",
        "gnoblin.configure {input = {mouse = {unknown = 'inherit'}}}\n",
        "gnoblin.configure {input = {touchpad = {scroll_speed = 2.1}}}\n",
        "gnoblin.configure {input = {tablets = {['1234:abcd'] = {speed = 'inherit'}}}}\n",
        "gnoblin.configure {input = {keyboard = {delay = 0}}}\n",
        "gnoblin.configure {input = {mouse = {accel_curve = {step = 0, points = {0, 1}}}}}\n",
        "gnoblin.configure {input = {mouse = {accel_curve = {step = 1, points = {0}}}}}\n",
        "gnoblin.configure {input = {tablets = {['not-a-device'] = {mapping = 'absolute'}}}}\n",
        "gnoblin.configure {input = {orientation_lock = 1}}\n",
        NULL,
    };
    for (guint i = 0; invalid_input_sources[i]; i++) {
        g_assert_true(g_file_set_contents(explicit_root, invalid_input_sources[i], -1, &error));
        g_clear_pointer(&document, g_variant_unref);
        document = load(explicit_root, NULL, &error);
        if (document)
            g_error("invalid input case %u was accepted: %s", i, invalid_input_sources[i]);
        g_assert_null(document);
        g_assert_nonnull(error);
        g_clear_error(&error);
    }

    const char* touchpad_gesture_source =
        "gnoblin.configure {touchpad_gestures = {"
        " {name = 'workspace-next', gesture = 'swipe', fingers = 3,"
        "  path = {{x = 0, y = 0}, {x = 1, y = 0}}, action = 'workspace.previous'},"
        " {name = 'open-terminal', gesture = 'pinch', fingers = 4, direction = 'out',"
        "  command = {'kgx'}, when = 'any', threshold = 0.12}"
        "}}\n";
    g_assert_true(g_file_set_contents(explicit_root, touchpad_gesture_source, -1, &error));
    g_clear_pointer(&document, g_variant_unref);
    document = load(explicit_root, NULL, &error);
    g_assert_no_error(error);
    g_assert_nonnull(document);
    g_autoptr(GVariant) touchpad_gestures =
        g_variant_lookup_value(document, "touchpad-gestures", G_VARIANT_TYPE("av"));
    g_assert_cmpuint(g_variant_n_children(touchpad_gestures), ==, 2);
    g_autoptr(GVariant) swipe_boxed = g_variant_get_child_value(touchpad_gestures, 0);
    g_autoptr(GVariant) swipe = g_variant_get_variant(swipe_boxed);
    g_autoptr(GVariant) swipe_path = g_variant_lookup_value(swipe, "path", G_VARIANT_TYPE("av"));
    g_assert_cmpuint(g_variant_n_children(swipe_path), ==, 2);
    g_autoptr(GVariant) point_boxed = g_variant_get_child_value(swipe_path, 1);
    g_autoptr(GVariant) point = g_variant_get_variant(point_boxed);
    g_autoptr(GVariant) point_x = g_variant_lookup_value(point, "x", G_VARIANT_TYPE_INT64);
    g_assert_cmpint(g_variant_get_int64(point_x), ==, 1);

    const char* invalid_touchpad_gesture_sources[] = {
        "gnoblin.configure {touchpad_gestures = {bad = true}}\n",
        "gnoblin.configure {touchpad_gestures = {{name = 'bad', gesture = 'swipe', fingers = 3,"
        " path = {{x = 0, y = 0}, {x = 1, y = 0}}, action = 'workspace.next', extra = true}}}\n",
        "gnoblin.configure {touchpad_gestures = {{name = 'bad name', gesture = 'swipe', fingers = "
        "3,"
        " path = {{x = 0, y = 0}, {x = 1, y = 0}}, action = 'workspace.next'}}}\n",
        "gnoblin.configure {touchpad_gestures = {{name = 'pinch', gesture = 'pinch', fingers = 3,"
        " direction = 'sideways', action = 'workspace.next'}}}\n",
        "gnoblin.configure {touchpad_gestures = {{name = 'path', gesture = 'swipe', fingers = 3,"
        " path = {{x = 0, y = 0}, {x = 0, y = 0}}, action = 'workspace.next'}}}\n",
        "gnoblin.configure {touchpad_gestures = {{name = 'fingers', gesture = 'swipe', fingers = 1,"
        " path = {{x = 0, y = 0}, {x = 1, y = 0}}, action = 'workspace.next'}}}\n",
        "gnoblin.configure {touchpad_gestures = {{name = 'choice', gesture = 'swipe', fingers = 3,"
        " path = {{x = 0, y = 0}, {x = 1, y = 0}}, action = 'workspace.next', command = "
        "{'kgx'}}}}\n",
        "gnoblin.configure {touchpad_gestures = {{name = 'argv', gesture = 'pinch', fingers = 3,"
        " direction = 'in', command = {}}}}\n",
        "gnoblin.configure {touchpad_gestures = {{name = 'when', gesture = 'pinch', fingers = 3,"
        " direction = 'in', action = 'workspace.next', when = 'elsewhere'}}}\n",
        "gnoblin.configure {touchpad_gestures = {{name = 'emoji', gesture = 'pinch', fingers = 3,"
        " direction = 'in', action = 'workspace.next', when = 'emoji-picker'}}}\n",
        "gnoblin.configure {touchpad_gestures = {{name = 'progress', gesture = 'swipe', fingers = "
        "3,"
        " path = {{x = 0, y = 0}, {x = 1, y = 0}}, action = 'workspace.progress'}}}\n",
        "gnoblin.configure {touchpad_gestures = {{name = 'threshold', gesture = 'swipe', fingers = "
        "3,"
        " path = {{x = 0, y = 0}, {x = 1, y = 0}}, action = 'workspace.next', threshold = 15}}}\n",
        "gnoblin.configure {touchpad_gestures = {{name = 'tolerance', gesture = 'pinch', fingers = "
        "3,"
        " direction = 'out', action = 'workspace.next', tolerance = 0.2}}}\n",
        "gnoblin.configure {touchpad_gestures = {{name = 'vertical', gesture = 'swipe', fingers = "
        "3,"
        " path = {{x = 0, y = 0}, {x = 0, y = 1}}, action = 'workspace.progress'}}}\n",
        "gnoblin.configure {touchpad_gestures = {"
        " {name = 'one', gesture = 'swipe', fingers = 3, path = {{x = 0, y = 0}, {x = 1, y = 0}}, "
        "action = 'workspace.next'},"
        " {name = 'two', gesture = 'swipe', fingers = 3, path = {{x = 0, y = 0}, {x = 1, y = 0}}, "
        "action = 'workspace.previous'}"
        "}}\n",
        NULL,
    };
    for (guint i = 0; invalid_touchpad_gesture_sources[i]; i++) {
        g_assert_true(
            g_file_set_contents(explicit_root, invalid_touchpad_gesture_sources[i], -1, &error));
        g_clear_pointer(&document, g_variant_unref);
        document = load(explicit_root, NULL, &error);
        if (document)
            g_error("invalid touchpad gesture case %u was accepted: %s", i,
                    invalid_touchpad_gesture_sources[i]);
        g_assert_null(document);
        g_assert_nonnull(error);
        g_clear_error(&error);
    }

    const char* event_config_source =
        "local g=require('gnoblin')\n"
        "assert(g.workspace == nil)\n"
        "assert(type(g.workspaces.list)=='function')\n"
        "assert(type(g.workspaces.create)=='function')\n"
        "assert(type(g.workspaces.rename)=='function')\n"
        "assert(type(g.workspaces.remove)=='function')\n"
        "assert(type(g.workspaces.activate)=='function')\n"
        "assert(type(g.workspaces.next)=='function')\n"
        "assert(type(g.workspaces.previous)=='function')\n"
        "assert(type(g.workspaces.move_active)=='function')\n"
        "assert(type(g.workspaces.move_window)=='function')\n"
        "g.animation {name='test-open', event='open', duration=240}\n"
        "g.animation {name='test-layer-open', event='layer-open', duration=230}\n"
        "g.animation {name='test-layer-close', event='layer-close', duration=240}\n"
        "g.window_rule {match={type='layer', layer='^test%-layer$'},"
        "animation={['in']='test-layer-open', out='test-layer-close',"
        "duration=321, easing='linear'}}\n"
        "g.window_rule {match={type='window', focused=true},"
        "corners={shadow={x=0, y=8, blur=20, opacity=0.2}}}\n"
        "g.permission_rule {name='remote-test', match='^app%-id:org%.example%.Remote$', "
        "capabilities={'remote-desktop'}, level='allow', devices={'keyboard','touchscreen'}, "
        "clipboard=true}\n"
        "g.on('mutter.touchpad.gesture', function(event)\n"
        "  if event.phase == 'begin' then g.workspaces.next() end\n"
        "end)\n"
        "g.on('test.animation', function()\n"
        "  local animations=g.animations.list()\n"
        "  assert(animations ~= nil, 'g.animations.list() returned nil')\n"
        "  assert(#animations==1 and animations[1].name=='test-open')\n"
        "  local animation=g.animations.get('test-open')\n"
        "  assert(animation.event=='open' and animation.duration==240)\n"
        "  assert(g.animations.get('missing')==nil)\n"
        "  local layer=g.layers.animation_policy('test-layer')\n"
        "  assert(layer.namespace=='test-layer' and layer.enter.animation=='test-layer-open')\n"
        "  assert(layer.exit.animation=='test-layer-close' and layer.exit.duration==321)\n"
        "  assert(layer.window_shadow.opacity==0.2)\n"
        "  assert(not pcall(function() animation.name='changed' end))\n"
        "  assert(not pcall(function() g.animations.get() end))\n"
        "  assert(not pcall(function() g.animations.get('bad name') end))\n"
        "  local decision=g.permissions.check('remote-desktop','app-id:org.example.Remote')\n"
        "  assert(decision.level=='allow' and decision.rule=='remote-test', "
        "'permission decision was '..tostring(decision.level)..' from '..tostring(decision.rule))\n"
        "  assert(#decision.devices==2 and decision.devices[1]=='keyboard' and "
        "decision.devices[2]=='touchscreen')\n"
        "  assert(decision.clipboard and decision.revision==g.permissions.policy().revision)\n"
        "end)\n"
        "g.on('gnoblin.shortcut.binding-activated', function(event)\n"
        "  assert(event.first==true)\n"
        "  if event.focus_context~=nil then assert(type(event.focus_context)=='userdata') end\n"
        "  assert(event.focus_context_handle==nil and event.focus_context_generation==nil)\n"
        "end)\n"
        "g.on('test.shortcut.register', function()\n"
        "  g.shortcuts.bind {id='test-switcher', accelerator='<Super>space', hold='super', "
        "mode='modal'}\n"
        "end)\n";
    g_assert_true(g_file_set_contents(explicit_root, event_config_source, -1, &error));
    g_autoptr(GVariant) runtime_document =
        gnoblin_config_load_runtime(explicit_root, NULL, NULL, &error);
    g_assert_no_error(error);
    g_assert_nonnull(runtime_document);
    g_auto(GnoblinPermission) expected_permission = gnoblin_permission_policy_evaluate(
        runtime_document, "remote-desktop", "app-id:org.example.Remote");
    g_assert_cmpint(expected_permission.level, ==, GNOBLIN_PERMISSION_ALLOW);
    g_assert_cmpstr(expected_permission.rule, ==, "remote-test");
    gnoblin_config_finish_load(TRUE);

    GVariantBuilder empty_bind_event;
    g_variant_builder_init(&empty_bind_event, G_VARIANT_TYPE_VARDICT);
    g_autoptr(GVariant) bind_event_payload =
        g_variant_ref_sink(g_variant_builder_end(&empty_bind_event));
    g_autoptr(GVariant) bind_event_result =
        gnoblin_config_dispatch_event("test.shortcut.register", bind_event_payload, &error);
    g_assert_no_error(error);
    g_assert_nonnull(bind_event_result);
    g_autoptr(GVariant) bind_operations = gnoblin_config_drain_runtime_operations();
    g_assert_cmpuint(g_variant_n_children(bind_operations), ==, 1);
    g_autoptr(GVariant) bind_operation = g_variant_get_child_value(bind_operations, 0);
    g_autoptr(GVariant) bind_method =
        g_variant_lookup_value(bind_operation, "method", G_VARIANT_TYPE_STRING);
    g_assert_cmpstr(g_variant_get_string(bind_method, NULL), ==, "shortcut.bind");
    g_autoptr(GVariant) bind_operation_args =
        g_variant_lookup_value(bind_operation, "arguments", G_VARIANT_TYPE_VARDICT);
    g_autoptr(GVariant) bind_hold =
        g_variant_lookup_value(bind_operation_args, "hold", G_VARIANT_TYPE_STRING);
    g_assert_cmpstr(g_variant_get_string(bind_hold, NULL), ==, "super");
    gnoblin_config_finish_event(TRUE);

    GVariantBuilder trusted_shortcut;
    g_autoptr(GVariant) dispatched_document = NULL;
    g_variant_builder_init(&trusted_shortcut, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&trusted_shortcut, "{sv}", "id", g_variant_new_string("test-switcher"));
    g_variant_builder_add(&trusted_shortcut, "{sv}", "first", g_variant_new_boolean(TRUE));
    g_variant_builder_add(&trusted_shortcut, "{sv}", "session_id", g_variant_new_uint64(9));
    g_autoptr(GVariant) trusted_payload =
        g_variant_ref_sink(g_variant_builder_end(&trusted_shortcut));
    g_clear_pointer(&dispatched_document, g_variant_unref);
    dispatched_document = gnoblin_config_dispatch_shortcut_event(
        "gnoblin.shortcut.binding-activated", trusted_payload, 1, 1,
        g_get_monotonic_time() + G_USEC_PER_SEC, &error);
    g_assert_no_error(error);
    g_assert_nonnull(dispatched_document);
    gnoblin_config_finish_event(TRUE);

    g_clear_pointer(&dispatched_document, g_variant_unref);
    dispatched_document = gnoblin_config_dispatch_event("gnoblin.shortcut.binding-activated",
                                                        trusted_payload, &error);
    g_assert_no_error(error);
    g_assert_nonnull(dispatched_document);
    gnoblin_config_finish_event(TRUE);

    GVariantBuilder animation_info;
    g_variant_builder_init(&animation_info, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&animation_info, "{sv}", "name", g_variant_new_string("test-open"));
    g_variant_builder_add(&animation_info, "{sv}", "event", g_variant_new_string("open"));
    g_variant_builder_add(&animation_info, "{sv}", "duration", g_variant_new_uint32(240));
    GVariantBuilder animation_entries;
    g_variant_builder_init(&animation_entries, G_VARIANT_TYPE("aa{sv}"));
    g_variant_builder_add_value(&animation_entries, g_variant_builder_end(&animation_info));
    GVariantBuilder animation_snapshot;
    g_variant_builder_init(&animation_snapshot, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&animation_snapshot, "{sv}", "animations",
                          g_variant_builder_end(&animation_entries));
    g_autoptr(GVariant) native_animations =
        g_variant_ref_sink(g_variant_builder_end(&animation_snapshot));
    gnoblin_config_update_animation_snapshot(native_animations, 1);

    GVariantBuilder layer_policy_arguments_builder;
    g_variant_builder_init(&layer_policy_arguments_builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&layer_policy_arguments_builder, "{sv}", "namespace",
                          g_variant_new_string("test-layer"));
    g_autoptr(GVariant) layer_policy_arguments =
        g_variant_ref_sink(g_variant_builder_end(&layer_policy_arguments_builder));
    g_autoptr(GVariant) layer_policy =
        gnoblin_config_read_api("layer.animation_policy", layer_policy_arguments, &error);
    g_assert_no_error(error);
    g_assert_nonnull(layer_policy);
    const char* policy_namespace = NULL;
    gint64 policy_revision = 0;
    g_assert_true(g_variant_lookup(layer_policy, "namespace", "&s", &policy_namespace));
    g_assert_cmpstr(policy_namespace, ==, "test-layer");
    g_assert_true(g_variant_lookup(layer_policy, "revision", "x", &policy_revision));
    g_assert_cmpint(policy_revision, ==, gnoblin_config_settings_revision());
    g_autoptr(GVariant) enter_policy =
        g_variant_lookup_value(layer_policy, "enter", G_VARIANT_TYPE_VARDICT);
    g_autoptr(GVariant) exit_policy =
        g_variant_lookup_value(layer_policy, "exit", G_VARIANT_TYPE_VARDICT);
    const char* selected_animation = NULL;
    gint64 policy_duration = 0;
    const char* policy_easing = NULL;
    g_assert_nonnull(enter_policy);
    g_assert_nonnull(exit_policy);
    g_assert_true(g_variant_lookup(enter_policy, "animation", "&s", &selected_animation));
    g_assert_cmpstr(selected_animation, ==, "test-layer-open");
    g_assert_true(g_variant_lookup(enter_policy, "duration", "x", &policy_duration));
    g_assert_cmpint(policy_duration, ==, 321);
    g_assert_true(g_variant_lookup(enter_policy, "easing", "&s", &policy_easing));
    g_assert_cmpstr(policy_easing, ==, "linear");
    g_assert_true(g_variant_lookup(exit_policy, "animation", "&s", &selected_animation));
    g_assert_cmpstr(selected_animation, ==, "test-layer-close");
    g_assert_true(g_variant_lookup(exit_policy, "duration", "x", &policy_duration));
    g_assert_cmpint(policy_duration, ==, 321);
    g_autoptr(GVariant) window_shadow =
        g_variant_lookup_value(layer_policy, "window_shadow", G_VARIANT_TYPE_VARDICT);
    g_assert_nonnull(window_shadow);
    GVariantBuilder invalid_layer_policy_arguments_builder;
    g_variant_builder_init(&invalid_layer_policy_arguments_builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&invalid_layer_policy_arguments_builder, "{sv}", "namespace",
                          g_variant_new_string(""));
    g_autoptr(GVariant) invalid_layer_policy_arguments =
        g_variant_ref_sink(g_variant_builder_end(&invalid_layer_policy_arguments_builder));
    g_autoptr(GVariant) invalid_layer_policy =
        gnoblin_config_read_api("layer.animation_policy", invalid_layer_policy_arguments, &error);
    g_assert_null(invalid_layer_policy);
    g_assert_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL);
    g_clear_error(&error);

    g_auto(GStrv) event_names = gnoblin_config_runtime_events();
    gboolean has_touchpad_event = FALSE;
    for (guint i = 0; event_names && event_names[i]; i++)
        has_touchpad_event |= g_str_equal(event_names[i], "mutter.touchpad.gesture");
    g_assert_true(has_touchpad_event);

    GVariantBuilder payload_builder;
    g_variant_builder_init(&payload_builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&payload_builder, "{sv}", "gesture", g_variant_new_string("swipe"));
    g_variant_builder_add(&payload_builder, "{sv}", "phase", g_variant_new_string("begin"));
    g_variant_builder_add(&payload_builder, "{sv}", "fingers", g_variant_new_int64(3));
    g_autoptr(GVariant) payload = g_variant_ref_sink(g_variant_builder_end(&payload_builder));
    dispatched_document = gnoblin_config_dispatch_event("mutter.touchpad.gesture", payload, &error);
    g_assert_no_error(error);
    g_assert_nonnull(dispatched_document);
    g_autoptr(GVariant) operations = gnoblin_config_drain_runtime_operations();
    g_assert_cmpuint(g_variant_n_children(operations), ==, 1);
    g_autoptr(GVariant) operation = g_variant_get_child_value(operations, 0);
    g_autoptr(GVariant) method = g_variant_lookup_value(operation, "method", G_VARIANT_TYPE_STRING);
    g_assert_cmpstr(g_variant_get_string(method, NULL), ==, "workspace.next");
    gnoblin_config_finish_event(TRUE);

    g_variant_builder_init(&payload_builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&payload_builder, "{sv}", "gesture", g_variant_new_string("swipe"));
    g_variant_builder_add(&payload_builder, "{sv}", "phase", g_variant_new_string("update"));
    g_variant_builder_add(&payload_builder, "{sv}", "fingers", g_variant_new_int64(3));
    g_clear_pointer(&payload, g_variant_unref);
    payload = g_variant_ref_sink(g_variant_builder_end(&payload_builder));
    g_clear_pointer(&dispatched_document, g_variant_unref);
    dispatched_document = gnoblin_config_dispatch_event("mutter.touchpad.gesture", payload, &error);
    g_assert_no_error(error);
    g_assert_nonnull(dispatched_document);
    gnoblin_config_finish_event(TRUE);

    g_variant_builder_init(&payload_builder, G_VARIANT_TYPE_VARDICT);
    g_clear_pointer(&payload, g_variant_unref);
    payload = g_variant_ref_sink(g_variant_builder_end(&payload_builder));
    g_clear_pointer(&dispatched_document, g_variant_unref);
    gchar* animation_callback_warning = NULL;
    guint animation_warning_handler = g_log_set_handler(
        NULL, G_LOG_LEVEL_WARNING, capture_animation_callback_warning, &animation_callback_warning);
    dispatched_document = gnoblin_config_dispatch_event("test.animation", payload, &error);
    g_log_remove_handler(NULL, animation_warning_handler);
    if (animation_callback_warning)
        g_error("unexpected animation callback warning: %s", animation_callback_warning);
    g_assert_null(animation_callback_warning);
    g_assert_no_error(error);
    g_assert_nonnull(dispatched_document);
    g_clear_pointer(&operations, g_variant_unref);
    operations = gnoblin_config_drain_runtime_operations();
    g_assert_cmpuint(g_variant_n_children(operations), ==, 0);
    gnoblin_config_finish_event(TRUE);

    g_assert_true(g_file_set_contents(nested, "return 1\n", -1, &error));
    g_clear_pointer(&document, g_variant_unref);
    document = load(root, NULL, &error);
    g_assert_null(document);
    g_assert_nonnull(error);
    g_clear_error(&error);

    const char* failures[] = {
        "error({})\n",
        "while true do end\n",
        "local x = string.rep('x', 16 * 1024 * 1024)\n",
        "local g=require('gnoblin'); g.config=42\n",
        "local g=require('gnoblin'); g.config=g.array{1}\n",
        "local g=require('gnoblin'); g.config=nil; setmetatable(g,{__index=function() error({}) "
        "end})\n",
        "return {1}\n",
        NULL,
    };
    for (guint i = 0; failures[i]; i++) {
        g_assert_true(g_file_set_contents(root, failures[i], -1, &error));
        document = load(root, NULL, &error);
        g_assert_null(document);
        g_assert_nonnull(error);
        g_clear_error(&error);
    }
    g_assert_true(g_file_set_contents(nested, "local g=require('gnoblin'); g.load('init.lua')\n",
                                      -1, &error));
    g_assert_true(g_file_set_contents(root, "local g=require('gnoblin'); g.load('nested.lua')\n",
                                      -1, &error));
    document = load(root, NULL, &error);
    g_assert_null(document);
    g_assert_nonnull(error);
    g_clear_error(&error);

    GVariantBuilder available_builder;
    g_variant_builder_init(&available_builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&available_builder, "{sv}", "screen_sharing",
                          g_variant_new_boolean(TRUE));
    g_variant_builder_add(&available_builder, "{sv}", "recording", g_variant_new_boolean(FALSE));
    g_variant_builder_add(&available_builder, "{sv}", "microphone_in_use",
                          g_variant_new_boolean(TRUE));
    g_variant_builder_add(&available_builder, "{sv}", "camera_in_use",
                          g_variant_new_boolean(FALSE));
    g_variant_builder_add(&available_builder, "{sv}", "location_in_use",
                          g_variant_new_boolean(TRUE));
    GVariantBuilder privacy_builder;
    g_variant_builder_init(&privacy_builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&privacy_builder, "{sv}", "available",
                          g_variant_builder_end(&available_builder));
    g_variant_builder_add(&privacy_builder, "{sv}", "screen_sharing", g_variant_new_boolean(TRUE));
    g_variant_builder_add(&privacy_builder, "{sv}", "microphone_in_use",
                          g_variant_new_boolean(FALSE));
    g_variant_builder_add(&privacy_builder, "{sv}", "location_in_use", g_variant_new_boolean(TRUE));
    g_autoptr(GVariant) privacy_snapshot =
        g_variant_ref_sink(g_variant_builder_end(&privacy_builder));
    gnoblin_config_update_privacy_snapshot(privacy_snapshot, 41);
    const char* privacy_source =
        "local g=require('gnoblin')\n"
        "local state=g.privacy.state()\n"
        "assert(state.revision==41 and state.screen_sharing and not state.microphone_in_use)\n"
        "assert(state.camera_in_use==nil and state.location_in_use)\n"
        "assert(state.available.screen_sharing and state.available.microphone_in_use)\n"
        "assert(not state.available.camera_in_use and state.available.location_in_use)\n"
        "assert(not pcall(function() state.screen_sharing=false end))\n"
        "assert(not pcall(function() state.available.camera_in_use=true end))\n"
        "g.on('test.privacy', function()\n"
        "  local latest=g.privacy.state()\n"
        "  assert(latest.revision==42 and not latest.screen_sharing)\n"
        "  assert(latest.microphone_in_use==nil and latest.camera_in_use==nil)\n"
        "  assert(latest.location_in_use==nil)\n"
        "  assert(not latest.available.microphone_in_use and not latest.available.camera_in_use)\n"
        "  assert(not latest.available.location_in_use)\n"
        "end)\n"
        "g.on('gnoblin.window.changed', function(event)\n"
        "  assert(event.name=='gnoblin.window.changed')\n"
        "  assert(event.window.id=='window-1')\n"
        "  assert(not pcall(function() event.window.above=true end))\n"
        "  assert(event.window:set_above(true).method=='window.set_above')\n"
        "end)\n"
        "g.on('test.snapshot_methods', function()\n"
        "  local window=assert(g.windows.by_id('window-1'))\n"
        "  assert(window:set_above(true).method=='window.set_above')\n"
        "  local workspace=assert(g.workspaces.active())\n"
        "  assert(workspace.id=='workspace-1' and workspace.name=='Main')\n"
        "  assert(g.workspaces.by_id('workspace-1'):rename('Work').method=='workspace.rename')\n"
        "  assert(g.workspaces.list()[1]:activate().method=='workspace.switch')\n"
        "  assert(workspace:remove().method=='workspace.remove')\n"
        "  assert(workspace:move_here(window,{follow=true}).method=='workspace.move_window')\n"
        "  "
        "assert(g.workspaces.create({name='Terminal',id='terminal',activate=true}).method=='"
        "workspace.create')\n"
        "  assert(g.workspaces.next().method=='workspace.next')\n"
        "  assert(g.workspaces.previous().method=='workspace.previous')\n"
        "end)\n";
    g_assert_true(g_file_set_contents(explicit_root, privacy_source, -1, &error));
    g_clear_pointer(&document, g_variant_unref);
    document = gnoblin_config_load_runtime(explicit_root, NULL, NULL, &error);
    g_assert_no_error(error);
    g_assert_nonnull(document);
    gnoblin_config_finish_load(TRUE);

    g_variant_builder_init(&available_builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&available_builder, "{sv}", "screen_sharing",
                          g_variant_new_boolean(TRUE));
    g_variant_builder_add(&available_builder, "{sv}", "recording", g_variant_new_boolean(FALSE));
    g_variant_builder_add(&available_builder, "{sv}", "microphone_in_use",
                          g_variant_new_boolean(FALSE));
    g_variant_builder_add(&available_builder, "{sv}", "camera_in_use",
                          g_variant_new_boolean(FALSE));
    g_variant_builder_add(&available_builder, "{sv}", "location_in_use",
                          g_variant_new_boolean(FALSE));
    g_variant_builder_init(&privacy_builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&privacy_builder, "{sv}", "available",
                          g_variant_builder_end(&available_builder));
    g_variant_builder_add(&privacy_builder, "{sv}", "screen_sharing", g_variant_new_boolean(FALSE));
    g_clear_pointer(&privacy_snapshot, g_variant_unref);
    privacy_snapshot = g_variant_ref_sink(g_variant_builder_end(&privacy_builder));
    gnoblin_config_update_privacy_snapshot(privacy_snapshot, 42);

    GVariantBuilder capability_record_builder;
    g_variant_builder_init(&capability_record_builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&capability_record_builder, "{sv}", "id",
                          g_variant_new_string("window-list"));
    g_variant_builder_add(&capability_record_builder, "{sv}", "available",
                          g_variant_new_boolean(TRUE));
    GVariantBuilder capabilities_builder;
    g_variant_builder_init(&capabilities_builder, G_VARIANT_TYPE("av"));
    g_variant_builder_add(&capabilities_builder, "v",
                          g_variant_builder_end(&capability_record_builder));
    GVariantBuilder capability_snapshot_builder;
    g_variant_builder_init(&capability_snapshot_builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&capability_snapshot_builder, "{sv}", "capabilities",
                          g_variant_builder_end(&capabilities_builder));
    g_autoptr(GVariant) capability_snapshot =
        g_variant_ref_sink(g_variant_builder_end(&capability_snapshot_builder));
    gnoblin_config_update_capability_snapshot(capability_snapshot, 9);

    GVariantBuilder window_record_builder;
    g_variant_builder_init(&window_record_builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&window_record_builder, "{sv}", "id", g_variant_new_string("window-1"));
    g_variant_builder_add(&window_record_builder, "{sv}", "focused", g_variant_new_boolean(TRUE));
    g_variant_builder_add(&window_record_builder, "{sv}", "workspace_id",
                          g_variant_new_string("workspace-1"));
    g_variant_builder_add(&window_record_builder, "{sv}", "monitor_id",
                          g_variant_new_string("monitor-1"));
    GVariantBuilder frame_builder;
    g_variant_builder_init(&frame_builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&frame_builder, "{sv}", "x", g_variant_new_int64(0));
    g_variant_builder_add(&frame_builder, "{sv}", "y", g_variant_new_int64(0));
    g_variant_builder_add(&frame_builder, "{sv}", "width", g_variant_new_int64(800));
    g_variant_builder_add(&frame_builder, "{sv}", "height", g_variant_new_int64(600));
    g_variant_builder_add(&window_record_builder, "{sv}", "frame",
                          g_variant_builder_end(&frame_builder));
    GVariantBuilder windows_builder;
    g_variant_builder_init(&windows_builder, G_VARIANT_TYPE("av"));
    g_variant_builder_add(&windows_builder, "v", g_variant_builder_end(&window_record_builder));
    GVariantBuilder window_snapshot_builder;
    g_variant_builder_init(&window_snapshot_builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&window_snapshot_builder, "{sv}", "windows",
                          g_variant_builder_end(&windows_builder));
    g_autoptr(GVariant) window_snapshot =
        g_variant_ref_sink(g_variant_builder_end(&window_snapshot_builder));
    gnoblin_config_update_window_snapshot(window_snapshot, 17);

    GVariantBuilder workspace_record_builder;
    g_variant_builder_init(&workspace_record_builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&workspace_record_builder, "{sv}", "id",
                          g_variant_new_string("workspace-1"));
    g_variant_builder_add(&workspace_record_builder, "{sv}", "name", g_variant_new_string("Main"));
    g_variant_builder_add(&workspace_record_builder, "{sv}", "number", g_variant_new_int64(1));
    g_variant_builder_add(&workspace_record_builder, "{sv}", "active", g_variant_new_boolean(TRUE));
    GVariantBuilder workspaces_builder;
    g_variant_builder_init(&workspaces_builder, G_VARIANT_TYPE("av"));
    g_variant_builder_add(&workspaces_builder, "v",
                          g_variant_builder_end(&workspace_record_builder));
    GVariantBuilder workspace_snapshot_builder;
    g_variant_builder_init(&workspace_snapshot_builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&workspace_snapshot_builder, "{sv}", "workspaces",
                          g_variant_builder_end(&workspaces_builder));
    g_autoptr(GVariant) workspace_snapshot =
        g_variant_ref_sink(g_variant_builder_end(&workspace_snapshot_builder));
    gnoblin_config_update_workspace_snapshot(workspace_snapshot, 17);

    GVariantBuilder snapshot_methods_payload_builder;
    g_variant_builder_init(&snapshot_methods_payload_builder, G_VARIANT_TYPE_VARDICT);
    g_autoptr(GVariant) snapshot_methods_payload =
        g_variant_ref_sink(g_variant_builder_end(&snapshot_methods_payload_builder));
    g_autoptr(GVariant) snapshot_methods_result =
        gnoblin_config_dispatch_event("test.snapshot_methods", snapshot_methods_payload, &error);
    g_assert_no_error(error);
    g_assert_nonnull(snapshot_methods_result);
    g_autoptr(GVariant) snapshot_method_operations = gnoblin_config_drain_runtime_operations();
    const char* const expected_snapshot_methods[] = {
        "window.set_above",      "workspace.rename", "workspace.switch", "workspace.remove",
        "workspace.move_window", "workspace.create", "workspace.next",   "workspace.previous",
    };
    g_assert_cmpuint(g_variant_n_children(snapshot_method_operations), ==,
                     G_N_ELEMENTS(expected_snapshot_methods));
    for (gsize i = 0; i < G_N_ELEMENTS(expected_snapshot_methods); i++) {
        g_autoptr(GVariant) operation = g_variant_get_child_value(snapshot_method_operations, i);
        g_autoptr(GVariant) method =
            g_variant_lookup_value(operation, "method", G_VARIANT_TYPE_STRING);
        g_assert_nonnull(method);
        g_assert_cmpstr(g_variant_get_string(method, NULL), ==, expected_snapshot_methods[i]);
    }
    gnoblin_config_finish_event(TRUE);

    GVariantBuilder event_window_builder;
    g_variant_builder_init(&event_window_builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&event_window_builder, "{sv}", "id", g_variant_new_string("window-1"));
    g_variant_builder_add(&event_window_builder, "{sv}", "focused", g_variant_new_boolean(TRUE));
    GVariantBuilder event_window_frame_builder;
    g_variant_builder_init(&event_window_frame_builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&event_window_frame_builder, "{sv}", "x", g_variant_new_int64(0));
    g_variant_builder_add(&event_window_frame_builder, "{sv}", "y", g_variant_new_int64(0));
    g_variant_builder_add(&event_window_frame_builder, "{sv}", "width", g_variant_new_int64(800));
    g_variant_builder_add(&event_window_frame_builder, "{sv}", "height", g_variant_new_int64(600));
    g_variant_builder_add(&event_window_builder, "{sv}", "frame",
                          g_variant_builder_end(&event_window_frame_builder));
    GVariantBuilder window_event_builder;
    g_variant_builder_init(&window_event_builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&window_event_builder, "{sv}", "window",
                          g_variant_builder_end(&event_window_builder));
    g_autoptr(GVariant) window_event_payload =
        g_variant_ref_sink(g_variant_builder_end(&window_event_builder));
    g_autoptr(GVariant) window_event_result =
        gnoblin_config_dispatch_event("gnoblin.window.changed", window_event_payload, &error);
    g_assert_no_error(error);
    g_assert_nonnull(window_event_result);
    g_autoptr(GVariant) window_event_operations = gnoblin_config_drain_runtime_operations();
    g_assert_cmpuint(g_variant_n_children(window_event_operations), ==, 1);
    g_autoptr(GVariant) window_event_operation =
        g_variant_get_child_value(window_event_operations, 0);
    g_autoptr(GVariant) window_event_method =
        g_variant_lookup_value(window_event_operation, "method", G_VARIANT_TYPE_STRING);
    g_assert_cmpstr(g_variant_get_string(window_event_method, NULL), ==, "window.set_above");
    gnoblin_config_finish_event(TRUE);

    GVariantBuilder read_filter_builder;
    g_variant_builder_init(&read_filter_builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&read_filter_builder, "{sv}", "workspace_id",
                          g_variant_new_string("workspace-1"));
    g_variant_builder_add(&read_filter_builder, "{sv}", "limit", g_variant_new_int64(1));
    g_autoptr(GVariant) read_filter =
        g_variant_ref_sink(g_variant_builder_end(&read_filter_builder));
    GVariantBuilder read_arguments_builder;
    g_variant_builder_init(&read_arguments_builder, G_VARIANT_TYPE_VARDICT);
    g_autoptr(GVariant) empty_read_arguments =
        g_variant_ref_sink(g_variant_builder_end(&read_arguments_builder));
    g_autoptr(GVariant) version_snapshot =
        gnoblin_config_read_api("version", empty_read_arguments, &error);
    g_assert_no_error(error);
    g_assert_true(g_variant_is_of_type(version_snapshot, G_VARIANT_TYPE_VARDICT));
    g_autoptr(GVariant) version_git_remote =
        g_variant_lookup_value(version_snapshot, "git_remote", G_VARIANT_TYPE_STRING);
    g_assert_nonnull(version_git_remote);

    g_autoptr(GVariant) settings_snapshot =
        gnoblin_config_read_api("settings", empty_read_arguments, &error);
    g_assert_no_error(error);
    g_assert_true(g_variant_is_of_type(settings_snapshot, G_VARIANT_TYPE_VARDICT));
    g_autoptr(GVariant) settings_revision =
        g_variant_lookup_value(settings_snapshot, "revision", G_VARIANT_TYPE_INT64);
    g_assert_nonnull(settings_revision);
    g_autoptr(GVariant) focus_policy =
        gnoblin_config_read_api("focus.policy", empty_read_arguments, &error);
    g_assert_no_error(error);
    const char* focus_mode = NULL;
    g_assert_true(g_variant_lookup(focus_policy, "focus_mode", "&s", &focus_mode));
    g_assert_cmpstr(focus_mode, ==, "click");
    const char* focus_new_windows = NULL;
    g_assert_true(g_variant_lookup(focus_policy, "focus_new_windows", "&s", &focus_new_windows));
    g_assert_cmpstr(focus_new_windows, ==, "strict");

    g_autoptr(GVariant) capabilities =
        gnoblin_config_read_api("capabilities.list", empty_read_arguments, &error);
    g_assert_no_error(error);
    g_assert_true(g_variant_is_of_type(capabilities, G_VARIANT_TYPE("av")));
    g_assert_cmpuint(g_variant_n_children(capabilities), ==, 1);
    g_autoptr(GVariant) capability_box = g_variant_get_child_value(capabilities, 0);
    g_autoptr(GVariant) capability = g_variant_get_variant(capability_box);
    const char* capability_id = NULL;
    g_assert_true(g_variant_lookup(capability, "id", "&s", &capability_id));
    g_assert_cmpstr(capability_id, ==, "window-list");

    g_autoptr(GVariant) focus_history =
        gnoblin_config_read_api("focus.history", read_filter, &error);
    g_assert_no_error(error);
    g_assert_true(g_variant_is_of_type(focus_history, G_VARIANT_TYPE("av")));
    g_assert_cmpuint(g_variant_n_children(focus_history), ==, 1);
    g_autoptr(GVariant) history_box = g_variant_get_child_value(focus_history, 0);
    g_autoptr(GVariant) history_window = g_variant_get_variant(history_box);
    const char* history_id = NULL;
    g_assert_true(g_variant_lookup(history_window, "id", "&s", &history_id));
    g_assert_cmpstr(history_id, ==, "window-1");

    GVariantBuilder empty_windows_snapshot_builder;
    g_variant_builder_init(&empty_windows_snapshot_builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&empty_windows_snapshot_builder, "{sv}", "windows",
                          g_variant_new_array(G_VARIANT_TYPE_VARIANT, NULL, 0));
    g_clear_pointer(&window_snapshot, g_variant_unref);
    window_snapshot = g_variant_ref_sink(g_variant_builder_end(&empty_windows_snapshot_builder));
    gnoblin_config_update_window_snapshot(window_snapshot, 18);
    g_clear_pointer(&focus_history, g_variant_unref);
    focus_history = gnoblin_config_read_api("focus.history", empty_read_arguments, &error);
    g_assert_no_error(error);
    g_assert_true(g_variant_is_of_type(focus_history, G_VARIANT_TYPE("av")));
    g_assert_cmpuint(g_variant_n_children(focus_history), ==, 0);

    g_variant_builder_init(&payload_builder, G_VARIANT_TYPE_VARDICT);
    g_autoptr(GVariant) privacy_payload =
        g_variant_ref_sink(g_variant_builder_end(&payload_builder));
    g_clear_pointer(&document, g_variant_unref);
    document = gnoblin_config_dispatch_event("test.privacy", privacy_payload, &error);
    g_assert_no_error(error);
    g_assert_nonnull(document);
    gnoblin_config_finish_event(TRUE);

    const char* smart_focus_source =
        "gnoblin.configure {window_management = {focus_new_windows = 'smart'}}\n";
    g_assert_true(g_file_set_contents(explicit_root, smart_focus_source, -1, &error));
    g_clear_pointer(&document, g_variant_unref);
    document = gnoblin_config_load_runtime(explicit_root, NULL, NULL, &error);
    g_assert_no_error(error);
    g_assert_nonnull(document);
    gnoblin_config_finish_load(TRUE);
    g_clear_pointer(&focus_policy, g_variant_unref);
    focus_policy = gnoblin_config_read_api("focus.policy", empty_read_arguments, &error);
    g_assert_no_error(error);
    g_assert_true(g_variant_lookup(focus_policy, "focus_new_windows", "&s", &focus_new_windows));
    g_assert_cmpstr(focus_new_windows, ==, "smart");

    g_unlink(fragment);
    g_unlink(malformed_patterns);
    g_unlink(nested);
    g_unlink(module);
    g_unlink(root);
    g_unlink(explicit_root);
    g_rmdir(conf);
    g_rmdir(dir);
    g_print("PASS: Lua config, runtime events, direct values, load, glob and errors\n");
    return 0;
}
