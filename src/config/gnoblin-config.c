/* Lua configuration evaluation and supervisor runtime state. */
#include "gnoblin-config.h"
#include "gnoblin-portal-policy.h"
#include "gnoblin-input-config.h"

#include <errno.h>
#include <gio/gio.h>
#include <glib/gstdio.h>
#include <math.h>
#include <string.h>

static gboolean input_number(GVariant* value, double* number) {
    if (g_variant_is_of_type(value, G_VARIANT_TYPE_DOUBLE)) {
        *number = g_variant_get_double(value);
        return isfinite(*number);
    }
    if (g_variant_is_of_type(value, G_VARIANT_TYPE_INT64)) {
        *number = (double)g_variant_get_int64(value);
        return TRUE;
    }
    if (g_variant_is_of_type(value, G_VARIANT_TYPE_INT32)) {
        *number = (double)g_variant_get_int32(value);
        return TRUE;
    }
    return FALSE;
}

static gboolean input_string_is(GVariant* value, const char* const* choices) {
    if (!g_variant_is_of_type(value, G_VARIANT_TYPE_STRING))
        return FALSE;
    const char* string = g_variant_get_string(value, NULL);
    for (guint i = 0; choices[i]; i++) {
        if (g_str_equal(string, choices[i]))
            return TRUE;
    }
    return FALSE;
}

static gboolean input_field_known(const char* group, const char* key) {
    static const char* const mouse_fields[] = {
        "speed", "left-handed", "natural-scroll", "accel-profile", "accel-curve", NULL,
    };
    static const char* const touchpad_fields[] = {
        "speed",
        "scroll-speed",
        "left-handed",
        "natural-scroll",
        "accel-profile",
        "accel-curve",
        "tap-to-click",
        "tap-button-map",
        "tap-and-drag",
        "tap-and-drag-lock",
        "disable-while-typing",
        "edge-scrolling-enabled",
        "two-finger-scrolling-enabled",
        "click-method",
        NULL,
    };
    static const char* const keyboard_fields[] = {
        "repeat",      "delay", "repeat-interval", "remember-numlock-state", "numlock-state",
        "xkb-options", NULL,
    };
    static const char* const tablet_fields[] = {"mapping", "left-handed", "keep-aspect", NULL};
    static const char* const stylus_fields[] = {
        "eraser-button-mode",         "eraser-button-action",
        "eraser-button-keybinding",   "button-action",
        "secondary-button-action",    "tertiary-button-action",
        "button-keybinding",          "secondary-button-keybinding",
        "tertiary-button-keybinding", NULL,
    };
    const char* const* fields = g_str_equal(group, "mouse")      ? mouse_fields
                                : g_str_equal(group, "touchpad") ? touchpad_fields
                                : g_str_equal(group, "keyboard") ? keyboard_fields
                                : g_str_equal(group, "tablets")  ? tablet_fields
                                : g_str_equal(group, "styluses") ? stylus_fields
                                                                 : NULL;
    if (!fields)
        return FALSE;
    for (guint i = 0; fields[i]; i++) {
        if (g_str_equal(key, fields[i]))
            return TRUE;
    }
    return FALSE;
}

static gboolean input_value_valid(const char* group, const char* key, GVariant* value) {
    static const char* accel_profiles[] = {"default", "flat", "adaptive", "custom", NULL};
    static const char* handedness[] = {"right", "left", "mouse", NULL};
    static const char* tap_button_maps[] = {"default", "lrm", "lmr", NULL};
    static const char* click_methods[] = {"default", "none", "areas", "fingers", NULL};
    static const char* tablet_mapping[] = {"absolute", "relative", NULL};
    static const char* stylus_eraser_modes[] = {"default", "button", NULL};
    static const char* stylus_actions[] = {"default", "middle",         "right",      "back",
                                           "forward", "switch-monitor", "keybinding", NULL};
    if (!input_field_known(group, key))
        return FALSE;
    if (g_variant_is_of_type(value, G_VARIANT_TYPE_STRING) &&
        g_str_equal(g_variant_get_string(value, NULL), "inherit"))
        return TRUE;

    if (g_str_equal(key, "accel-curve")) {
        if (!g_variant_is_of_type(value, G_VARIANT_TYPE_VARDICT))
            return FALSE;
        g_autoptr(GVariant) step_value = g_variant_lookup_value(value, "step", NULL);
        g_autoptr(GVariant) points = g_variant_lookup_value(value, "points", NULL);
        double step;
        gboolean valid = step_value && input_number(step_value, &step) && step > 0 && points &&
                         g_variant_is_of_type(points, G_VARIANT_TYPE("av")) &&
                         g_variant_n_children(points) >= 2;
        GVariantIter iter;
        GVariant* item;
        const char* field;
        GVariant* field_value;
        if (valid) {
            g_variant_iter_init(&iter, value);
            while (valid && g_variant_iter_next(&iter, "{&sv}", &field, &field_value)) {
                valid = g_str_equal(field, "step") || g_str_equal(field, "points");
                g_variant_unref(field_value);
            }
        }
        if (valid) {
            g_variant_iter_init(&iter, points);
            while (valid && (item = g_variant_iter_next_value(&iter))) {
                g_autoptr(GVariant) unboxed = g_variant_get_variant(item);
                double point;
                valid = input_number(unboxed, &point) && point >= 0;
                g_variant_unref(item);
            }
        }
        return valid;
    }
    if (g_str_equal(key, "speed") || g_str_equal(key, "scroll-speed")) {
        double number;
        if (!input_number(value, &number))
            return FALSE;
        return g_str_equal(key, "scroll-speed") ? number >= 0 && number <= 2
                                                : number >= -1 && number <= 1;
    }
    if (g_str_equal(key, "delay") || g_str_equal(key, "repeat-interval")) {
        double number;
        return input_number(value, &number) && number >= 1 && number <= 10000 &&
               number == (gint64)number;
    }
    if (g_str_equal(key, "xkb-options")) {
        if (!g_variant_is_of_type(value, G_VARIANT_TYPE("av")))
            return FALSE;
        GVariantIter iter;
        GVariant* boxed;
        g_variant_iter_init(&iter, value);
        while ((boxed = g_variant_iter_next_value(&iter))) {
            g_autoptr(GVariant) item = g_variant_get_variant(boxed);
            gboolean valid = g_variant_is_of_type(item, G_VARIANT_TYPE_STRING);
            g_variant_unref(boxed);
            if (!valid)
                return FALSE;
        }
        return TRUE;
    }
    if (g_str_equal(key, "accel-profile"))
        return input_string_is(value, accel_profiles);
    if (g_str_equal(key, "left-handed")) {
        if (g_str_equal(group, "touchpad"))
            return input_string_is(value, handedness);
        return g_variant_is_of_type(value, G_VARIANT_TYPE_BOOLEAN);
    }
    if (g_str_equal(key, "tap-button-map"))
        return input_string_is(value, tap_button_maps);
    if (g_str_equal(key, "click-method"))
        return input_string_is(value, click_methods);
    if (g_str_equal(key, "mapping"))
        return input_string_is(value, tablet_mapping);
    if (g_str_equal(key, "eraser-button-mode"))
        return input_string_is(value, stylus_eraser_modes);
    if (g_str_has_suffix(key, "button-action"))
        return input_string_is(value, stylus_actions);
    if (g_str_has_suffix(key, "button-keybinding"))
        return g_variant_is_of_type(value, G_VARIANT_TYPE_STRING);
    static const char* boolean_fields[] = {
        "natural-scroll",
        "repeat",
        "remember-numlock-state",
        "numlock-state",
        "tap-to-click",
        "tap-and-drag",
        "tap-and-drag-lock",
        "disable-while-typing",
        "edge-scrolling-enabled",
        "two-finger-scrolling-enabled",
        "keep-aspect",
        NULL,
    };
    for (guint i = 0; boolean_fields[i]; i++) {
        if (g_str_equal(key, boolean_fields[i]))
            return g_variant_is_of_type(value, G_VARIANT_TYPE_BOOLEAN);
    }
    return FALSE;
}

