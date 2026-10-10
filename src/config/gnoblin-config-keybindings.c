#include "gnoblin-config-internal.h"

static gint titlebar_button_index(const char* name) {
    static const char* const buttons[] = {"menu", "minimize", "maximize", "close", NULL};
    for (guint i = 0; buttons[i]; i++)
        if (g_str_equal(name, buttons[i]))
            return (gint)i;
    return -1;
}

static gboolean titlebar_button_side_valid(GVariant* side) {
    if (!g_variant_is_of_type(side, G_VARIANT_TYPE("av")) || g_variant_n_children(side) > 7)
        return FALSE;

    gboolean used[4] = {FALSE};
    guint button_count = 0;
    gboolean previous_was_spacer = FALSE;
    gsize length = g_variant_n_children(side);
    for (gsize i = 0; i < length; i++) {
        g_autoptr(GVariant) boxed = g_variant_get_child_value(side, i);
        g_autoptr(GVariant) item = g_variant_get_variant(boxed);
        if (!g_variant_is_of_type(item, G_VARIANT_TYPE_STRING))
            return FALSE;

        const char* name = g_variant_get_string(item, NULL);
        if (g_str_equal(name, "spacer")) {
            if (i == 0 || i + 1 == length || previous_was_spacer)
                return FALSE;
            previous_was_spacer = TRUE;
            continue;
        }

        gint index = titlebar_button_index(name);
        if (index < 0 || used[index])
            return FALSE;
        used[index] = TRUE;
        button_count++;
        previous_was_spacer = FALSE;
    }

    return button_count <= G_N_ELEMENTS(used);
}

gboolean titlebar_button_layout_valid(GVariant* layout) {
    if (!g_variant_is_of_type(layout, G_VARIANT_TYPE_VARDICT))
        return FALSE;

    gboolean valid = TRUE;
    GVariantIter iter;
    const char* name;
    GVariant* value;
    g_variant_iter_init(&iter, layout);
    while (valid && g_variant_iter_next(&iter, "{&sv}", &name, &value)) {
        valid = (g_str_equal(name, "left") || g_str_equal(name, "right")) &&
                titlebar_button_side_valid(value);
        g_variant_unref(value);
    }
    return valid;
}

gboolean validate_monitors(GVariant* monitors, GError** error) {
    if (!g_variant_is_of_type(monitors, G_VARIANT_TYPE_VARDICT))
        goto invalid;

    GVariantIter iter;
    const char* name;
    GVariant* value;
    g_variant_iter_init(&iter, monitors);
    while (g_variant_iter_next(&iter, "{&sv}", &name, &value)) {
        gboolean valid = FALSE;
        if (g_str_equal(name, "privacy-screen")) {
            valid = g_variant_is_of_type(value, G_VARIANT_TYPE_BOOLEAN);
            if (g_variant_is_of_type(value, G_VARIANT_TYPE_STRING))
                valid = g_str_equal(g_variant_get_string(value, NULL), "inherit");
        } else if (g_str_equal(name, "global-scale")) {
            double scale;
            if (input_number(value, &scale))
                valid = scale >= 0 && scale <= 4 && scale == floor(scale);
            if (g_variant_is_of_type(value, G_VARIANT_TYPE_STRING))
                valid = g_str_equal(g_variant_get_string(value, NULL), "inherit");
        }
        g_variant_unref(value);
        if (!valid)
            goto invalid;
    }
    return TRUE;

invalid:
    g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                        "monitors accepts privacy-screen = true, false, or 'inherit', and "
                        "global-scale = 'inherit' or an integer from 0 through 4");
    return FALSE;
}

/* Pointer gestures share the keybinding table but use button accelerators. */
static gboolean validate_pointer_keybindings(GVariant* actions, GError** error) {
    if (!g_variant_is_of_type(actions, G_VARIANT_TYPE_VARDICT)) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                            "keybindings.pointer must be an action table");
        return FALSE;
    }
    GVariantIter iter;
    const char* action;
    GVariant* value;
    g_variant_iter_init(&iter, actions);
    while (g_variant_iter_next(&iter, "{&sv}", &action, &value)) {
        g_autoptr(GVariant) bindings = value;
        if (!g_regex_match_simple("^[a-z0-9]+(?:_[a-z0-9]+)*$", action, 0, 0) ||
            !g_variant_is_of_type(bindings, G_VARIANT_TYPE("av"))) {
            g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                                "keybindings.pointer needs named arrays of button accelerators");
            return FALSE;
        }
        for (gsize i = 0; i < g_variant_n_children(bindings); i++) {
            g_autoptr(GVariant) boxed = g_variant_get_child_value(bindings, i);
            g_autoptr(GVariant) binding = g_variant_get_variant(boxed);
            const char* text = g_variant_is_of_type(binding, G_VARIANT_TYPE_STRING)
                                   ? g_variant_get_string(binding, NULL)
                                   : NULL;
            const char* button = text ? strstr(text, "Button") : NULL;
            g_autofree char* modifiers = button ? g_strndup(text, button - text) : NULL;
            if (!text || strlen(text) > 160 || !button || !modifiers[0] ||
                !window_modifier_valid(modifiers) ||
                (!g_str_equal(button, "Button1") && !g_str_equal(button, "Button2") &&
                 !g_str_equal(button, "Button3"))) {
                g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                            "keybindings.pointer.%s needs modifier tokens followed by Button1, "
                            "Button2 or Button3",
                            action);
                return FALSE;
            }
        }
    }
    return TRUE;
}

