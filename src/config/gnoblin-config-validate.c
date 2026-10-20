#include "gnoblin-config-internal.h"

gboolean gnoblin_config_validate_document(GVariant* document, GError** error) {
    g_autoptr(GVariant) shell = g_variant_lookup_value(document, "shell", NULL);
    if (shell) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                            "shell settings are no longer supported; configure shell UI in the "
                            "shell client, and use gnoblin.animation and window rules for "
                            "compositor animations");
        return FALSE;
    }
    if (!validate_input_handlers(document, error) || !validate_keybindings(document, error))
        return FALSE;
    if (!gnoblin_permission_policy_validate(document, error))
        return FALSE;
    g_autoptr(GVariant) location = g_variant_lookup_value(document, "location", NULL);
    if (location && !validate_location(location, error))
        return FALSE;
    g_autoptr(GVariant) prompts = g_variant_lookup_value(document, "prompts", NULL);
    if (prompts && !validate_prompts(prompts, error))
        return FALSE;
    g_autoptr(GVariant) auth = g_variant_lookup_value(document, "auth", NULL);
    if (auth && !validate_auth(auth, error))
        return FALSE;
    g_autoptr(GVariant) portals = g_variant_lookup_value(document, "portals", NULL);
    if (portals && !validate_portal_selection(portals, error))
        return FALSE;
    g_autoptr(GVariant) monitors = g_variant_lookup_value(document, "monitors", NULL);
    if (monitors && !validate_monitors(monitors, error))
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
        gsize failed_index = 0;
        for (gsize i = 0; valid && i < g_variant_n_children(workspaces); i++) {
            failed_index = i + 1;
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
            g_autofree char* where =
                failed_index > 0 ? g_strdup_printf("check entry %" G_GSIZE_FORMAT, failed_index)
                                 : g_strdup("the value is not a nonempty array");
            g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                        "workspaces must be a nonempty array of objects with unique valid "
                        "ids and nonempty names up to 80 characters (%s)",
                        where);
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
        const char* name = NULL;
        GVariant* value;
        if (valid)
            g_variant_iter_init(&iter, section);
        while (valid && g_variant_iter_next(&iter, "{&sv}", &name, &value)) {
            valid = g_variant_is_of_type(value, G_VARIANT_TYPE_BOOLEAN) &&
                    (!keys[i] || g_str_equal(name, keys[i]));
            g_variant_unref(value);
        }
        if (!valid) {
            if (name)
                g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                            "%s \"%s\" must be a supported setting with a boolean value",
                            sections[i], name);
            else
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
            "attach-modal-dialogs",
            "disable-workarounds",
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
        const char* name = NULL;
        GVariant* value;
        if (valid)
            g_variant_iter_init(&iter, window);
        while (valid && g_variant_iter_next(&iter, "{&sv}", &name, &value)) {
            gboolean known_boolean = FALSE;
            for (guint i = 0; booleans[i]; i++)
                known_boolean |= g_str_equal(name, booleans[i]);
            if (known_boolean) {
                valid = g_variant_is_of_type(value, G_VARIANT_TYPE_BOOLEAN);
            } else if (g_str_equal(name, "center-new-windows")) {
                g_variant_unref(value);
                g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                                    "window-management center-new-windows was replaced by "
                                    "new-window-placement, which takes \"free_space\", "
                                    "\"center\" or \"cascade\"");
                return FALSE;
            } else if (g_str_equal(name, "new-window-placement")) {
                const char* string = g_variant_is_of_type(value, G_VARIANT_TYPE_STRING)
                                         ? g_variant_get_string(value, NULL)
                                         : "";
                valid = g_str_equal(string, "free_space") || g_str_equal(string, "center") ||
                        g_str_equal(string, "cascade");
            } else if (g_str_equal(name, "auto-raise-delay")) {
                gint64 number =
                    g_variant_is_of_type(value, G_VARIANT_TYPE_INT32)   ? g_variant_get_int32(value)
                    : g_variant_is_of_type(value, G_VARIANT_TYPE_INT64) ? g_variant_get_int64(value)
                                                                        : -1;
                valid = number >= 0 && number <= 10000;
            } else if (g_str_equal(name, "check-alive-timeout")) {
                gint64 number =
                    g_variant_is_of_type(value, G_VARIANT_TYPE_INT32)   ? g_variant_get_int32(value)
                    : g_variant_is_of_type(value, G_VARIANT_TYPE_INT64) ? g_variant_get_int64(value)
                                                                        : -1;
                valid = number >= 0 && (guint64)number <= G_MAXUINT;
            } else if (g_str_equal(name, "button-layout")) {
                valid = titlebar_button_layout_valid(value);
            } else if (g_variant_is_of_type(value, G_VARIANT_TYPE_STRING)) {
                const char* string = g_variant_get_string(value, NULL);
                if (g_str_equal(name, "focus-mode"))
                    valid = g_str_equal(string, "click") || g_str_equal(string, "hover") ||
                            g_str_equal(string, "hover-strict") || g_str_equal(string, "sloppy") ||
                            g_str_equal(string, "mouse");
                else if (g_str_equal(name, "focus-new-windows"))
                    valid = g_str_equal(string, "allow") || g_str_equal(string, "prevent") ||
                            g_str_equal(string, "smart") || g_str_equal(string, "strict");
                else if (g_str_equal(name, "action-double-click-titlebar") ||
                         g_str_equal(name, "action-middle-click-titlebar") ||
                         g_str_equal(name, "action-right-click-titlebar")) {
                    valid = FALSE;
                    for (guint i = 0; titlebar[i]; i++)
                        valid |= g_str_equal(string, titlebar[i]);
                } else
                    valid = FALSE;
            } else
                valid = FALSE;
            g_variant_unref(value);
        }
        if (!valid) {
            g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                        "window-management %s%s%s has an unsupported name or value",
                        name ? "\"" : "", name ? name : "(not a table)", name ? "\"" : "");
            return FALSE;
        }
    }
    g_autoptr(GVariant) compositor = g_variant_lookup_value(document, "compositor", NULL);
    if (compositor) {
        static const char* booleans[] = {
            "debug", "locate-pointer", "visual-bell", "audible-bell", NULL,
        };
        gboolean valid = g_variant_is_of_type(compositor, G_VARIANT_TYPE_VARDICT);
        GVariantIter iter;
        const char* name = NULL;
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
            } else if (g_str_equal(name, "blur") &&
                       g_variant_is_of_type(value, G_VARIANT_TYPE_STRING)) {
                const char* blur = g_variant_get_string(value, NULL);
                valid = g_str_equal(blur, "auto") || g_str_equal(blur, "on") ||
                        g_str_equal(blur, "off");
            } else if (g_str_equal(name, "locate-pointer-key") &&
                       g_variant_is_of_type(value, G_VARIANT_TYPE_STRING)) {
                valid = compositor_locate_pointer_key_valid(g_variant_get_string(value, NULL));
            } else
                valid = FALSE;
            g_variant_unref(value);
        }
        if (!valid) {
            g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                        "compositor %s%s%s has an unsupported name or value", name ? "\"" : "",
                        name ? name : "(not a table)", name ? "\"" : "");
            return FALSE;
        }
    }
    g_autoptr(GVariant) cursor = g_variant_lookup_value(document, "cursor", NULL);
    if (cursor) {
        gboolean valid = g_variant_is_of_type(cursor, G_VARIANT_TYPE_VARDICT);
        GVariantIter iter;
        const char* name = NULL;
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
            g_set_error(
                error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                "cursor%s%s%s is not valid; cursor takes a nonempty theme and a size from 1 to 256",
                name ? " \"" : "", name ? name : "", name ? "\"" : "");
            return FALSE;
        }
    }
    g_autoptr(GVariant) xwayland = g_variant_lookup_value(document, "xwayland", NULL);
    if (xwayland && !validate_xwayland(xwayland, error))
        return FALSE;
    return TRUE;
}