static gboolean input_fields_valid(const char* group, GVariant* values, GError** error) {
    if (!g_variant_is_of_type(values, G_VARIANT_TYPE_VARDICT)) {
        g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL, "input.%s must be a table", group);
        return FALSE;
    }
    GVariantIter iter;
    const char* key;
    GVariant* value;
    g_variant_iter_init(&iter, values);
    while (g_variant_iter_next(&iter, "{&sv}", &key, &value)) {
        gboolean valid = input_value_valid(group, key, value);
        g_variant_unref(value);
        if (!valid) {
            g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                        "input.%s.%s is unsupported or has an invalid value", group, key);
            return FALSE;
        }
    }
    return TRUE;
}

static gboolean input_group_valid(const char* group, GVariant* values, GError** error) {
    if (!g_variant_is_of_type(values, G_VARIANT_TYPE_VARDICT)) {
        g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL, "input.%s must be a table", group);
        return FALSE;
    }
    gboolean device_group = g_str_equal(group, "tablets") || g_str_equal(group, "styluses");
    if (!device_group)
        return input_fields_valid(group, values, error);
    GVariantIter iter;
    const char* device;
    GVariant* fields;
    g_variant_iter_init(&iter, values);
    while (g_variant_iter_next(&iter, "{&sv}", &device, &fields)) {
        gboolean valid =
            g_regex_match_simple(g_str_equal(group, "tablets")
                                     ? "^[0-9a-fA-F]{4}:[0-9a-fA-F]{4}$"
                                     : "^(?:[0-9a-fA-F]+|default-[0-9a-fA-F]{4}:[0-9a-fA-F]{4})$",
                                 device, G_REGEX_OPTIMIZE, G_REGEX_MATCH_NOTEMPTY);
        if (!valid)
            g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                        "input.%s has an invalid device identifier", group);
        else
            valid = input_fields_valid(group, fields, error);
        g_variant_unref(fields);
        if (!valid)
            return FALSE;
    }
    return TRUE;
}

static gboolean validate_input(GVariant* input, GError** error) {
    if (!g_variant_is_of_type(input, G_VARIANT_TYPE_VARDICT))
        goto invalid_table;
    GVariantIter iter;
    const char* group;
    GVariant* values;
    g_variant_iter_init(&iter, input);
    while (g_variant_iter_next(&iter, "{&sv}", &group, &values)) {
        gboolean valid = TRUE;
        if (g_str_equal(group, "orientation-lock")) {
            valid = g_variant_is_of_type(values, G_VARIANT_TYPE_BOOLEAN) ||
                    (g_variant_is_of_type(values, G_VARIANT_TYPE_STRING) &&
                     g_str_equal(g_variant_get_string(values, NULL), "inherit"));
            if (!valid)
                g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                                    "input.orientation-lock must be a boolean or inherit");
        } else if (g_str_equal(group, "mouse") || g_str_equal(group, "touchpad") ||
                   g_str_equal(group, "keyboard") || g_str_equal(group, "tablets") ||
                   g_str_equal(group, "styluses")) {
            valid = input_group_valid(group, values, error);
        } else {
            g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL, "input.%s is an unsupported group",
                        group);
        }
        g_variant_unref(values);
        if (!valid)
            return FALSE;
    }
    return TRUE;

invalid_table:
    g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL, "input must be a table");
    return FALSE;
}

GVariant* gnoblin_config_normalize_input(GVariant* document, GError** error) {
    g_autoptr(GVariant) input = g_variant_lookup_value(document, "input", NULL);
    if (!input)
        return NULL;
    if (!validate_input(input, error))
        return NULL;
    return gnoblin_native_input_normalize(input);
}

typedef struct {
    char* kind;
    gint64 fingers;
    char* direction;
    GVariant* path;
    char* when;
} TouchpadGestureInput;

static void touchpad_gesture_input_free(gpointer data) {
    TouchpadGestureInput* input = data;
    g_free(input->kind);
    g_free(input->direction);
    g_clear_pointer(&input->path, g_variant_unref);
    g_free(input->when);
    g_free(input);
}

static gboolean variant_integer_in_range(GVariant* value, gint64 minimum, gint64 maximum,
                                         gint64* result) {
    double number;
    if (!input_number(value, &number) || number != floor(number) || number < minimum ||
        number > maximum)
        return FALSE;
    if (result)
        *result = (gint64)number;
    return TRUE;
}

static gboolean variant_string_without_nul(GVariant* value, gboolean nonempty) {
    if (!g_variant_is_of_type(value, G_VARIANT_TYPE_STRING))
        return FALSE;
    gsize length;
    const char* string = g_variant_get_string(value, &length);
    return (!nonempty || length > 0) && !memchr(string, '\0', length);
}

