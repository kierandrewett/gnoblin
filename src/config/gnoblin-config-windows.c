#include "gnoblin-config-internal.h"

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

gboolean window_modifier_valid(const char* modifier) {
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

static gboolean compositor_key_symbol_valid(const char* name) {
    if (xkb_keysym_from_name(name, XKB_KEYSYM_CASE_INSENSITIVE) != XKB_KEY_NoSymbol)
        return TRUE;
    g_autofree char* xf86_name = g_strconcat("XF86", name, NULL);
    return xkb_keysym_from_name(xf86_name, XKB_KEYSYM_CASE_INSENSITIVE) != XKB_KEY_NoSymbol;
}

gboolean compositor_locate_pointer_key_valid(const char* name) {
    if (!name[0] || g_str_equal(name, "disabled") || g_str_equal(name, "Above_Tab"))
        return TRUE;
    if (strchr(name, '<') || strchr(name, '>') || strpbrk(name, " \t\r\n\v\f"))
        return FALSE;
    if (compositor_key_symbol_valid(name))
        return TRUE;

    g_autofree char* left = g_strconcat(name, "_L", NULL);
    g_autofree char* right = g_strconcat(name, "_R", NULL);
    return compositor_key_symbol_valid(left) && compositor_key_symbol_valid(right);
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

gboolean validate_touchpad_gestures(GVariant* gestures, GError** error) {
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
            g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                        "touchpad-gestures entry %" G_GSIZE_FORMAT " contains an unknown field",
                        i + 1);
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
            g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                        "touchpad-gestures entry %" G_GSIZE_FORMAT
                        " needs a unique name of 1 to 64 characters (letters, digits, - and _)",
                        i + 1);
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
                               : "unlocked";
        /* "normal" and "unlock-screen" are the deprecated spellings of "unlocked" and "locked". */
        if (when_value && (!g_variant_is_of_type(when_value, G_VARIANT_TYPE_STRING) ||
                           (!g_str_equal(when, "unlocked") && !g_str_equal(when, "locked") &&
                            !g_str_equal(when, "normal") && !g_str_equal(when, "unlock-screen") &&
                            !g_str_equal(when, "any")))) {
            g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                        "%s: when must be unlocked, locked, or any", name);
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

static gboolean window_rule_color_is_valid(const char* text) {
    const gsize length = strlen(text);
    if ((length != 7 && length != 9) || text[0] != '#')
        return FALSE;
    for (gsize i = 1; i < length; i++) {
        if (!g_ascii_isxdigit(text[i]))
            return FALSE;
    }
    return TRUE;
}

/* The inner outline is new, so unlike border_width (which the compositor clamps) a bad value is a
 * configuration error. The Lua loader may store the keys with underscores or hyphens. */
static gboolean validate_window_rule_inner_border(GVariant* corners, gsize rule_index,
                                                  GError** error) {
    static const char* const width_keys[] = {"inner_border_width", "inner-border-width"};
    static const char* const color_keys[] = {"inner_border_color", "inner-border-color"};
    for (guint i = 0; i < G_N_ELEMENTS(width_keys); i++) {
        g_autoptr(GVariant) width = g_variant_lookup_value(corners, width_keys[i], NULL);
        if (!width)
            continue;
        double number;
        if (!input_number(width, &number) || number < 0 || number > 40) {
            g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                        "window-rules[%zu].corners.inner_border_width must be a finite number "
                        "from 0 to 40",
                        rule_index);
            return FALSE;
        }
    }
    for (guint i = 0; i < G_N_ELEMENTS(color_keys); i++) {
        g_autoptr(GVariant) color = g_variant_lookup_value(corners, color_keys[i], NULL);
        if (!color)
            continue;
        if (!g_variant_is_of_type(color, G_VARIANT_TYPE_STRING) ||
            !window_rule_color_is_valid(g_variant_get_string(color, NULL))) {
            g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                        "window-rules[%zu].corners.inner_border_color must be #RRGGBB or "
                        "#RRGGBBAA",
                        rule_index);
            return FALSE;
        }
    }
    return TRUE;
}