/* Input policy publishes selectors, never Lua functions, to the compositor. */
gboolean validate_input_handlers(GVariant* document, GError** error) {
    g_autoptr(GHashTable) ids = NULL;
    g_autoptr(GVariant) handlers = g_variant_lookup_value(document, "input-handlers", NULL);
    if (!handlers)
        return TRUE;
    if (!g_variant_is_of_type(handlers, G_VARIANT_TYPE("av")) ||
        g_variant_n_children(handlers) > 256)
        goto invalid;
    ids = g_hash_table_new_full(g_str_hash, g_str_equal, g_free, NULL);
    for (gsize i = 0; i < g_variant_n_children(handlers); i++) {
        g_autoptr(GVariant) boxed = g_variant_get_child_value(handlers, i);
        g_autoptr(GVariant) item = g_variant_get_variant(boxed);
        const char* id = NULL;
        if (!g_variant_is_of_type(item, G_VARIANT_TYPE_VARDICT) ||
            g_variant_n_children(item) != 2 || !g_variant_lookup(item, "id", "&s", &id) || !*id ||
            strlen(id) > 128 || !g_utf8_validate(id, -1, NULL) || g_hash_table_contains(ids, id))
            goto invalid;
        /* IDs must remain alive after the item is released. */
        g_hash_table_add(ids, g_strdup(id));
        g_autoptr(GVariant) match = g_variant_lookup_value(item, "match", G_VARIANT_TYPE_VARDICT);
        const char* type = NULL;
        static const char* const types[] = {"key",   "button", "motion",  "scroll",
                                            "touch", "tablet", "gesture", NULL};
        if (!match || !g_variant_lookup(match, "type", "&s", &type))
            goto invalid;
        gboolean known = FALSE;
        for (guint j = 0; types[j]; j++)
            known |= g_str_equal(type, types[j]);
        if (!known)
            goto invalid;
        GVariantIter fields;
        const char* name;
        GVariant* value;
        g_variant_iter_init(&fields, match);
        while (g_variant_iter_next(&fields, "{&sv}", &name, &value)) {
            g_autoptr(GVariant) field = value;
            gboolean valid = FALSE;
            if (g_str_equal(name, "type"))
                valid = TRUE;
            else if (g_str_equal(name, "device-id") || g_str_equal(name, "accelerator")) {
                if (g_variant_is_of_type(field, G_VARIANT_TYPE_STRING)) {
                    const char* text = g_variant_get_string(field, NULL);
                    valid = *text && strlen(text) <= 160 && g_utf8_validate(text, -1, NULL);
                }
                if (g_str_equal(name, "accelerator") && !g_str_equal(type, "key") &&
                    !g_str_equal(type, "button"))
                    valid = FALSE;
            } else if (g_str_equal(name, "phase")) {
                static const char* const discrete[] = {"press", "release", NULL};
                static const char* const stream[] = {"begin", "update", "end", "cancel", NULL};
                static const char* const tablet[] = {"press", "release", "update", NULL};
                static const char* const update[] = {"update", NULL};
                const char* const* phases =
                    (g_str_equal(type, "key") || g_str_equal(type, "button"))      ? discrete
                    : (g_str_equal(type, "touch") || g_str_equal(type, "gesture")) ? stream
                    : g_str_equal(type, "tablet")                                  ? tablet
                                                                                   : update;
                valid = input_string_is(field, phases);
            } else if (g_str_equal(name, "keycode") || g_str_equal(name, "button") ||
                       g_str_equal(name, "modifiers")) {
                double number;
                valid = (g_variant_is_of_type(field, G_VARIANT_TYPE_INT64) ||
                         g_variant_is_of_type(field, G_VARIANT_TYPE_UINT32)) &&
                        input_number(field, &number) && number >= 0 && number <= G_MAXUINT32 &&
                        number == floor(number);
                if (g_str_equal(name, "button"))
                    valid = valid && number >= 1 && number <= 32 &&
                            (g_str_equal(type, "button") || g_str_equal(type, "tablet"));
                if (g_str_equal(name, "keycode"))
                    valid = valid && g_str_equal(type, "key");
            } else if (g_str_equal(name, "allow-when-shortcuts-inhibited"))
                valid = g_variant_is_of_type(field, G_VARIANT_TYPE_BOOLEAN);
            if (!valid)
                goto invalid;
        }
    }
    return TRUE;
invalid:
    g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                        "input handlers require unique string IDs and valid "
                        "type/device/phase/key/button/modifier selectors");
    return FALSE;
}