static gboolean window_modifier_valid(const char* modifier) {
    static const char* const modifiers[] = {
        "<Primary>", "<Control>", "<Shift>", "<Shft>", "<Ctrl>", "<Ctl>",   "<Mod1>",  "<Mod2>",
        "<Mod3>",    "<Mod4>",    "<Mod5>",  "<Alt>",  "<Meta>", "<Hyper>", "<Super>", NULL,
    };
    if (g_str_equal(modifier, "disabled") || !modifier[0])
        return TRUE;
    const char* current = modifier;
    while (*current) {
        if (*current != '<')
            return FALSE;
        const char* end = strchr(current, '>');
        if (!end)
            return FALSE;
        gsize length = end - current + 1;
        gboolean known = FALSE;
        for (guint i = 0; modifiers[i]; i++) {
            if (strlen(modifiers[i]) == length &&
                g_ascii_strncasecmp(current, modifiers[i], length) == 0) {
                known = TRUE;
                break;
            }
        }
        if (!known)
            return FALSE;
        current = end + 1;
    }
    return TRUE;
}

static gboolean touchpad_gesture_fields_allowed(GVariant* gesture) {
    static const char* const fields[] = {
        "name",    "gesture", "fingers",   "direction", "path", "action",
        "command", "when",    "threshold", "tolerance", NULL,
    };
    if (!g_variant_is_of_type(gesture, G_VARIANT_TYPE_VARDICT))
        return FALSE;
    GVariantIter iter;
    const char* key;
    GVariant* value;
    g_variant_iter_init(&iter, gesture);
    while (g_variant_iter_next(&iter, "{&sv}", &key, &value)) {
        gboolean known = FALSE;
        for (guint i = 0; fields[i]; i++)
            known |= g_str_equal(key, fields[i]);
        g_variant_unref(value);
        if (!known)
            return FALSE;
    }
    return TRUE;
}

static gboolean touchpad_gesture_path_valid(GVariant* path, const char* name, GError** error) {
    if (!g_variant_is_of_type(path, G_VARIANT_TYPE("av")) || g_variant_n_children(path) < 2 ||
        g_variant_n_children(path) > 16) {
        g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                    "%s: swipe requires a path of 2 to 16 points and no direction", name);
        return FALSE;
    }
    double previous_x = 0;
    double previous_y = 0;
    double last_x = 0;
    double last_y = 0;
    for (gsize i = 0; i < g_variant_n_children(path); i++) {
        g_autoptr(GVariant) boxed = g_variant_get_child_value(path, i);
        g_autoptr(GVariant) point = g_variant_get_variant(boxed);
        if (!g_variant_is_of_type(point, G_VARIANT_TYPE_VARDICT) ||
            g_variant_n_children(point) != 2) {
            g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                        "%s: path points need normalized x and y values from -1 to 1", name);
            return FALSE;
        }
        g_autoptr(GVariant) x_value = g_variant_lookup_value(point, "x", NULL);
        g_autoptr(GVariant) y_value = g_variant_lookup_value(point, "y", NULL);
        double x;
        double y;
        if (!x_value || !y_value || !input_number(x_value, &x) || !input_number(y_value, &y) ||
            fabs(x) > 1 || fabs(y) > 1) {
            g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                        "%s: path points need normalized x and y values from -1 to 1", name);
            return FALSE;
        }
        if ((i == 0 && (x != 0 || y != 0)) || (i > 0 && x == previous_x && y == previous_y)) {
            g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                        "%s: path must start at {x=0, y=0} and describe movement", name);
            return FALSE;
        }
        previous_x = x;
        previous_y = y;
        last_x = x;
        last_y = y;
    }
    if (last_x == 0 && last_y == 0) {
        g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                    "%s: path must start at {x=0, y=0} and describe movement", name);
        return FALSE;
    }
    return TRUE;
}

static gboolean touchpad_gesture_paths_equal(GVariant* first, GVariant* second) {
    if (g_variant_n_children(first) != g_variant_n_children(second))
        return FALSE;
    for (gsize i = 0; i < g_variant_n_children(first); i++) {
        g_autoptr(GVariant) first_boxed = g_variant_get_child_value(first, i);
        g_autoptr(GVariant) second_boxed = g_variant_get_child_value(second, i);
        g_autoptr(GVariant) first_point = g_variant_get_variant(first_boxed);
        g_autoptr(GVariant) second_point = g_variant_get_variant(second_boxed);
        g_autoptr(GVariant) first_x = g_variant_lookup_value(first_point, "x", NULL);
        g_autoptr(GVariant) first_y = g_variant_lookup_value(first_point, "y", NULL);
        g_autoptr(GVariant) second_x = g_variant_lookup_value(second_point, "x", NULL);
        g_autoptr(GVariant) second_y = g_variant_lookup_value(second_point, "y", NULL);
        double first_x_number, first_y_number, second_x_number, second_y_number;
        if (!input_number(first_x, &first_x_number) || !input_number(first_y, &first_y_number) ||
            !input_number(second_x, &second_x_number) ||
            !input_number(second_y, &second_y_number) || first_x_number != second_x_number ||
            first_y_number != second_y_number)
            return FALSE;
    }
    return TRUE;
}

static gboolean touchpad_gesture_input_conflicts(GPtrArray* inputs, const char* kind,
                                                 gint64 fingers, const char* direction,
                                                 GVariant* path, const char* when) {
    for (guint i = 0; i < inputs->len; i++) {
        TouchpadGestureInput* previous = g_ptr_array_index(inputs, i);
        gboolean same_input =
            g_str_equal(previous->kind, kind) && previous->fingers == fingers &&
            ((path && previous->path && touchpad_gesture_paths_equal(previous->path, path)) ||
             (!path && !previous->path && g_str_equal(previous->direction, direction)));
        if (same_input && (g_str_equal(previous->when, "any") || g_str_equal(when, "any") ||
                           g_str_equal(previous->when, when)))
            return TRUE;
    }
    return FALSE;
}