static gboolean validate_window_rule_blur(GVariant* rule, gsize rule_index, GError** error) {
    g_autoptr(GVariant) blur = g_variant_lookup_value(rule, "blur", NULL);
    if (blur) {
        double number = 0;
        if (g_variant_is_of_type(blur, G_VARIANT_TYPE_DOUBLE))
            number = g_variant_get_double(blur);
        else if (g_variant_is_of_type(blur, G_VARIANT_TYPE_INT64))
            number = (double)g_variant_get_int64(blur);
        else if (g_variant_is_of_type(blur, G_VARIANT_TYPE_INT32))
            number = (double)g_variant_get_int32(blur);
        else
            number = -1;
        if (!(number >= 0 && number <= 100)) {
            g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                        "window-rules[%zu].blur must be a number from 0 to 100", rule_index);
            return FALSE;
        }
    }

    /* The Lua loader stores snake_case keys in kebab-case. */
    g_autoptr(GVariant) ignore = g_variant_lookup_value(rule, "blur-ignore-shadows", NULL);
    if (!ignore)
        ignore = g_variant_lookup_value(rule, "blur_ignore_shadows", NULL);
    if (ignore && !g_variant_is_of_type(ignore, G_VARIANT_TYPE_BOOLEAN)) {
        g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                    "window-rules[%zu].blur_ignore_shadows must be a boolean", rule_index);
        return FALSE;
    }
    return TRUE;
}

static gboolean validate_window_rule_workspace(GVariant* rule, gsize rule_index, GError** error) {
    g_autoptr(GVariant) workspace = g_variant_lookup_value(rule, "workspace", NULL);
    if (!workspace)
        return TRUE;
    if (!g_variant_is_of_type(workspace, G_VARIANT_TYPE_VARDICT)) {
        g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                    "window-rules[%zu].workspace must be a table with id or number", rule_index);
        return FALSE;
    }

    gboolean has_id = FALSE;
    gboolean has_number = FALSE;
    GVariantIter iter;
    const char* key;
    GVariant* value;
    g_variant_iter_init(&iter, workspace);
    while (g_variant_iter_next(&iter, "{&sv}", &key, &value)) {
        g_autoptr(GVariant) field = value;
        if (g_str_equal(key, "id")) {
            has_id = TRUE;
            if (!g_variant_is_of_type(field, G_VARIANT_TYPE_STRING) ||
                !*g_variant_get_string(field, NULL) ||
                strlen(g_variant_get_string(field, NULL)) > 128) {
                g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                            "window-rules[%zu].workspace.id must be a nonempty string of at most "
                            "128 bytes",
                            rule_index);
                return FALSE;
            }
        } else if (g_str_equal(key, "number")) {
            has_number = TRUE;
            gint64 number = 0;
            gboolean integer = TRUE;
            if (g_variant_is_of_type(field, G_VARIANT_TYPE_INT64))
                number = g_variant_get_int64(field);
            else if (g_variant_is_of_type(field, G_VARIANT_TYPE_INT32))
                number = g_variant_get_int32(field);
            else
                integer = FALSE;
            if (!integer || number < 1 || number > 1024) {
                g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                            "window-rules[%zu].workspace.number must be an integer from 1 to 1024",
                            rule_index);
                return FALSE;
            }
        } else {
            g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                        "window-rules[%zu].workspace.%s is not supported; use id or number",
                        rule_index, key);
            return FALSE;
        }
    }
    if (has_id == has_number) {
        g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                    "window-rules[%zu].workspace must set exactly one of id or number", rule_index);
        return FALSE;
    }
    return TRUE;
}

gboolean validate_window_rule_patterns(GVariant* document, GError** error) {
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
                        "use corners.border_width and corners.border_color for the outline, and "
                        "corners.inner_border_width and corners.inner_border_color for an inner "
                        "line",
                        i + 1);
            return FALSE;
        }
        if (!validate_window_rule_workspace(rule, i + 1, error))
            return FALSE;
        if (!validate_window_rule_blur(rule, i + 1, error))
            return FALSE;
        g_autoptr(GVariant) corners = g_variant_lookup_value(rule, "corners", NULL);
        if (corners && g_variant_is_of_type(corners, G_VARIANT_TYPE_VARDICT) &&
            !validate_window_rule_inner_border(corners, i + 1, error))
            return FALSE;
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
