#include "gnoblin-config-internal.h"

gboolean input_number(GVariant* value, double* number) {
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
    /* Mutter stores the keyboard delay and repeat interval as unsigned 32-bit integers, so the
     * accepted state that a replacement worker validates holds them in this type. */
    if (g_variant_is_of_type(value, G_VARIANT_TYPE_UINT32)) {
        *number = (double)g_variant_get_uint32(value);
        return TRUE;
    }
    return FALSE;
}

static gboolean xwayland_string_array_valid(GVariant* value, gboolean extensions) {
    if (!g_variant_is_of_type(value, G_VARIANT_TYPE("av")))
        return FALSE;

    GVariantIter iter;
    GVariant* boxed;
    g_variant_iter_init(&iter, value);
    while ((boxed = g_variant_iter_next_value(&iter))) {
        g_autoptr(GVariant) item = g_variant_get_variant(boxed);
        gboolean valid = g_variant_is_of_type(item, G_VARIANT_TYPE_STRING);
        if (valid && extensions) {
            const char* name = g_variant_get_string(item, NULL);
            valid = g_str_equal(name, "security") || g_str_equal(name, "xtest");
        }
        g_variant_unref(boxed);
        if (!valid)
            return FALSE;
    }
    return TRUE;
}

gboolean validate_xwayland(GVariant* xwayland, GError** error) {
    gboolean valid = g_variant_is_of_type(xwayland, G_VARIANT_TYPE_VARDICT);
    GVariantIter iter;
    const char* name = NULL;
    GVariant* value;
    if (valid)
        g_variant_iter_init(&iter, xwayland);

    while (valid && g_variant_iter_next(&iter, "{&sv}", &name, &value)) {
        if (g_str_equal(name, "allow-grabs") || g_str_equal(name, "allow-byte-swapped-clients")) {
            valid = g_variant_is_of_type(value, G_VARIANT_TYPE_BOOLEAN);
        } else if (g_str_equal(name, "grab-access-rules")) {
            valid = xwayland_string_array_valid(value, FALSE);
        } else if (g_str_equal(name, "disable-extensions")) {
            valid = xwayland_string_array_valid(value, TRUE);
        } else if (g_str_equal(name, "scaling-factor")) {
            double number;
            const double maximum = (double)nextafterf((float)G_MAXINT, 0.0f);
            valid = input_number(value, &number) &&
                    (number == 0 || (number >= 0.5 && number <= maximum));
        } else {
            valid = FALSE;
        }
        g_variant_unref(value);
    }

    if (!valid) {
        g_set_error(
            error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
            "xwayland%s%s%s has an unsupported name or value; scaling-factor must be 0 or a "
            "finite value from 0.5 through the largest safe signed-integer scale",
            name ? " \"" : "", name ? name : "", name ? "\"" : "");
        return FALSE;
    }
    return TRUE;
}

gboolean input_string_is(GVariant* value, const char* const* choices) {
    if (!g_variant_is_of_type(value, G_VARIANT_TYPE_STRING))
        return FALSE;
    const char* string = g_variant_get_string(value, NULL);
    for (guint i = 0; choices[i]; i++) {
        if (g_str_equal(string, choices[i]))
            return TRUE;
    }
    return FALSE;
}