static gboolean validate_touchpad_gestures(GVariant* gestures, GError** error) {
    static const char* const actions[] = {
        "workspace.next",  "workspace.previous",     "window.close",
        "window.minimize", "window.toggle-maximize", NULL,
    };
    if (!g_variant_is_of_type(gestures, G_VARIANT_TYPE("av")) ||
        g_variant_n_children(gestures) > 64) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                            "touchpad-gestures must be a list of at most 64 gesture tables");
        return FALSE;
    }
    g_autoptr(GHashTable) names = g_hash_table_new_full(g_str_hash, g_str_equal, g_free, NULL);
    g_autoptr(GPtrArray) inputs = g_ptr_array_new_with_free_func(touchpad_gesture_input_free);
    for (gsize i = 0; i < g_variant_n_children(gestures); i++) {
        g_autoptr(GVariant) boxed = g_variant_get_child_value(gestures, i);
        g_autoptr(GVariant) gesture = g_variant_get_variant(boxed);
        if (!touchpad_gesture_fields_allowed(gesture)) {
            g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                                "touchpad gesture contains an unknown field");
            return FALSE;
        }
        g_autoptr(GVariant) name_value = g_variant_lookup_value(gesture, "name", NULL);
        g_autoptr(GVariant) kind_value = g_variant_lookup_value(gesture, "gesture", NULL);
        g_autoptr(GVariant) fingers_value = g_variant_lookup_value(gesture, "fingers", NULL);
        g_autoptr(GVariant) direction_value = g_variant_lookup_value(gesture, "direction", NULL);
        g_autoptr(GVariant) path = g_variant_lookup_value(gesture, "path", NULL);
        g_autoptr(GVariant) action_value = g_variant_lookup_value(gesture, "action", NULL);
        g_autoptr(GVariant) command = g_variant_lookup_value(gesture, "command", NULL);
        g_autoptr(GVariant) when_value = g_variant_lookup_value(gesture, "when", NULL);
        g_autoptr(GVariant) threshold_value = g_variant_lookup_value(gesture, "threshold", NULL);
        g_autoptr(GVariant) tolerance_value = g_variant_lookup_value(gesture, "tolerance", NULL);
        const char* name = name_value && g_variant_is_of_type(name_value, G_VARIANT_TYPE_STRING)
                               ? g_variant_get_string(name_value, NULL)
                               : "";
        if (!name_value ||
            !g_regex_match_simple("^[A-Za-z0-9][A-Za-z0-9_-]{0,63}$", name, G_REGEX_OPTIMIZE,
                                  G_REGEX_MATCH_NOTEMPTY) ||
            g_hash_table_contains(names, name)) {
            g_set_error_literal(
                error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                "touchpad gesture names must be unique identifiers of 1 to 64 characters");
            return FALSE;
        }
        g_hash_table_add(names, g_strdup(name));
        const char* kind = kind_value && g_variant_is_of_type(kind_value, G_VARIANT_TYPE_STRING)
                               ? g_variant_get_string(kind_value, NULL)
                               : "";
        if (!g_str_equal(kind, "swipe") && !g_str_equal(kind, "pinch")) {
            g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                        "%s: gesture must be swipe or pinch", name);
            return FALSE;
        }
        gint64 fingers;
        if (!fingers_value || !variant_integer_in_range(fingers_value, 2, 5, &fingers)) {
            g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                        "%s: fingers must be an integer from 2 to 5", name);
            return FALSE;
        }
        const char* direction =
            direction_value && g_variant_is_of_type(direction_value, G_VARIANT_TYPE_STRING)
                ? g_variant_get_string(direction_value, NULL)
                : NULL;
        if (g_str_equal(kind, "swipe")) {
            if (direction_value || !path || !touchpad_gesture_path_valid(path, name, error))
                return FALSE;
        } else if (path || !direction ||
                   (!g_str_equal(direction, "in") && !g_str_equal(direction, "out"))) {
            g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                        "%s: pinch requires direction = in or out and does not use a swipe path",
                        name);
            return FALSE;
        }
        if ((action_value != NULL) == (command != NULL)) {
            g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                        "%s: set exactly one of action or command", name);
            return FALSE;
        }
        const char* action =
            action_value && g_variant_is_of_type(action_value, G_VARIANT_TYPE_STRING)
                ? g_variant_get_string(action_value, NULL)
                : NULL;
        if (action_value && !input_string_is(action_value, actions)) {
            g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                        "%s: unsupported touchpad gesture action", name);
            return FALSE;
        }
        if (command) {
            gboolean valid = g_variant_is_of_type(command, G_VARIANT_TYPE("av")) &&
                             g_variant_n_children(command) > 0;
            for (gsize j = 0; valid && j < g_variant_n_children(command); j++) {
                g_autoptr(GVariant) command_boxed = g_variant_get_child_value(command, j);
                g_autoptr(GVariant) argument = g_variant_get_variant(command_boxed);
                valid = variant_string_without_nul(argument, j == 0);
            }
            if (!valid) {
                g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                            "%s: command must be a nonempty argv array", name);
                return FALSE;
            }
        }
        const char* when = when_value && g_variant_is_of_type(when_value, G_VARIANT_TYPE_STRING)
                               ? g_variant_get_string(when_value, NULL)
                               : "normal";
        if (when_value && (!g_variant_is_of_type(when_value, G_VARIANT_TYPE_STRING) ||
                           (!g_str_equal(when, "normal") && !g_str_equal(when, "unlock-screen") &&
                            !g_str_equal(when, "any")))) {
            g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                        "%s: when must be normal, unlock-screen, or any", name);
            return FALSE;
        }
        double threshold = g_str_equal(kind, "swipe") ? 48 : 0.12;
        if (threshold_value && !input_number(threshold_value, &threshold))
            threshold = NAN;
        if (!isfinite(threshold) ||
            (g_str_equal(kind, "swipe") && (threshold < 16 || threshold > 240)) ||
            (g_str_equal(kind, "pinch") && (threshold < 0.05 || threshold > 0.5))) {
            g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL, "%s: invalid %s threshold", name,
                        kind);
            return FALSE;
        }
        double tolerance = 0.22;
        if (tolerance_value && !input_number(tolerance_value, &tolerance))
            tolerance = NAN;
        if ((g_str_equal(kind, "swipe") &&
             (!isfinite(tolerance) || tolerance < 0.05 || tolerance > 0.5)) ||
            (g_str_equal(kind, "pinch") && tolerance_value)) {
            g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                        "%s: tolerance must be 0.05 to 0.5 and applies only to swipe paths", name);
            return FALSE;
        }
        if (touchpad_gesture_input_conflicts(inputs, kind, fingers, direction, path, when)) {
            g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                        "%s: another gesture already claims this input in an overlapping context",
                        name);
            return FALSE;
        }
        TouchpadGestureInput* input = g_new0(TouchpadGestureInput, 1);
        input->kind = g_strdup(kind);
        input->fingers = fingers;
        input->direction = g_strdup(direction);
        input->path = path ? g_variant_ref(path) : NULL;
        input->when = g_strdup(when);
        g_ptr_array_add(inputs, input);
    }
    return TRUE;
}