static gboolean validate_keyboard_callbacks(GVariant* actions, GError** error) {
    if (!g_variant_is_of_type(actions, G_VARIANT_TYPE_VARDICT))
        goto invalid;
    GVariantIter iter;
    const char* name;
    GVariant* value;
    g_variant_iter_init(&iter, actions);
    while (g_variant_iter_next(&iter, "{&sv}", &name, &value)) {
        g_autoptr(GVariant) bindings = value;
        if (!*name || strlen(name) > 128 || !g_variant_is_of_type(bindings, G_VARIANT_TYPE("av")))
            goto invalid;
        for (gsize i = 0; i < g_variant_n_children(bindings); i++) {
            g_autoptr(GVariant) boxed = g_variant_get_child_value(bindings, i);
            g_autoptr(GVariant) binding = g_variant_get_variant(boxed);
            if (!g_variant_is_of_type(binding, G_VARIANT_TYPE_STRING))
                goto invalid;
            const char* text = g_variant_get_string(binding, NULL);
            if (!*text || strlen(text) > 160 || !g_utf8_validate(text, -1, NULL))
                goto invalid;
        }
    }
    return TRUE;
invalid:
    g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                        "keybindings.keyboard requires named arrays of keyboard accelerators");
    return FALSE;
}

gboolean validate_keybindings(GVariant* document, GError** error) {
    g_autoptr(GVariant) configured = g_variant_lookup_value(document, "keybindings", NULL);
    if (!configured)
        return TRUE;
    if (!g_variant_is_of_type(configured, G_VARIANT_TYPE_VARDICT)) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL, "keybindings must be a table");
        return FALSE;
    }

    g_autoptr(GnoblinKeybindingCatalog) catalog = gnoblin_keybinding_catalog_load(error);
    if (!catalog)
        return FALSE;

    GVariantIter groups;
    const char* group;
    GVariant* group_value;
    g_variant_iter_init(&groups, configured);
    while (g_variant_iter_next(&groups, "{&sv}", &group, &group_value)) {
        g_autoptr(GVariant) actions = group_value;
        if (g_str_equal(group, "keyboard")) {
            if (!validate_keyboard_callbacks(actions, error))
                return FALSE;
            continue;
        }
        if (g_str_equal(group, "pointer")) {
            if (!validate_pointer_keybindings(actions, error))
                return FALSE;
            continue;
        }
        if (!gnoblin_keybinding_catalog_get_group(catalog, group) ||
            !g_variant_is_of_type(actions, G_VARIANT_TYPE_VARDICT)) {
            g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                        "keybindings.%s must be a supported group table", group);
            return FALSE;
        }

        GVariantIter action_iter;
        const char* name;
        GVariant* bindings;
        g_variant_iter_init(&action_iter, actions);
        while (g_variant_iter_next(&action_iter, "{&sv}", &name, &bindings)) {
            g_autoptr(GVariant) values = bindings;
            if (!g_regex_match_simple("^[a-z0-9]+(?:_[a-z0-9]+)*$", name, 0, 0)) {
                g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                            "invalid keybinding name: %s.%s (use snake_case)", group, name);
                return FALSE;
            }

            g_autofree char* native_name = g_strdup(name);
            g_strdelimit(native_name, "_", '-');
            if (!gnoblin_keybinding_catalog_lookup(catalog, group, native_name)) {
                g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                            "keybinding has no action in this Mutter build: %s.%s", group, name);
                return FALSE;
            }
            if (!g_variant_is_of_type(values, G_VARIANT_TYPE("av"))) {
                g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                            "keybindings.%s.%s must be an array of accelerators", group, name);
                return FALSE;
            }
            for (gsize i = 0; i < g_variant_n_children(values); i++) {
                g_autoptr(GVariant) boxed = g_variant_get_child_value(values, i);
                g_autoptr(GVariant) binding = g_variant_get_variant(boxed);
                const char* accelerator = g_variant_is_of_type(binding, G_VARIANT_TYPE_STRING)
                                              ? g_variant_get_string(binding, NULL)
                                              : NULL;
                if (!accelerator || !*accelerator || strlen(accelerator) > 160 ||
                    !g_utf8_validate(accelerator, -1, NULL)) {
                    g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                                "keybindings.%s.%s contains an invalid accelerator", group, name);
                    return FALSE;
                }
            }
        }
    }
    return TRUE;
}