static gboolean tablet_pad_buttons_valid(GVariant* value) {
    static const char* actions[] = {"default",        "none",       "help",
                                    "switch-monitor", "keybinding", NULL};
    if (!g_variant_is_of_type(value, G_VARIANT_TYPE("av")) || g_variant_n_children(value) > 256)
        return FALSE;

    gboolean seen[256] = {FALSE};
    for (gsize i = 0; i < g_variant_n_children(value); i++) {
        g_autoptr(GVariant) boxed = g_variant_get_child_value(value, i);
        g_autoptr(GVariant) button = g_variant_get_variant(boxed);
        if (!g_variant_is_of_type(button, G_VARIANT_TYPE_VARDICT))
            return FALSE;

        g_autoptr(GVariant) button_value = g_variant_lookup_value(button, "button", NULL);
        g_autoptr(GVariant) action_value = g_variant_lookup_value(button, "action", NULL);
        g_autoptr(GVariant) keybinding_value = g_variant_lookup_value(button, "keybinding", NULL);
        double button_number;
        if (!button_value || !input_number(button_value, &button_number) || button_number < 0 ||
            button_number > 255 || button_number != floor(button_number) || !action_value ||
            !input_string_is(action_value, actions))
            return FALSE;

        guint button_id = (guint)button_number;
        if (seen[button_id])
            return FALSE;
        seen[button_id] = TRUE;

        const char* action = g_variant_get_string(action_value, NULL);
        if (g_str_equal(action, "keybinding")) {
            if (!keybinding_value || !g_variant_is_of_type(keybinding_value, G_VARIANT_TYPE_STRING))
                return FALSE;
            const char* keybinding = g_variant_get_string(keybinding_value, NULL);
            if (!*keybinding || strlen(keybinding) > 160)
                return FALSE;
        } else if (keybinding_value) {
            return FALSE;
        }

        GVariantIter fields;
        const char* name;
        GVariant* field_value;
        g_variant_iter_init(&fields, button);
        while (g_variant_iter_next(&fields, "{&sv}", &name, &field_value)) {
            gboolean known = g_str_equal(name, "button") || g_str_equal(name, "action") ||
                             g_str_equal(name, "keybinding");
            g_variant_unref(field_value);
            if (!known)
                return FALSE;
        }
    }
    return TRUE;
}

static gboolean tablet_area_valid(GVariant* value) {
    if (!g_variant_is_of_type(value, G_VARIANT_TYPE("av")) || g_variant_n_children(value) != 4)
        return FALSE;

    double area[4];
    for (gsize i = 0; i < G_N_ELEMENTS(area); i++) {
        g_autoptr(GVariant) boxed = g_variant_get_child_value(value, i);
        g_autoptr(GVariant) item = g_variant_get_variant(boxed);
        if (!input_number(item, &area[i]) || area[i] < 0 || area[i] >= 1)
            return FALSE;
    }

    return area[0] + area[1] < 1 && area[2] + area[3] < 1;
}