static gboolean validate_window_rule_padding(GVariant* section, gsize rule_index,
                                             const char* section_name, GError** error) {
    g_autoptr(GVariant) padding = g_variant_lookup_value(section, "padding", NULL);
    if (!padding)
        return TRUE;
    if (!g_variant_is_of_type(padding, G_VARIANT_TYPE("av")) ||
        g_variant_n_children(padding) != 4) {
        g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                    "window-rules[%zu].%s.padding must contain four numbers", rule_index,
                    section_name);
        return FALSE;
    }

    for (gsize i = 0; i < 4; i++) {
        g_autoptr(GVariant) boxed = g_variant_get_child_value(padding, i);
        g_autoptr(GVariant) value = g_variant_get_variant(boxed);
        double number;
        if (!input_number(value, &number)) {
            g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                        "window-rules[%zu].%s.padding[%zu] must be a finite number", rule_index,
                        section_name, i + 1);
            return FALSE;
        }
        if (number < -128.0 || number > 128.0) {
            g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                        "window-rules[%zu].%s.padding[%zu] must be finite and from -128 to 128",
                        rule_index, section_name, i + 1);
            return FALSE;
        }
    }
    return TRUE;
}

static gboolean validate_window_rule_patterns(GVariant* document, GError** error) {
    g_autoptr(GVariant) rules = g_variant_lookup_value(document, "window-rules", NULL);
    if (!rules)
        return TRUE;
    if (!g_variant_is_of_type(rules, G_VARIANT_TYPE("av"))) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                            "window-rules must be an array of rule tables");
        return FALSE;
    }

    static const char* const fields[] = {"app-id", "title", "layer", NULL};
    for (gsize i = 0; i < g_variant_n_children(rules); i++) {
        g_autoptr(GVariant) boxed_rule = g_variant_get_child_value(rules, i);
        g_autoptr(GVariant) rule = g_variant_get_variant(boxed_rule);
        if (!g_variant_is_of_type(rule, G_VARIANT_TYPE_VARDICT))
            continue;
        g_autoptr(GVariant) legacy_borders = g_variant_lookup_value(rule, "borders", NULL);
        if (legacy_borders) {
            g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                        "window-rules[%zu].borders is not supported in standalone Gnoblin; "
                        "use corners.border_width and corners.border_color for one native "
                        "outline",
                        i + 1);
            return FALSE;
        }
        g_autoptr(GVariant) match = g_variant_lookup_value(rule, "match", NULL);
        if (!match || !g_variant_is_of_type(match, G_VARIANT_TYPE_VARDICT))
            continue;

        for (guint field_index = 0; fields[field_index]; field_index++) {
            g_autoptr(GVariant) pattern_value =
                g_variant_lookup_value(match, fields[field_index], NULL);
            if (!pattern_value)
                continue;
            if (!g_variant_is_of_type(pattern_value, G_VARIANT_TYPE_STRING)) {
                g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                            "window-rules[%zu].match.%s must be a string Lua pattern", i + 1,
                            fields[field_index]);
                return FALSE;
            }
            const char* pattern = g_variant_get_string(pattern_value, NULL);
            g_autoptr(GError) pattern_error = NULL;
            gnoblin_config_window_pattern_match(pattern, "", &pattern_error);
            if (pattern_error) {
                g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                            "window-rules[%zu].match.%s: %s", i + 1, fields[field_index],
                            pattern_error->message);
                return FALSE;
            }
        }

        static const char* const effect_sections[] = {"corners", NULL};
        for (guint section_index = 0; effect_sections[section_index]; section_index++) {
            g_autoptr(GVariant) section =
                g_variant_lookup_value(rule, effect_sections[section_index], NULL);
            if (section && g_variant_is_of_type(section, G_VARIANT_TYPE_VARDICT) &&
                !validate_window_rule_padding(section, i + 1, effect_sections[section_index],
                                              error))
                return FALSE;
        }
    }
    return TRUE;
}

static gboolean portal_backend_list_valid(GVariant* backends, const char* field, GError** error) {
    if (!g_variant_is_of_type(backends, G_VARIANT_TYPE("av")) &&
        !g_variant_is_of_type(backends, G_VARIANT_TYPE_STRING_ARRAY)) {
        g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                    "portals.%s must be a nonempty array of backend names", field);
        return FALSE;
    }

    gsize count = g_variant_n_children(backends);
    gboolean has_none = FALSE;
    if (count == 0 || count > 32) {
        g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                    "portals.%s must contain between 1 and 32 backend names", field);
        return FALSE;
    }

    for (gsize i = 0; i < count; i++) {
        g_autoptr(GVariant) child = g_variant_get_child_value(backends, i);
        g_autoptr(GVariant) item = g_variant_is_of_type(child, G_VARIANT_TYPE_VARIANT)
                                       ? g_variant_get_variant(child)
                                       : g_variant_ref(child);
        if (!g_variant_is_of_type(item, G_VARIANT_TYPE_STRING)) {
            g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                        "portals.%s[%zu] must be a backend name", field, i + 1);
            return FALSE;
        }
        const char* name = g_variant_get_string(item, NULL);
        gboolean valid = g_str_equal(name, "*") || g_str_equal(name, "none") ||
                         g_regex_match_simple("^[A-Za-z0-9][A-Za-z0-9_-]{0,63}$", name,
                                              G_REGEX_OPTIMIZE, G_REGEX_MATCH_NOTEMPTY);
        if (!valid) {
            g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                        "portals.%s[%zu] is not a valid portal backend name", field, i + 1);
            return FALSE;
        }
        has_none |= g_str_equal(name, "none");
    }

    if (has_none && count != 1) {
        g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                    "portals.%s may use 'none' only by itself", field);
        return FALSE;
    }
    return TRUE;
}

static gboolean validate_portal_selection(GVariant* portals, GError** error) {
    if (!g_variant_is_of_type(portals, G_VARIANT_TYPE_VARDICT)) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                            "portals must be a table with default and optional interfaces");
        return FALSE;
    }

    gboolean has_default = FALSE;
    g_autoptr(GVariant) interfaces = NULL;
    GVariantIter iter;
    const char* key;
    GVariant* value;
    g_variant_iter_init(&iter, portals);
    while (g_variant_iter_next(&iter, "{&sv}", &key, &value)) {
        g_autoptr(GVariant) field = value;
        if (g_str_equal(key, "default")) {
            has_default = TRUE;
            if (!portal_backend_list_valid(field, "default", error))
                return FALSE;
        } else if (g_str_equal(key, "interfaces")) {
            if (!g_variant_is_of_type(field, G_VARIANT_TYPE_VARDICT)) {
                g_set_error_literal(
                    error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                    "portals.interfaces must map portal interface names to backend arrays");
                return FALSE;
            }
            interfaces = g_variant_ref(field);
        } else {
            g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                        "portals.%s is not a supported field", key);
            return FALSE;
        }
    }

    if (!has_default) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                            "portals.default must list one or more portal backends");
        return FALSE;
    }

    if (interfaces) {
        GVariantIter interface_iter;
        const char* interface_name;
        GVariant* route;
        g_variant_iter_init(&interface_iter, interfaces);
        while (g_variant_iter_next(&interface_iter, "{&sv}", &interface_name, &route)) {
            g_autoptr(GVariant) route_value = route;
            if (!g_dbus_is_interface_name(interface_name)) {
                g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                            "portals.interfaces key '%s' is not a D-Bus interface name",
                            interface_name);
                return FALSE;
            }
            g_autofree char* field_name = g_strdup_printf("interfaces.%s", interface_name);
            if (!portal_backend_list_valid(route_value, field_name, error))
                return FALSE;
        }
    }
    return TRUE;
}