static gboolean input_field_known(const char* group, const char* key) {
    static const char* const mouse_fields[] = {
        "speed",         "double-click-time", "drag-threshold",
        "left-handed",   "natural-scroll",    "middle-click-emulation",
        "accel-profile", "accel-curve",       NULL,
    };
    static const char* const touchpad_fields[] = {
        "speed",
        "scroll-speed",
        "left-handed",
        "natural-scroll",
        "middle-click-emulation",
        "accel-profile",
        "accel-curve",
        "tap-to-click",
        "tap-button-map",
        "tap-and-drag",
        "tap-and-drag-lock",
        "disable-while-typing",
        "disable-while-typing-timeout",
        "edge-scrolling-enabled",
        "two-finger-scrolling-enabled",
        "click-method",
        NULL,
    };
    static const char* const trackball_fields[] = {
        "accel-profile",
        "middle-click-emulation",
        "scroll-wheel-emulation-button",
        "scroll-wheel-emulation-button-lock",
        NULL,
    };
    static const char* const pointing_stick_fields[] = {
        "speed",
        "accel-profile",
        "scroll-method",
        NULL,
    };
    static const char* const keyboard_fields[] = {
        "repeat",        "delay",       "repeat-interval", "remember-numlock-state",
        "numlock-state", "xkb-options", "accessibility",   NULL,
    };
    static const char* const tablet_fields[] = {
        "mapping", "output", "left-handed", "keep-aspect", "area", "pad-buttons", NULL,
    };
    static const char* const stylus_fields[] = {
        "eraser-button-mode",         "eraser-button-action",
        "eraser-button-keybinding",   "button-action",
        "secondary-button-action",    "tertiary-button-action",
        "button-keybinding",          "secondary-button-keybinding",
        "tertiary-button-keybinding", NULL,
    };
    const char* const* fields = g_str_equal(group, "mouse")            ? mouse_fields
                                : g_str_equal(group, "touchpad")       ? touchpad_fields
                                : g_str_equal(group, "trackball")      ? trackball_fields
                                : g_str_equal(group, "pointing-stick") ? pointing_stick_fields
                                : g_str_equal(group, "keyboard")       ? keyboard_fields
                                : g_str_equal(group, "tablets")        ? tablet_fields
                                : g_str_equal(group, "styluses")       ? stylus_fields
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
    static const char* handedness[] = {"right", "left", "follow-mouse", "mouse", NULL};
    static const char* tap_button_maps[] = {
        "default", "left-right-middle", "left-middle-right", "lrm", "lmr", NULL};
    static const char* click_methods[] = {"default", "none", "areas", "fingers", NULL};
    static const char* trackball_accel_profiles[] = {"default", "flat", "adaptive", NULL};
    static const char* pointing_stick_scroll_methods[] = {"default", "none", "on-button-down",
                                                          NULL};
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
    if (g_str_equal(group, "touchpad") && g_str_equal(key, "disable-while-typing-timeout")) {
        double number;
        return input_number(value, &number) && number >= 100 && number <= 5000 &&
               number == floor(number);
    }
    if (g_str_equal(key, "delay") || g_str_equal(key, "repeat-interval")) {
        double number;
        return input_number(value, &number) && number >= 1 && number <= 10000 &&
               number == (gint64)number;
    }
    if (g_str_equal(key, "drag-threshold")) {
        double number;
        return input_number(value, &number) && number >= 1 && number <= G_MAXINT &&
               number == floor(number);
    }
    if (g_str_equal(key, "double-click-time")) {
        double number;
        return input_number(value, &number) && number >= 0 && number <= G_MAXINT &&
               number == floor(number);
    }
    if (g_str_equal(key, "scroll-wheel-emulation-button")) {
        double number;
        return input_number(value, &number) && number >= 0 && number <= 24 &&
               number == floor(number);
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
    if (g_str_equal(key, "accel-profile")) {
        if (g_str_equal(group, "trackball") || g_str_equal(group, "pointing-stick"))
            return input_string_is(value, trackball_accel_profiles);
        return input_string_is(value, accel_profiles);
    }
    if (g_str_equal(group, "pointing-stick") && g_str_equal(key, "scroll-method"))
        return input_string_is(value, pointing_stick_scroll_methods);
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
    if (g_str_equal(group, "tablets") && g_str_equal(key, "output")) {
        if (!g_variant_is_of_type(value, G_VARIANT_TYPE_STRING))
            return FALSE;
        const char* output = g_variant_get_string(value, NULL);
        return g_str_equal(output, "auto") ||
               g_regex_match_simple("^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$", output, G_REGEX_OPTIMIZE,
                                    0);
    }
    if (g_str_equal(group, "tablets") && g_str_equal(key, "pad-buttons"))
        return tablet_pad_buttons_valid(value);
    if (g_str_equal(group, "tablets") && g_str_equal(key, "area"))
        return tablet_area_valid(value);
    if (g_str_equal(key, "eraser-button-mode"))
        return input_string_is(value, stylus_eraser_modes);
    if (g_str_has_suffix(key, "button-action"))
        return input_string_is(value, stylus_actions);
    if (g_str_has_suffix(key, "button-keybinding"))
        return g_variant_is_of_type(value, G_VARIANT_TYPE_STRING);
    static const char* boolean_fields[] = {
        "natural-scroll",
        "middle-click-emulation",
        "scroll-wheel-emulation-button-lock",
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

static const KeyboardAccessibilityField keyboard_accessibility_fields[] = {
    {"", "shortcuts-enabled", KEYBOARD_ACCESSIBILITY_BOOLEAN, 0, 0},
    {"", "beep-on-feature-state-change", KEYBOARD_ACCESSIBILITY_BOOLEAN, 0, 0},
    {"bounce-keys", "enabled", KEYBOARD_ACCESSIBILITY_BOOLEAN, 0, 0},
    {"bounce-keys", "delay-ms", KEYBOARD_ACCESSIBILITY_INTEGER, 0, 10000},
    {"bounce-keys", "beep-on-reject", KEYBOARD_ACCESSIBILITY_BOOLEAN, 0, 0},
    {"mouse-keys", "enabled", KEYBOARD_ACCESSIBILITY_BOOLEAN, 0, 0},
    {"mouse-keys", "max-speed", KEYBOARD_ACCESSIBILITY_INTEGER, 1, 10000},
    {"mouse-keys", "acceleration-time-ms", KEYBOARD_ACCESSIBILITY_INTEGER, 1, 10000},
    {"mouse-keys", "initial-delay-ms", KEYBOARD_ACCESSIBILITY_INTEGER, 0, 10000},
    {"slow-keys", "enabled", KEYBOARD_ACCESSIBILITY_BOOLEAN, 0, 0},
    {"slow-keys", "delay-ms", KEYBOARD_ACCESSIBILITY_INTEGER, 0, 10000},
    {"slow-keys", "beep-on-press", KEYBOARD_ACCESSIBILITY_BOOLEAN, 0, 0},
    {"slow-keys", "beep-on-accept", KEYBOARD_ACCESSIBILITY_BOOLEAN, 0, 0},
    {"slow-keys", "beep-on-reject", KEYBOARD_ACCESSIBILITY_BOOLEAN, 0, 0},
    {"sticky-keys", "enabled", KEYBOARD_ACCESSIBILITY_BOOLEAN, 0, 0},
    {"sticky-keys", "two-key-off", KEYBOARD_ACCESSIBILITY_BOOLEAN, 0, 0},
    {"sticky-keys", "beep-on-modifier", KEYBOARD_ACCESSIBILITY_BOOLEAN, 0, 0},
    {"toggle-keys", "enabled", KEYBOARD_ACCESSIBILITY_BOOLEAN, 0, 0},
};

static const KeyboardAccessibilityField* keyboard_accessibility_field(const char* section,
                                                                      const char* name) {
    for (guint i = 0; i < G_N_ELEMENTS(keyboard_accessibility_fields); i++) {
        const KeyboardAccessibilityField* field = &keyboard_accessibility_fields[i];
        if (g_str_equal(field->section, section) && g_str_equal(field->name, name))
            return field;
    }
    return NULL;
}

static gboolean keyboard_accessibility_section_known(const char* section) {
    for (guint i = 0; i < G_N_ELEMENTS(keyboard_accessibility_fields); i++) {
        if (keyboard_accessibility_fields[i].section[0] &&
            g_str_equal(keyboard_accessibility_fields[i].section, section))
            return TRUE;
    }
    return FALSE;
}

static gboolean keyboard_accessibility_value_valid(const KeyboardAccessibilityField* field,
                                                   GVariant* value) {
    if (g_variant_is_of_type(value, G_VARIANT_TYPE_STRING) &&
        g_str_equal(g_variant_get_string(value, NULL), "inherit"))
        return TRUE;
    if (field->type == KEYBOARD_ACCESSIBILITY_BOOLEAN)
        return g_variant_is_of_type(value, G_VARIANT_TYPE_BOOLEAN);

    double number;
    return input_number(value, &number) && number >= field->minimum && number <= field->maximum &&
           number == floor(number);
}

static gboolean keyboard_accessibility_valid(GVariant* accessibility, GError** error) {
    if (g_variant_is_of_type(accessibility, G_VARIANT_TYPE_STRING) &&
        g_str_equal(g_variant_get_string(accessibility, NULL), "inherit"))
        return TRUE;
    if (!g_variant_is_of_type(accessibility, G_VARIANT_TYPE_VARDICT)) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                            "input.keyboard.accessibility must be a table");
        return FALSE;
    }

    GVariantIter iter;
    const char* name;
    GVariant* value;
    g_variant_iter_init(&iter, accessibility);
    while (g_variant_iter_next(&iter, "{&sv}", &name, &value)) {
        g_autoptr(GVariant) member = value;
        const KeyboardAccessibilityField* field = keyboard_accessibility_field("", name);
        if (field) {
            if (!keyboard_accessibility_value_valid(field, member)) {
                g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                            "input.keyboard.accessibility.%s must be a boolean or inherit", name);
                return FALSE;
            }
            continue;
        }

        if (!keyboard_accessibility_section_known(name)) {
            g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                        "input.keyboard.accessibility.%s is unsupported", name);
            return FALSE;
        }
        if (g_variant_is_of_type(member, G_VARIANT_TYPE_STRING) &&
            g_str_equal(g_variant_get_string(member, NULL), "inherit"))
            continue;
        if (!g_variant_is_of_type(member, G_VARIANT_TYPE_VARDICT)) {
            g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                        "input.keyboard.accessibility.%s must be a table or inherit", name);
            return FALSE;
        }

        GVariantIter section_iter;
        const char* field_name;
        GVariant* field_value;
        g_variant_iter_init(&section_iter, member);
        while (g_variant_iter_next(&section_iter, "{&sv}", &field_name, &field_value)) {
            g_autoptr(GVariant) setting = field_value;
            field = keyboard_accessibility_field(name, field_name);
            if (!field || !keyboard_accessibility_value_valid(field, setting)) {
                g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                            "input.keyboard.accessibility.%s.%s is unsupported or invalid", name,
                            field_name);
                return FALSE;
            }
        }
    }
    return TRUE;
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
        if (g_str_equal(group, "keyboard") && g_str_equal(key, "accessibility")) {
            gboolean valid = keyboard_accessibility_valid(value, error);
            g_variant_unref(value);
            if (!valid)
                return FALSE;
            continue;
        }
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