static const char portal_config_marker[] =
    "# Generated by Gnoblin from gnoblin.configure.portals. Do not edit.\n";

static GVariant* portal_backend_at(GVariant* backends, gsize index) {
    GVariant* child = g_variant_get_child_value(backends, index);
    if (g_variant_is_of_type(child, G_VARIANT_TYPE_VARIANT)) {
        GVariant* value = g_variant_get_variant(child);
        g_variant_unref(child);
        return value;
    }
    return child;
}

static void append_portal_backends(GString* output, const char* interface, GVariant* backends) {
    g_string_append_printf(output, "%s=", interface);
    for (gsize i = 0; i < g_variant_n_children(backends); i++) {
        g_autoptr(GVariant) backend = portal_backend_at(backends, i);
        if (i > 0)
            g_string_append_c(output, ';');
        g_string_append(output, g_variant_get_string(backend, NULL));
    }
    g_string_append(output, ";\n");
}

static gint portal_interface_compare(gconstpointer left, gconstpointer right) {
    return g_strcmp0(*(char* const*)left, *(char* const*)right);
}

static char* render_portal_selection(GVariant* document) {
    g_autoptr(GVariant) portals =
        g_variant_lookup_value(document, "portals", G_VARIANT_TYPE_VARDICT);
    if (!portals)
        return NULL;

    g_autoptr(GVariant) defaults = g_variant_lookup_value(portals, "default", G_VARIANT_TYPE("av"));
    if (!defaults)
        defaults = g_variant_lookup_value(portals, "default", G_VARIANT_TYPE_STRING_ARRAY);
    g_autoptr(GVariant) interfaces =
        g_variant_lookup_value(portals, "interfaces", G_VARIANT_TYPE_VARDICT);
    GString* output = g_string_new(portal_config_marker);
    g_string_append(output, "[preferred]\n");
    append_portal_backends(output, "default", defaults);

    if (interfaces) {
        GPtrArray* names = g_ptr_array_new_with_free_func(g_free);
        GVariantIter iter;
        const char* name;
        GVariant* route;
        g_variant_iter_init(&iter, interfaces);
        while (g_variant_iter_next(&iter, "{&sv}", &name, &route)) {
            g_ptr_array_add(names, g_strdup(name));
            g_variant_unref(route);
        }
        g_ptr_array_sort(names, portal_interface_compare);
        for (guint i = 0; i < names->len; i++) {
            const char* interface = g_ptr_array_index(names, i);
            g_autoptr(GVariant) route =
                g_variant_lookup_value(interfaces, interface, G_VARIANT_TYPE("av"));
            if (!route)
                route = g_variant_lookup_value(interfaces, interface, G_VARIANT_TYPE_STRING_ARRAY);
            append_portal_backends(output, interface, route);
        }
        g_ptr_array_unref(names);
    }

    return g_string_free(output, FALSE);
}

static gboolean portal_config_is_managed(const char* contents) {
    return contents && g_str_has_prefix(contents, portal_config_marker);
}

gboolean gnoblin_config_sync_portal_selection(GVariant* document, const char* config_home,
                                              GError** error) {
    if (!document || !g_variant_is_of_type(document, G_VARIANT_TYPE_VARDICT)) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                            "portal preferences need a configuration table");
        return FALSE;
    }
    if (!gnoblin_config_validate_document(document, error))
        return FALSE;

    const char* home = config_home && *config_home ? config_home : g_get_user_config_dir();
    g_autofree char* directory = g_build_filename(home, "xdg-desktop-portal", NULL);
    g_autofree char* path = g_build_filename(directory, "gnoblin-portals.conf", NULL);
    g_autofree char* existing = NULL;
    gboolean has_existing = g_file_test(path, G_FILE_TEST_EXISTS);

    if (has_existing && !g_file_get_contents(path, &existing, NULL, error))
        return FALSE;

    g_autoptr(GVariant) portals =
        g_variant_lookup_value(document, "portals", G_VARIANT_TYPE_VARDICT);
    if (!portals) {
        if (!has_existing || !portal_config_is_managed(existing))
            return TRUE;
        if (g_unlink(path) == 0 || errno == ENOENT)
            return TRUE;
        g_set_error(error, G_FILE_ERROR, g_file_error_from_errno(errno),
                    "could not remove generated portal preferences %s: %s", path,
                    g_strerror(errno));
        return FALSE;
    }

    if (has_existing && !portal_config_is_managed(existing)) {
        g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_EXIST,
                    "user portal preferences at %s take precedence; move or remove that file to "
                    "use gnoblin.configure.portals from your Lua config",
                    path);
        return FALSE;
    }

    g_autofree char* contents = render_portal_selection(document);
    if (!contents) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                            "could not render portal preferences from Gnoblin configuration");
        return FALSE;
    }
    if (g_strcmp0(existing, contents) == 0)
        return TRUE;
    if (g_mkdir_with_parents(directory, 0700) != 0) {
        g_set_error(error, G_FILE_ERROR, g_file_error_from_errno(errno),
                    "could not create portal preferences directory %s: %s", directory,
                    g_strerror(errno));
        return FALSE;
    }
    if (!g_file_set_contents_full(path, contents, -1, G_FILE_SET_CONTENTS_CONSISTENT, 0600, error))
        return FALSE;
    return TRUE;
}

gboolean gnoblin_config_validate_document(GVariant* document, GError** error) {
    g_autoptr(GVariant) shell = g_variant_lookup_value(document, "shell", NULL);
    if (shell) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                            "shell settings are no longer supported; configure shell UI in the "
                            "shell client, and use gnoblin.animation and window rules for "
                            "compositor animations");
        return FALSE;
    }
    if (!gnoblin_permission_policy_validate(document, error))
        return FALSE;
    g_autoptr(GVariant) portals = g_variant_lookup_value(document, "portals", NULL);
    if (portals && !validate_portal_selection(portals, error))
        return FALSE;
    if (!validate_window_rule_patterns(document, error))
        return FALSE;
    g_autoptr(GVariant) input = g_variant_lookup_value(document, "input", NULL);
    if (input && !validate_input(input, error))
        return FALSE;
    g_autoptr(GVariant) touchpad_gestures =
        g_variant_lookup_value(document, "touchpad-gestures", NULL);
    if (touchpad_gestures && !validate_touchpad_gestures(touchpad_gestures, error))
        return FALSE;
    g_autoptr(GVariant) workspaces = g_variant_lookup_value(document, "workspaces", NULL);
    if (workspaces) {
        gboolean valid = g_variant_is_of_type(workspaces, G_VARIANT_TYPE("av")) &&
                         g_variant_n_children(workspaces) >= 1;
        GHashTable* ids = g_hash_table_new_full(g_str_hash, g_str_equal, g_free, NULL);
        for (gsize i = 0; valid && i < g_variant_n_children(workspaces); i++) {
            g_autoptr(GVariant) boxed = g_variant_get_child_value(workspaces, i);
            g_autoptr(GVariant) entry = g_variant_get_variant(boxed);
            if (!g_variant_is_of_type(entry, G_VARIANT_TYPE_VARDICT) ||
                g_variant_n_children(entry) != 2) {
                valid = FALSE;
                break;
            }
            g_autoptr(GVariant) id_value = g_variant_lookup_value(entry, "id", NULL);
            g_autoptr(GVariant) name_value = g_variant_lookup_value(entry, "name", NULL);
            const char* id = id_value && g_variant_is_of_type(id_value, G_VARIANT_TYPE_STRING)
                                 ? g_variant_get_string(id_value, NULL)
                                 : "";
            const char* name = name_value && g_variant_is_of_type(name_value, G_VARIANT_TYPE_STRING)
                                   ? g_variant_get_string(name_value, NULL)
                                   : "";
            valid = id_value && name_value &&
                    g_regex_match_simple("^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$", id, G_REGEX_OPTIMIZE,
                                         G_REGEX_MATCH_NOTEMPTY) &&
                    *name && g_utf8_validate(name, -1, NULL) && g_utf8_strlen(name, -1) <= 80 &&
                    !g_hash_table_contains(ids, id);
            if (valid)
                g_hash_table_add(ids, g_strdup(id));
        }
        g_hash_table_unref(ids);
        if (!valid) {
            g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                                "workspaces must be a nonempty array of objects with unique valid "
                                "ids and nonempty names up to 80 characters");
            return FALSE;
        }
    }
    const char* sections[] = {"protocols", "layer-shell"};
    const char* keys[] = {NULL, "preserve-active-window"};
    for (guint i = 0; i < G_N_ELEMENTS(sections); i++) {
        g_autoptr(GVariant) section = g_variant_lookup_value(document, sections[i], NULL);
        if (!section)
            continue;
        gboolean valid = g_variant_is_of_type(section, G_VARIANT_TYPE_VARDICT);
        GVariantIter iter;
        const char* name;
        GVariant* value;
        if (valid)
            g_variant_iter_init(&iter, section);
        while (valid && g_variant_iter_next(&iter, "{&sv}", &name, &value)) {
            valid = g_variant_is_of_type(value, G_VARIANT_TYPE_BOOLEAN) &&
                    (!keys[i] || g_str_equal(name, keys[i]));
            g_variant_unref(value);
        }
        if (!valid) {
            g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                        "%s must be a table of supported boolean settings", sections[i]);
            return FALSE;
        }
    }
    g_autoptr(GVariant) window = g_variant_lookup_value(document, "window-management", NULL);
    if (window) {
        if (g_variant_is_of_type(window, G_VARIANT_TYPE_VARDICT)) {
            const char* removed_workspace_fields[] = {"workspace-ids", "workspace-names",
                                                      "num-workspaces", NULL};
            for (guint i = 0; removed_workspace_fields[i]; i++) {
                g_autoptr(GVariant) legacy =
                    g_variant_lookup_value(window, removed_workspace_fields[i], NULL);
                if (legacy) {
                    g_set_error_literal(
                        error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                        "window-management workspace-ids, workspace-names, and num-workspaces "
                        "are no longer supported; declare named workspaces in the top-level "
                        "workspaces field");
                    return FALSE;
                }
            }
            g_autoptr(GVariant) dynamic_workspaces =
                g_variant_lookup_value(window, "dynamic-workspaces", NULL);
            if (workspaces && dynamic_workspaces) {
                g_set_error_literal(
                    error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                    "top-level workspaces cannot be combined with window-management "
                    "dynamic-workspaces");
                return FALSE;
            }
        }
        static const char* booleans[] = {
            "constrain-drag-to-work-area",
            "auto-maximize",
            "raise-on-click",
            "auto-raise",
            "focus-change-on-pointer-rest",
            "dynamic-workspaces",
            "workspaces-only-on-primary",
            "edge-tiling",
            "center-new-windows",
            "attach-modal-dialogs",
            NULL,
        };
        static const char* titlebar[] = {
            "toggle-maximize",
            "toggle-maximize-horizontally",
            "toggle-maximize-vertically",
            "minimize",
            "none",
            "lower",
            "menu",
            NULL,
        };
        gboolean valid = g_variant_is_of_type(window, G_VARIANT_TYPE_VARDICT);
        GVariantIter iter;
        const char* name;
        GVariant* value;
        if (valid)
            g_variant_iter_init(&iter, window);
        while (valid && g_variant_iter_next(&iter, "{&sv}", &name, &value)) {
            gboolean known_boolean = FALSE;
            for (guint i = 0; booleans[i]; i++)
                known_boolean |= g_str_equal(name, booleans[i]);
            if (known_boolean) {
                valid = g_variant_is_of_type(value, G_VARIANT_TYPE_BOOLEAN);
            } else if (g_str_equal(name, "auto-raise-delay")) {
                gint64 number =
                    g_variant_is_of_type(value, G_VARIANT_TYPE_INT32)   ? g_variant_get_int32(value)
                    : g_variant_is_of_type(value, G_VARIANT_TYPE_INT64) ? g_variant_get_int64(value)
                                                                        : -1;
                valid = number >= 0 && number <= 10000;
            } else if (g_variant_is_of_type(value, G_VARIANT_TYPE_STRING)) {
                const char* string = g_variant_get_string(value, NULL);
                if (g_str_equal(name, "focus-mode"))
                    valid = g_str_equal(string, "click") || g_str_equal(string, "sloppy") ||
                            g_str_equal(string, "mouse");
                else if (g_str_equal(name, "focus-new-windows"))
                    valid = g_str_equal(string, "smart") || g_str_equal(string, "strict");
                else if (g_str_equal(name, "action-double-click-titlebar") ||
                         g_str_equal(name, "action-middle-click-titlebar") ||
                         g_str_equal(name, "action-right-click-titlebar")) {
                    valid = FALSE;
                    for (guint i = 0; titlebar[i]; i++)
                        valid |= g_str_equal(string, titlebar[i]);
                } else if (g_str_equal(name, "mouse-button-modifier")) {
                    valid = window_modifier_valid(string);
                } else
                    valid = FALSE;
            } else
                valid = FALSE;
            g_variant_unref(value);
        }
        if (!valid) {
            g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                                "window-management contains an unsupported name or value");
            return FALSE;
        }
    }
    g_autoptr(GVariant) compositor = g_variant_lookup_value(document, "compositor", NULL);
    if (compositor) {
        static const char* booleans[] = {
            "enable-animations", "locate-pointer", "visual-bell", "audible-bell", NULL,
        };
        gboolean valid = g_variant_is_of_type(compositor, G_VARIANT_TYPE_VARDICT);
        GVariantIter iter;
        const char* name;
        GVariant* value;
        if (valid)
            g_variant_iter_init(&iter, compositor);
        while (valid && g_variant_iter_next(&iter, "{&sv}", &name, &value)) {
            gboolean known_boolean = FALSE;
            for (guint i = 0; booleans[i]; i++)
                known_boolean |= g_str_equal(name, booleans[i]);
            if (known_boolean)
                valid = g_variant_is_of_type(value, G_VARIANT_TYPE_BOOLEAN);
            else if (g_str_equal(name, "visual-bell-type") &&
                     g_variant_is_of_type(value, G_VARIANT_TYPE_STRING)) {
                const char* bell = g_variant_get_string(value, NULL);
                valid = g_str_equal(bell, "fullscreen-flash") || g_str_equal(bell, "frame-flash");
            } else
                valid = FALSE;
            g_variant_unref(value);
        }
        if (!valid) {
            g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                                "compositor contains an unsupported name or value");
            return FALSE;
        }
    }
    g_autoptr(GVariant) cursor = g_variant_lookup_value(document, "cursor", NULL);
    if (cursor) {
        gboolean valid = g_variant_is_of_type(cursor, G_VARIANT_TYPE_VARDICT);
        GVariantIter iter;
        const char* name;
        GVariant* value;
        if (valid)
            g_variant_iter_init(&iter, cursor);
        while (valid && g_variant_iter_next(&iter, "{&sv}", &name, &value)) {
            if (g_str_equal(name, "theme")) {
                if (g_variant_is_of_type(value, G_VARIANT_TYPE_STRING)) {
                    g_autofree char* theme = g_variant_dup_string(value, NULL);
                    valid = *g_strstrip(theme) != '\0';
                } else {
                    valid = FALSE;
                }
            } else if (g_str_equal(name, "size")) {
                gint64 size =
                    g_variant_is_of_type(value, G_VARIANT_TYPE_INT32)   ? g_variant_get_int32(value)
                    : g_variant_is_of_type(value, G_VARIANT_TYPE_INT64) ? g_variant_get_int64(value)
                                                                        : 0;
                valid = size >= 1 && size <= 256 &&
                        (g_variant_is_of_type(value, G_VARIANT_TYPE_INT32) ||
                         g_variant_is_of_type(value, G_VARIANT_TYPE_INT64));
            } else {
                valid = FALSE;
            }
            g_variant_unref(value);
        }
        if (!valid) {
            g_set_error_literal(
                error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                "cursor must contain only a nonempty theme and a size from 1 to 256");
            return FALSE;
        }
    }
    return TRUE;
}