gboolean validate_location(GVariant* location, GError** error) {
    if (!g_variant_is_of_type(location, G_VARIANT_TYPE_VARDICT)) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL, "location must be a table");
        return FALSE;
    }

    GVariantIter iter;
    const char* name;
    GVariant* value;
    g_variant_iter_init(&iter, location);
    while (g_variant_iter_next(&iter, "{&sv}", &name, &value)) {
        gboolean valid = FALSE;
        if (g_str_equal(name, "enabled")) {
            valid = g_variant_is_of_type(value, G_VARIANT_TYPE_BOOLEAN) ||
                    (g_variant_is_of_type(value, G_VARIANT_TYPE_STRING) &&
                     g_str_equal(g_variant_get_string(value, NULL), "inherit"));
            if (!valid)
                g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                                    "location.enabled must be a boolean or inherit");
        } else if (g_str_equal(name, "max-accuracy")) {
            if (g_variant_is_of_type(value, G_VARIANT_TYPE_STRING)) {
                const char* accuracy = g_variant_get_string(value, NULL);
                valid = g_str_equal(accuracy, "inherit") || g_str_equal(accuracy, "country") ||
                        g_str_equal(accuracy, "city") || g_str_equal(accuracy, "neighborhood") ||
                        g_str_equal(accuracy, "street") || g_str_equal(accuracy, "exact");
            }
            if (!valid)
                g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                                    "location.max_accuracy must be inherit, country, city, "
                                    "neighborhood, street, or exact");
        } else {
            g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                        "location.%s is an unsupported setting", name);
        }
        g_variant_unref(value);
        if (!valid)
            return FALSE;
    }
    return TRUE;
}

gboolean validate_auth(GVariant* auth, GError** error) {
    if (!g_variant_is_of_type(auth, G_VARIANT_TYPE_VARDICT)) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL, "auth must be a table");
        return FALSE;
    }

    GVariantIter iter;
    const char* name;
    GVariant* value;
    g_variant_iter_init(&iter, auth);
    while (g_variant_iter_next(&iter, "{&sv}", &name, &value)) {
        gboolean valid = FALSE;
        /* The Lua loader stores snake_case keys in kebab-case. */
        if (g_str_equal(name, "polkit-agent") || g_str_equal(name, "polkit_agent")) {
            valid = g_variant_is_of_type(value, G_VARIANT_TYPE_BOOLEAN);
            if (!valid)
                g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                                    "auth.polkit_agent must be a boolean");
        } else {
            g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                        "auth.%s is an unsupported setting", name);
        }
        g_variant_unref(value);
        if (!valid)
            return FALSE;
    }
    return TRUE;
}

gboolean validate_prompts(GVariant* prompts, GError** error) {
    if (!g_variant_is_of_type(prompts, G_VARIANT_TYPE_VARDICT)) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL, "prompts must be a table");
        return FALSE;
    }

    GVariantIter iter;
    const char* name;
    GVariant* value;
    g_variant_iter_init(&iter, prompts);
    while (g_variant_iter_next(&iter, "{&sv}", &name, &value)) {
        gboolean valid = FALSE;
        if (g_str_equal(name, "enabled")) {
            valid = g_variant_is_of_type(value, G_VARIANT_TYPE_BOOLEAN);
            if (!valid)
                g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                                    "prompts.enabled must be a boolean");
        } else {
            g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                        "prompts.%s is an unsupported setting", name);
        }
        g_variant_unref(value);
        if (!valid)
            return FALSE;
    }
    return TRUE;
}

gboolean validate_input(GVariant* input, GError** error) {
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
                   g_str_equal(group, "trackball") || g_str_equal(group, "pointing-stick") ||
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