GVariant* gnoblin_config_load_document(const char* path, GPtrArray** paths, GPtrArray** directories,
                                       GError** error) {
    g_autoptr(GPtrArray) loaded_paths = g_ptr_array_new_with_free_func(g_free);
    g_autoptr(GPtrArray) watched_dirs = g_ptr_array_new_with_free_func(g_free);
    g_autofree char* canonical = g_canonicalize_filename(path, NULL);
    GVariant* document = NULL;
    g_ptr_array_add(loaded_paths, g_strdup(canonical));

    if (!g_str_has_suffix(canonical, ".lua")) {
        g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                    "%s: Gnoblin configuration uses Lua only. Convert this file to Lua and "
                    "give it a .lua filename.",
                    canonical);
    } else if (!g_file_test(canonical, G_FILE_TEST_EXISTS)) {
        GVariantBuilder empty;
        g_variant_builder_init(&empty, G_VARIANT_TYPE_VARDICT);
        document = g_variant_ref_sink(g_variant_builder_end(&empty));
    } else {
        document = gnoblin_config_evaluate_file(canonical, loaded_paths, watched_dirs, error);
        if (document)
            g_variant_ref_sink(document);
        if (document && !g_variant_is_of_type(document, G_VARIANT_TYPE_VARDICT)) {
            g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                                "configuration must be a table of settings");
            g_clear_pointer(&document, g_variant_unref);
        }
        if (document && !gnoblin_config_validate_document(document, error))
            g_clear_pointer(&document, g_variant_unref);
    }
    if (paths)
        *paths = g_steal_pointer(&loaded_paths);
    if (directories)
        *directories = g_steal_pointer(&watched_dirs);
    return document;
}

char* gnoblin_config_path(void) {
    const char* override = g_getenv("GNOBLIN_CONFIG");
    if (override && override[0])
        return g_canonicalize_filename(override, NULL);
    g_autofree char* directory = g_build_filename(g_get_user_config_dir(), "gnoblin", NULL);
    /* Detect old configs instead of silently shadowing them with a new default. */
    const char* names[] = {"init.lua", "gnoblin.toml", "gnoblin.conf"};
    for (guint i = 0; i < G_N_ELEMENTS(names); i++) {
        g_autofree char* candidate = g_build_filename(directory, names[i], NULL);
        if (g_file_test(candidate, G_FILE_TEST_EXISTS))
            return g_steal_pointer(&candidate);
    }
    return g_build_filename(directory, "init.lua", NULL);
}
