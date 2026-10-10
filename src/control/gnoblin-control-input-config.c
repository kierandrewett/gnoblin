#include "gnoblin-native-control-private.h"

static GVariant* normalize_input_accessibility(GVariant* accessibility, GError** error);

static void native_overlay_key(MetaDisplay* display, gpointer user_data) {
    GnoblinNativeControl* control = user_data;
    for (guint i = 0; i < control->shortcuts->len; i++) {
        NativeShortcut* shortcut = g_ptr_array_index(control->shortcuts, i);
        if (shortcut->overlay)
            gnoblin_control_launch_command("shortcut", shortcut->name, shortcut->argv);
    }
}

static gboolean native_action_target(GVariant* action, const GnoblinKeybindingCatalog* catalog,
                                     const char** group, char** native_name) {
    const char* name = NULL;
    const char* schema = NULL;
    if (g_variant_is_of_type(action, G_VARIANT_TYPE_STRING)) {
        const char* label = g_variant_get_string(action, NULL);
        const char* separator = strchr(label, '.');
        if (!separator)
            return FALSE;
        g_autofree char* namespace = g_strndup(label, separator - label);
        if (g_str_equal(namespace, "wm") || g_str_equal(namespace, "mutter") ||
            g_str_equal(namespace, "wayland"))
            *group = g_intern_string(namespace);
        else
            return FALSE;
        name = separator + 1;
        if (!g_regex_match_simple("^[a-z0-9]+(?:_[a-z0-9]+)*$", name, 0, 0))
            return FALSE;
        *native_name = g_strdup(name);
        g_strdelimit(*native_name, "_", '-');
        return gnoblin_keybinding_catalog_lookup(catalog, *group, *native_name) &&
               meta_keybindings_gnoblin_action_supported(*group, *native_name);
    }
    if (!g_variant_is_of_type(action, G_VARIANT_TYPE_VARDICT) ||
        g_variant_n_children(action) != 2 || !g_variant_lookup(action, "schema", "&s", &schema) ||
        !g_variant_lookup(action, "key", "&s", &name) ||
        !g_regex_match_simple("^[a-z0-9]+(?:-[a-z0-9]+)*$", name, 0, 0))
        return FALSE;
    if (g_str_equal(schema, "org.gnome.desktop.wm.keybindings"))
        *group = "wm";
    else if (g_str_equal(schema, "org.gnome.mutter.keybindings"))
        *group = "mutter";
    else if (g_str_equal(schema, "org.gnome.mutter.wayland.keybindings"))
        *group = "wayland";
    else
        return FALSE;
    *native_name = g_strdup(name);
    return gnoblin_keybinding_catalog_lookup(catalog, *group, *native_name) &&
           meta_keybindings_gnoblin_action_supported(*group, *native_name);
}

static GVariant* native_binding_array(GVariant* bindings) {
    if (!bindings || !g_variant_is_of_type(bindings, G_VARIANT_TYPE("av")))
        return NULL;
    g_auto(GStrv) strings = g_new0(char*, g_variant_n_children(bindings) + 1);
    for (gsize i = 0; i < g_variant_n_children(bindings); i++) {
        g_autoptr(GVariant) boxed = g_variant_get_child_value(bindings, i);
        g_autoptr(GVariant) binding = g_variant_get_variant(boxed);
        if (!g_variant_is_of_type(binding, G_VARIANT_TYPE_STRING) ||
            !*g_variant_get_string(binding, NULL) ||
            strlen(g_variant_get_string(binding, NULL)) > 160)
            return NULL;
        strings[i] = g_variant_dup_string(binding, NULL);
    }
    return g_variant_ref_sink(g_variant_new_strv((const char* const*)strings, -1));
}

static gboolean merge_native_action(GVariantDict* groups, GVariant* entry,
                                    const GnoblinKeybindingCatalog* catalog, GError** error) {
    g_autoptr(GVariant) action = g_variant_lookup_value(entry, "action", NULL);
    g_autoptr(GVariant) command = g_variant_lookup_value(entry, "command", NULL);
    g_autoptr(GVariant) trigger = g_variant_lookup_value(entry, "trigger", NULL);
    g_autoptr(GVariant) binding = g_variant_lookup_value(entry, "binding", NULL);
    g_autoptr(GVariant) capture = g_variant_lookup_value(entry, "capture-input", NULL);
    g_autoptr(GVariant) native_bindings = NULL;
    g_autoptr(GVariant) existing_group = NULL;
    g_autoptr(GVariant) existing_action = NULL;
    const char* group = NULL;
    g_autofree char* native_name = NULL;
    if (!native_action_target(action, catalog, &group, &native_name)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "named shortcut action must identify a supported Mutter or Wayland "
                            "keybinding; GNOME Shell actions are not supported");
        return FALSE;
    }
    if (command || trigger ||
        (capture && (!g_variant_is_of_type(capture, G_VARIANT_TYPE_BOOLEAN) ||
                     g_variant_get_boolean(capture)))) {
        g_set_error_literal(
            error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
            "named built-in actions cannot have command, trigger, or input capture");
        return FALSE;
    }
    native_bindings = native_binding_array(binding);
    if (!native_bindings) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "named built-in action needs an array of accelerators");
        return FALSE;
    }
    existing_group = g_variant_dict_lookup_value(groups, group, G_VARIANT_TYPE_VARDICT);
    existing_action =
        existing_group ? g_variant_lookup_value(existing_group, native_name, NULL) : NULL;
    if (existing_action) {
        g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                    "native action configured more than once: %s.%s", group, native_name);
        return FALSE;
    }
    GVariantDict actions;
    g_variant_dict_init(&actions, existing_group);
    g_variant_dict_insert_value(&actions, native_name, native_bindings);
    g_autoptr(GVariant) merged_actions = g_variant_ref_sink(g_variant_dict_end(&actions));
    g_variant_dict_insert_value(groups, group, merged_actions);
    return TRUE;
}

static GVariant* merge_native_actions(GVariant* base, GVariant* document,
                                      const GnoblinKeybindingCatalog* catalog, GError** error) {
    g_autoptr(GVariant) shortcuts =
        document ? g_variant_lookup_value(document, "shortcuts", NULL) : NULL;
    if (!shortcuts)
        return g_variant_ref(base);
    if (!g_variant_is_of_type(shortcuts, G_VARIANT_TYPE("av")) ||
        g_variant_n_children(shortcuts) > 256) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "shortcuts must be an array of at most 256 entries");
        return NULL;
    }
    GVariantDict groups;
    g_variant_dict_init(&groups, base);
    for (gsize i = 0; i < g_variant_n_children(shortcuts); i++) {
        g_autoptr(GVariant) boxed = g_variant_get_child_value(shortcuts, i);
        g_autoptr(GVariant) entry = g_variant_get_variant(boxed);
        if (!g_variant_is_of_type(entry, G_VARIANT_TYPE_VARDICT)) {
            g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                        "shortcut entry %zu must be a table", i + 1);
            g_variant_dict_clear(&groups);
            return NULL;
        }
        g_autoptr(GVariant) action = g_variant_lookup_value(entry, "action", NULL);
        if (action && !merge_native_action(&groups, entry, catalog, error)) {
            g_variant_dict_clear(&groups);
            return NULL;
        }
    }
    return g_variant_ref_sink(g_variant_dict_end(&groups));
}

gboolean apply_native_keybindings(GnoblinNativeControl* control, GVariant* document,
                                  GError** error) {
    g_autoptr(GVariant) input_handlers =
        document ? g_variant_lookup_value(document, "input-handlers", G_VARIANT_TYPE("av")) : NULL;
    for (gsize i = 0; input_handlers && i < g_variant_n_children(input_handlers); i++) {
        g_autoptr(GVariant) boxed = g_variant_get_child_value(input_handlers, i);
        g_autoptr(GVariant) handler = g_variant_get_variant(boxed);
        g_autoptr(GVariant) match =
            g_variant_lookup_value(handler, "match", G_VARIANT_TYPE_VARDICT);
        const char *accelerator = NULL, *type = NULL;
        if (match && g_variant_lookup(match, "accelerator", "&s", &accelerator) &&
            (!g_variant_lookup(match, "type", "&s", &type) ||
             !meta_keybindings_gnoblin_accelerator_valid(control->display, accelerator,
                                                         g_str_equal(type, "button")))) {
            g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                        "invalid input handler accelerator: %s", accelerator);
            return FALSE;
        }
    }
    g_autoptr(GVariant) configured =
        document ? g_variant_lookup_value(document, "keybindings", NULL) : NULL;
    if (configured && !g_variant_is_of_type(configured, G_VARIANT_TYPE_VARDICT)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "keybindings must be a table");
        return FALSE;
    }
    g_autoptr(GnoblinKeybindingCatalog) catalog = gnoblin_keybinding_catalog_load(error);
    if (!catalog)
        return FALSE;

    GVariantBuilder groups;
    g_autoptr(GVariant) normalized = NULL;
    g_autoptr(GVariant) merged = NULL;
    g_variant_builder_init(&groups, G_VARIANT_TYPE_VARDICT);
    if (configured) {
        GVariantIter iter;
        const char* group;
        GVariant* entries;
        g_variant_iter_init(&iter, configured);
        while (g_variant_iter_next(&iter, "{&sv}", &group, &entries)) {
            g_autoptr(GVariant) group_entries = entries;
            /* Pointer accelerators are consumed by native window input,
             * not by Mutter's keyboard action catalog. */
            if (g_str_equal(group, "pointer") || g_str_equal(group, "keyboard"))
                continue;
            if (!gnoblin_keybinding_catalog_get_group(catalog, group)) {
                g_set_error(error, G_IO_ERROR, G_IO_ERROR_NOT_SUPPORTED,
                            "keybindings.%s is unsupported in the standalone session; use wm, "
                            "mutter, or wayland",
                            group);
                goto invalid_keybindings;
            }
            if (!g_variant_is_of_type(group_entries, G_VARIANT_TYPE_VARDICT)) {
                g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "keybindings.%s must be a table", group);
                goto invalid_keybindings;
            }
            GVariantBuilder actions;
            GVariantIter action_iter;
            const char* name;
            GVariant* value;
            g_variant_builder_init(&actions, G_VARIANT_TYPE_VARDICT);
            g_variant_iter_init(&action_iter, group_entries);
            while (g_variant_iter_next(&action_iter, "{&sv}", &name, &value)) {
                g_autoptr(GVariant) bindings = value;
                if (!g_regex_match_simple("^[a-z0-9]+(?:_[a-z0-9]+)*$", name, 0, 0)) {
                    g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                                "invalid keybinding name: %s.%s", group, name);
                    g_variant_builder_clear(&actions);
                    goto invalid_keybindings;
                }
                g_autofree char* native_name = g_strdup(name);
                g_strdelimit(native_name, "_", '-');
                if (!gnoblin_keybinding_catalog_lookup(catalog, group, native_name)) {
                    if (meta_keybindings_gnoblin_action_supported(group, native_name))
                        g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                                    "keybinding is missing from the installed Gnoblin catalog: "
                                    "%s.%s",
                                    group, name);
                    else
                        g_set_error(error, G_IO_ERROR, G_IO_ERROR_NOT_SUPPORTED,
                                    "keybinding has no executable Mutter handler: %s.%s", group,
                                    name);
                    g_variant_builder_clear(&actions);
                    goto invalid_keybindings;
                }
                if (!meta_keybindings_gnoblin_action_supported(group, native_name)) {
                    g_set_error(error, G_IO_ERROR, G_IO_ERROR_NOT_SUPPORTED,
                                "keybinding has no executable Mutter handler: %s.%s", group, name);
                    g_variant_builder_clear(&actions);
                    goto invalid_keybindings;
                }
                g_autoptr(GVariant) native_bindings = native_binding_array(bindings);
                if (!native_bindings) {
                    g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                                "keybindings.%s.%s needs an array of accelerators", group, name);
                    g_variant_builder_clear(&actions);
                    goto invalid_keybindings;
                }
                g_variant_builder_add(&actions, "{sv}", native_name,
                                      g_variant_ref(native_bindings));
            }
            g_autoptr(GVariant) native_actions =
                g_variant_ref_sink(g_variant_builder_end(&actions));
            g_variant_builder_add(&groups, "{sv}", group, g_variant_ref(native_actions));
        }
    }
    normalized = g_variant_ref_sink(g_variant_builder_end(&groups));
    merged = merge_native_actions(normalized, document, catalog, error);
    if (!merged)
        return FALSE;
    /* Raw input policy dispatches configured pointer handlers directly to
     * Lua. Keep Mutter's legacy pointer registry empty so one physical press
     * cannot enter both routing paths. */
    if (!meta_keybindings_apply_gnoblin_pointer_bindings(control->display, NULL, error))
        return FALSE;
    meta_prefs_apply_gnoblin_keybindings(merged);
    return TRUE;

invalid_keybindings:
    g_variant_builder_clear(&groups);
    return FALSE;
}

gboolean start_native_shortcuts(GnoblinNativeControl* control, GVariant* document, GError** error) {
    g_autoptr(GVariant) declarations =
        document ? g_variant_lookup_value(document, "shortcuts", NULL) : NULL;
    if (!declarations)
        declarations = g_variant_ref_sink(g_variant_new_array(G_VARIANT_TYPE_VARIANT, NULL, 0));
    if (!g_variant_is_of_type(declarations, G_VARIANT_TYPE("av")) ||
        g_variant_n_children(declarations) > 256) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "shortcuts must be an array of at most 256 entries");
        return FALSE;
    }
    g_autoptr(GnoblinKeybindingCatalog) catalog = gnoblin_keybinding_catalog_load(error);
    if (!catalog)
        return FALSE;
    g_autoptr(GHashTable) names = g_hash_table_new_full(g_str_hash, g_str_equal, g_free, NULL);
    g_autoptr(GPtrArray) shortcuts = g_ptr_array_new_with_free_func(native_shortcut_free);
    gboolean has_overlay = FALSE;
    gboolean has_capture = FALSE;
    for (gsize index = 0; index < g_variant_n_children(declarations); index++) {
        g_autoptr(GVariant) boxed = g_variant_get_child_value(declarations, index);
        g_autoptr(GVariant) entry = g_variant_get_variant(boxed);
        g_autoptr(GVariant) command = NULL;
        g_autoptr(GVariant) binding_value = NULL;
        g_autoptr(GVariant) trigger_value = NULL;
        g_autoptr(GVariant) capture_value = NULL;
        g_autoptr(GVariant) action_value = NULL;
        const char* name = NULL;
        const char* binding = NULL;
        const char* trigger = "press";
        if (!g_variant_is_of_type(entry, G_VARIANT_TYPE_VARDICT))
            goto invalid_shortcut;
        GVariantIter fields;
        const char* key;
        GVariant* value;
        g_variant_iter_init(&fields, entry);
        while (g_variant_iter_next(&fields, "{&sv}", &key, &value)) {
            gboolean supported = g_str_equal(key, "name") || g_str_equal(key, "binding") ||
                                 g_str_equal(key, "command") || g_str_equal(key, "action") ||
                                 g_str_equal(key, "trigger") || g_str_equal(key, "capture-input");
            g_variant_unref(value);
            if (!supported)
                goto invalid_shortcut;
        }
        if (!g_variant_lookup(entry, "name", "&s", &name) || !*name || strlen(name) > 80 ||
            g_hash_table_contains(names, name))
            goto invalid_shortcut;
        for (const char* character = name; *character; character++)
            if (!g_ascii_isalnum(*character) && *character != '_' && *character != '-')
                goto invalid_shortcut;
        action_value = g_variant_lookup_value(entry, "action", NULL);
        capture_value = g_variant_lookup_value(entry, "capture-input", NULL);
        if (capture_value && g_variant_is_of_type(capture_value, G_VARIANT_TYPE_BOOLEAN) &&
            g_variant_get_boolean(capture_value)) {
            if (has_overlay || has_capture)
                goto invalid_shortcut;
            has_capture = TRUE;
            g_hash_table_add(names, g_strdup(name));
            continue;
        }
        if (action_value) {
            const char* group = NULL;
            g_autofree char* native_name = NULL;
            if (!native_action_target(action_value, catalog, &group, &native_name)) {
                g_set_error(error, G_IO_ERROR, G_IO_ERROR_NOT_SUPPORTED,
                            "shortcuts.%s.action is unsupported; use a command or a wm, mutter, "
                            "or wayland native action",
                            name ? name : "<unnamed>");
                goto invalid_shortcut;
            }
            binding_value = g_variant_lookup_value(entry, "binding", NULL);
            g_autoptr(GVariant) native_bindings = native_binding_array(binding_value);
            if (!native_bindings)
                goto invalid_shortcut;
            NativeShortcut* shortcut = g_new0(NativeShortcut, 1);
            shortcut->name = g_strdup(name);
            shortcut->action_id = g_strdup_printf("%s.%s", group, native_name);
            shortcut->bindings = g_variant_dup_strv(native_bindings, NULL);
            shortcut->binding_count = g_variant_n_children(native_bindings);
            shortcut->binding = shortcut->bindings[0] ? g_strdup(shortcut->bindings[0]) : NULL;
            shortcut->enabled = shortcut->bindings[0] != NULL;
            g_ptr_array_add(shortcuts, shortcut);
            g_hash_table_add(names, g_strdup(name));
            continue;
        }
        binding_value = g_variant_lookup_value(entry, "binding", NULL);
        if (!binding_value || !g_variant_is_of_type(binding_value, G_VARIANT_TYPE_STRING))
            goto invalid_shortcut;
        binding = g_variant_get_string(binding_value, NULL);
        if (!*binding || strlen(binding) > 160)
            goto invalid_shortcut;
        trigger_value = g_variant_lookup_value(entry, "trigger", NULL);
        if (trigger_value) {
            if (!g_variant_is_of_type(trigger_value, G_VARIANT_TYPE_STRING))
                goto invalid_shortcut;
            trigger = g_variant_get_string(trigger_value, NULL);
            if (!g_str_equal(trigger, "press") && !g_str_equal(trigger, "release"))
                goto invalid_shortcut;
        }
        if (capture_value) {
            if (!g_variant_is_of_type(capture_value, G_VARIANT_TYPE_BOOLEAN))
                goto invalid_shortcut;
        }
        command = g_variant_lookup_value(entry, "command", NULL);
        NativeShortcut* shortcut = g_new0(NativeShortcut, 1);
        shortcut->name = g_strdup(name);
        shortcut->binding = g_strdup(binding);
        shortcut->bindings = g_new0(char*, 2);
        shortcut->bindings[0] = g_strdup(binding);
        shortcut->binding_count = 1;
        shortcut->argv = native_command_argv(command);
        shortcut->enabled = TRUE;
        if (!shortcut->argv) {
            native_shortcut_free(shortcut);
            goto invalid_shortcut;
        }
        shortcut->overlay = g_str_equal(binding, "Super");
        shortcut->release = g_str_equal(trigger, "release");
        if (shortcut->overlay && (!shortcut->release || has_overlay || has_capture)) {
            native_shortcut_free(shortcut);
            goto invalid_shortcut;
        }
        has_overlay |= shortcut->overlay;
        g_hash_table_add(names, g_strdup(name));
        g_ptr_array_add(shortcuts, shortcut);
        continue;

    invalid_shortcut:
        if (!error || !*error)
            g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                        "shortcut entry %zu needs a unique name, binding, command array, and valid "
                        "trigger",
                        index + 1);
        return FALSE;
    }
    for (guint i = 0; i < shortcuts->len; i++) {
        NativeShortcut* shortcut = g_ptr_array_index(shortcuts, i);
        if (shortcut->overlay)
            continue;
        MetaKeyBindingFlags flags = META_KEY_BINDING_IGNORE_AUTOREPEAT;
        if (shortcut->release)
            flags |= META_KEY_BINDING_TRIGGER_RELEASE;
        shortcut->action =
            meta_display_grab_accelerator(control->display, shortcut->binding, flags);
        if (shortcut->action == META_KEYBINDING_ACTION_NONE) {
            g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                        "shortcut '%s' could not claim %s", shortcut->name, shortcut->binding);
            for (guint previous = 0; previous < i; previous++) {
                NativeShortcut* claimed = g_ptr_array_index(shortcuts, previous);
                if (!claimed->overlay)
                    meta_display_ungrab_accelerator(control->display, claimed->action);
            }
            return FALSE;
        }
    }
    /* native_settings_changed() applies the initial config before shortcut
     * registrations are initialized. Replace that provisional arm now. */
    gnoblin_control_clear_configured_capture_shortcut(control, "config_changed");
    if (!apply_configured_capture_shortcut(control, document, error))
        return FALSE;
    if (control->shortcuts)
        g_ptr_array_unref(control->shortcuts);
    control->shortcuts = g_steal_pointer(&shortcuts);
    if (!control->shortcut_handlers_connected) {
        meta_display_set_gnoblin_shortcut_activated_handler(
            control->display, native_trusted_shortcut_activated, control);
        g_signal_connect(control->display, "accelerator-activated",
                         G_CALLBACK(native_shortcut_activated), control);
        g_signal_connect(control->display, "accelerator-deactivated",
                         G_CALLBACK(native_shortcut_deactivated), control);
        g_signal_connect(control->display, "overlay-key", G_CALLBACK(native_overlay_key), control);
        control->shortcut_handlers_connected = TRUE;
    }
    g_autoptr(GVariant) shortcut_snapshot = native_shortcut_snapshot(control);
    native_publish_runtime_snapshot(control, "shortcuts", shortcut_snapshot,
                                    control->state_revision);
    return TRUE;
}

gboolean replace_native_shortcuts(GnoblinNativeControl* control, GVariant* document,
                                  GError** error) {
    GPtrArray* previous = control->shortcuts;
    control->shortcuts = NULL;
    if (previous) {
        for (guint i = 0; i < previous->len; i++) {
            NativeShortcut* shortcut = g_ptr_array_index(previous, i);
            if (!shortcut->overlay)
                meta_display_ungrab_accelerator(control->display, shortcut->action);
        }
    }

    if (start_native_shortcuts(control, document, error)) {
        if (previous)
            g_ptr_array_unref(previous);
        return TRUE;
    }

    control->shortcuts = previous;
    if (previous) {
        for (guint i = 0; i < previous->len; i++) {
            NativeShortcut* shortcut = g_ptr_array_index(previous, i);
            if (shortcut->overlay)
                continue;
            MetaKeyBindingFlags flags = META_KEY_BINDING_IGNORE_AUTOREPEAT;
            if (shortcut->release)
                flags |= META_KEY_BINDING_TRIGGER_RELEASE;
            shortcut->action =
                meta_display_grab_accelerator(control->display, shortcut->binding, flags);
        }
    }
    return FALSE;
}

static const InputField input_fields[] = {
    {"mouse", "speed", INPUT_DOUBLE, NULL, -1, 1},
    {"mouse", "double-click-time", INPUT_INTEGER, NULL, 0, G_MAXINT},
    {"mouse", "drag-threshold", INPUT_INTEGER, NULL, 1, G_MAXINT},
    {"mouse", "left-handed", INPUT_BOOLEAN},
    {"mouse", "natural-scroll", INPUT_BOOLEAN},
    {"mouse", "middle-click-emulation", INPUT_BOOLEAN},
    {"mouse", "accel-profile", INPUT_CHOICE, "default flat adaptive custom"},
    {"mouse", "accel-curve", INPUT_ACCEL_CURVE},
    {"touchpad", "speed", INPUT_DOUBLE, NULL, -1, 1},
    {"touchpad", "scroll-speed", INPUT_DOUBLE, NULL, 0, 2},
    {"touchpad", "left-handed", INPUT_CHOICE, "right left follow-mouse"},
    {"touchpad", "natural-scroll", INPUT_BOOLEAN},
    {"touchpad", "middle-click-emulation", INPUT_BOOLEAN},
    {"touchpad", "accel-profile", INPUT_CHOICE, "default flat adaptive custom"},
    {"touchpad", "accel-curve", INPUT_ACCEL_CURVE},
    {"touchpad", "tap-to-click", INPUT_BOOLEAN},
    {"touchpad", "tap-button-map", INPUT_CHOICE, "default left-right-middle left-middle-right"},
    {"touchpad", "tap-and-drag", INPUT_BOOLEAN},
    {"touchpad", "tap-and-drag-lock", INPUT_BOOLEAN},
    {"touchpad", "disable-while-typing", INPUT_BOOLEAN},
    {"touchpad", "disable-while-typing-timeout", INPUT_MILLISECONDS, NULL, 100, 5000},
    {"touchpad", "edge-scrolling-enabled", INPUT_BOOLEAN},
    {"touchpad", "two-finger-scrolling-enabled", INPUT_BOOLEAN},
    {"touchpad", "click-method", INPUT_CHOICE, "default none areas fingers"},
    {"trackball", "accel-profile", INPUT_CHOICE, "default flat adaptive"},
    {"trackball", "middle-click-emulation", INPUT_BOOLEAN},
    {"trackball", "scroll-wheel-emulation-button", INPUT_INTEGER, NULL, 0, 24},
    {"trackball", "scroll-wheel-emulation-button-lock", INPUT_BOOLEAN},
    {"pointing-stick", "speed", INPUT_DOUBLE, NULL, -1, 1},
    {"pointing-stick", "accel-profile", INPUT_CHOICE, "default flat adaptive"},
    {"pointing-stick", "scroll-method", INPUT_CHOICE, "default none on-button-down"},
    {"keyboard", "repeat", INPUT_BOOLEAN},
    {"keyboard", "delay", INPUT_MILLISECONDS, NULL, 1, 10000},
    {"keyboard", "repeat-interval", INPUT_MILLISECONDS, NULL, 1, 10000},
    {"keyboard", "remember-numlock-state", INPUT_BOOLEAN},
    {"keyboard", "numlock-state", INPUT_BOOLEAN},
    {"keyboard", "xkb-options", INPUT_STRINGS},
    {"keyboard", "accessibility", INPUT_ACCESSIBILITY},
    {"keyboard-accessibility", "shortcuts-enabled", INPUT_BOOLEAN},
    {"keyboard-accessibility", "beep-on-feature-state-change", INPUT_BOOLEAN},
    {"keyboard-accessibility", "bounce-keys.enabled", INPUT_BOOLEAN},
    {"keyboard-accessibility", "bounce-keys.delay-ms", INPUT_INTEGER, NULL, 0, 10000},
    {"keyboard-accessibility", "bounce-keys.beep-on-reject", INPUT_BOOLEAN},
    {"keyboard-accessibility", "mouse-keys.enabled", INPUT_BOOLEAN},
    {"keyboard-accessibility", "mouse-keys.max-speed", INPUT_INTEGER, NULL, 1, 10000},
    {"keyboard-accessibility", "mouse-keys.acceleration-time-ms", INPUT_INTEGER, NULL, 1, 10000},
    {"keyboard-accessibility", "mouse-keys.initial-delay-ms", INPUT_INTEGER, NULL, 0, 10000},
    {"keyboard-accessibility", "slow-keys.enabled", INPUT_BOOLEAN},
    {"keyboard-accessibility", "slow-keys.delay-ms", INPUT_INTEGER, NULL, 0, 10000},
    {"keyboard-accessibility", "slow-keys.beep-on-press", INPUT_BOOLEAN},
    {"keyboard-accessibility", "slow-keys.beep-on-accept", INPUT_BOOLEAN},
    {"keyboard-accessibility", "slow-keys.beep-on-reject", INPUT_BOOLEAN},
    {"keyboard-accessibility", "sticky-keys.enabled", INPUT_BOOLEAN},
    {"keyboard-accessibility", "sticky-keys.two-key-off", INPUT_BOOLEAN},
    {"keyboard-accessibility", "sticky-keys.beep-on-modifier", INPUT_BOOLEAN},
    {"keyboard-accessibility", "toggle-keys.enabled", INPUT_BOOLEAN},
    {"tablets", "mapping", INPUT_CHOICE, "absolute relative"},
    {"tablets", "output", INPUT_TABLET_OUTPUT},
    {"tablets", "left-handed", INPUT_BOOLEAN},
    {"tablets", "keep-aspect", INPUT_BOOLEAN},
    {"tablets", "area", INPUT_TABLET_AREA},
    {"tablets", "pad-buttons", INPUT_PAD_BUTTONS},
    {"styluses", "eraser-button-mode", INPUT_CHOICE, "default button"},
    {"styluses", "eraser-button-action", INPUT_CHOICE,
     "default middle right back forward switch-monitor keybinding"},
    {"styluses", "eraser-button-keybinding", INPUT_STRING},
    {"styluses", "button-action", INPUT_CHOICE,
     "default middle right back forward switch-monitor keybinding"},
    {"styluses", "secondary-button-action", INPUT_CHOICE,
     "default middle right back forward switch-monitor keybinding"},
    {"styluses", "tertiary-button-action", INPUT_CHOICE,
     "default middle right back forward switch-monitor keybinding"},
    {"styluses", "button-keybinding", INPUT_STRING},
    {"styluses", "secondary-button-keybinding", INPUT_STRING},
    {"styluses", "tertiary-button-keybinding", INPUT_STRING},
};

static const InputField* find_input_field(const char* group, const char* name) {
    for (guint i = 0; i < G_N_ELEMENTS(input_fields); i++)
        if (g_str_equal(input_fields[i].group, group) && g_str_equal(input_fields[i].name, name))
            return &input_fields[i];
    return NULL;
}

static gboolean input_is_inherit(GVariant* value) {
    return g_variant_is_of_type(value, G_VARIANT_TYPE_STRING) &&
           g_str_equal(g_variant_get_string(value, NULL), "inherit");
}

static gboolean input_accessibility_section_known(const char* section) {
    g_autofree char* prefix = g_strdup_printf("%s.", section);
    for (guint i = 0; i < G_N_ELEMENTS(input_fields); i++)
        if (g_str_equal(input_fields[i].group, "keyboard-accessibility") &&
            g_str_has_prefix(input_fields[i].name, prefix))
            return TRUE;
    return FALSE;
}

static gboolean input_numeric_value(GVariant* value, double* number) {
    if (g_variant_is_of_type(value, G_VARIANT_TYPE_DOUBLE))
        *number = g_variant_get_double(value);
    else if (g_variant_is_of_type(value, G_VARIANT_TYPE_INT64))
        *number = (double)g_variant_get_int64(value);
    else if (g_variant_is_of_type(value, G_VARIANT_TYPE_INT32))
        *number = (double)g_variant_get_int32(value);
    else if (g_variant_is_of_type(value, G_VARIANT_TYPE_UINT32))
        *number = (double)g_variant_get_uint32(value);
    else
        return FALSE;

    return isfinite(*number);
}

static GVariant* normalize_tablet_pad_buttons(GVariant* value, GError** error) {
    static const char* actions[] = {"default",        "none",       "help",
                                    "switch-monitor", "keybinding", NULL};
    if (!g_variant_is_of_type(value, G_VARIANT_TYPE("av")) || g_variant_n_children(value) > 256) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "input.tablets.pad-buttons must be an array with at most 256 entries");
        return NULL;
    }

    gboolean seen[256] = {FALSE};
    GVariantBuilder buttons;
    g_variant_builder_init(&buttons, G_VARIANT_TYPE("av"));
    for (gsize i = 0; i < g_variant_n_children(value); i++) {
        g_autoptr(GVariant) boxed = g_variant_get_child_value(value, i);
        g_autoptr(GVariant) button = g_variant_get_variant(boxed);
        g_autoptr(GVariant) number_value = NULL;
        g_autoptr(GVariant) action_value = NULL;
        g_autoptr(GVariant) keybinding_value = NULL;
        double number;
        guint button_id;
        if (!g_variant_is_of_type(button, G_VARIANT_TYPE_VARDICT))
            goto invalid_button;

        number_value = g_variant_lookup_value(button, "button", NULL);
        action_value = g_variant_lookup_value(button, "action", G_VARIANT_TYPE_STRING);
        keybinding_value = g_variant_lookup_value(button, "keybinding", NULL);
        if (!number_value || !input_numeric_value(number_value, &number) || number < 0 ||
            number > 255 || number != floor(number) || !action_value)
            goto invalid_button;
        button_id = (guint)number;
        if (seen[button_id])
            goto invalid_button;
        seen[button_id] = TRUE;

        const char* action = g_variant_get_string(action_value, NULL);
        if (!g_strv_contains(actions, action))
            goto invalid_button;
        if (g_str_equal(action, "keybinding")) {
            if (!keybinding_value || !g_variant_is_of_type(keybinding_value, G_VARIANT_TYPE_STRING))
                goto invalid_button;
            const char* keybinding = g_variant_get_string(keybinding_value, NULL);
            if (!*keybinding || strlen(keybinding) > 160)
                goto invalid_button;
        } else if (keybinding_value) {
            goto invalid_button;
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
                goto invalid_button;
        }

        GVariantBuilder normalized;
        g_variant_builder_init(&normalized, G_VARIANT_TYPE_VARDICT);
        g_variant_builder_add(&normalized, "{sv}", "button", g_variant_new_uint32(button_id));
        g_variant_builder_add(&normalized, "{sv}", "action", action_value);
        if (keybinding_value)
            g_variant_builder_add(&normalized, "{sv}", "keybinding", keybinding_value);
        g_variant_builder_add_value(&buttons,
                                    g_variant_new_variant(g_variant_builder_end(&normalized)));
        continue;

    invalid_button:
        g_variant_builder_clear(&buttons);
        g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                    "input.tablets.pad-buttons[%zu] needs a unique button from 0 to 255, a valid "
                    "action, and a keybinding only when action is 'keybinding'",
                    i + 1);
        return NULL;
    }

    return g_variant_ref_sink(g_variant_builder_end(&buttons));
}

static GVariant* normalize_tablet_area(GVariant* value, GError** error) {
    double values[4];

    if (g_variant_is_of_type(value, G_VARIANT_TYPE("ad"))) {
        gsize length = 0;
        const double* array = g_variant_get_fixed_array(value, &length, sizeof(double));
        if (length != G_N_ELEMENTS(values))
            goto invalid_area;
        for (gsize i = 0; i < G_N_ELEMENTS(values); i++)
            values[i] = array[i];
    } else if (g_variant_is_of_type(value, G_VARIANT_TYPE("av")) &&
               g_variant_n_children(value) == G_N_ELEMENTS(values)) {
        for (gsize i = 0; i < G_N_ELEMENTS(values); i++) {
            g_autoptr(GVariant) boxed = g_variant_get_child_value(value, i);
            g_autoptr(GVariant) item = g_variant_get_variant(boxed);
            if (!input_numeric_value(item, &values[i]))
                goto invalid_area;
        }
    } else {
        goto invalid_area;
    }

    for (gsize i = 0; i < G_N_ELEMENTS(values); i++)
        if (!isfinite(values[i]) || values[i] < 0 || values[i] >= 1)
            goto invalid_area;
    if (values[0] + values[1] >= 1 || values[2] + values[3] >= 1)
        goto invalid_area;

    return g_variant_ref_sink(g_variant_new_fixed_array(G_VARIANT_TYPE_DOUBLE, values,
                                                        G_N_ELEMENTS(values), sizeof(double)));

invalid_area:
    g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                        "input.tablets.area must contain four fractions in [0, 1), with opposing "
                        "edges totaling less than 1");
    return NULL;
}

static GVariant* normalize_input_value(const InputField* field, GVariant* value, GError** error) {
    if (field->kind == INPUT_ACCESSIBILITY)
        return normalize_input_accessibility(value, error);
    if (field->kind == INPUT_PAD_BUTTONS)
        return normalize_tablet_pad_buttons(value, error);
    if (field->kind == INPUT_TABLET_AREA)
        return normalize_tablet_area(value, error);
    if (field->kind == INPUT_TABLET_OUTPUT && g_variant_is_of_type(value, G_VARIANT_TYPE_STRING)) {
        const char* output = g_variant_get_string(value, NULL);
        if (g_str_equal(output, "auto") ||
            g_regex_match_simple("^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$", output, G_REGEX_OPTIMIZE, 0))
            return g_variant_ref(value);
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "input.tablets.output must be 'auto' or a monitor connector ID");
        return NULL;
    }
    if (field->kind == INPUT_ACCEL_CURVE && g_variant_is_of_type(value, G_VARIANT_TYPE_VARDICT)) {
        GVariantBuilder curve;
        GVariantIter iter;
        const char* name;
        GVariant* member;
        g_autoptr(GVariant) step_value = NULL;
        g_autoptr(GVariant) points_value = NULL;
        double step;
        GArray* points = g_array_new(FALSE, FALSE, sizeof(double));
        g_variant_iter_init(&iter, value);
        while (g_variant_iter_next(&iter, "{&sv}", &name, &member)) {
            g_autoptr(GVariant) current = member;
            if (g_str_equal(name, "step"))
                step_value = g_steal_pointer(&current);
            else if (g_str_equal(name, "points"))
                points_value = g_steal_pointer(&current);
            else {
                g_array_unref(points);
                return NULL;
            }
        }
        if (!step_value || !points_value) {
            g_array_unref(points);
            return NULL;
        }
        if (g_variant_is_of_type(step_value, G_VARIANT_TYPE_DOUBLE))
            step = g_variant_get_double(step_value);
        else if (g_variant_is_of_type(step_value, G_VARIANT_TYPE_INT64))
            step = (double)g_variant_get_int64(step_value);
        else {
            g_array_unref(points);
            return NULL;
        }
        if (!isfinite(step) || step <= 0) {
            g_array_unref(points);
            return NULL;
        }
        if (g_variant_is_of_type(points_value, G_VARIANT_TYPE("ad"))) {
            gsize n_points;
            const double* values =
                g_variant_get_fixed_array(points_value, &n_points, sizeof(double));
            for (gsize i = 0; i < n_points; i++) {
                if (!isfinite(values[i]) || values[i] < 0) {
                    g_array_unref(points);
                    return NULL;
                }
                g_array_append_val(points, values[i]);
            }
        } else if (g_variant_is_of_type(points_value, G_VARIANT_TYPE("av"))) {
            for (gsize i = 0; i < g_variant_n_children(points_value); i++) {
                g_autoptr(GVariant) boxed = g_variant_get_child_value(points_value, i);
                g_autoptr(GVariant) point = g_variant_get_variant(boxed);
                double number;
                if (g_variant_is_of_type(point, G_VARIANT_TYPE_DOUBLE))
                    number = g_variant_get_double(point);
                else if (g_variant_is_of_type(point, G_VARIANT_TYPE_INT64))
                    number = (double)g_variant_get_int64(point);
                else {
                    g_array_unref(points);
                    return NULL;
                }
                if (!isfinite(number) || number < 0) {
                    g_array_unref(points);
                    return NULL;
                }
                g_array_append_val(points, number);
            }
        } else {
            g_array_unref(points);
            return NULL;
        }
        if (points->len < 2) {
            g_array_unref(points);
            return NULL;
        }
        g_variant_builder_init(&curve, G_VARIANT_TYPE_VARDICT);
        g_variant_builder_add(&curve, "{sv}", "step", g_variant_new_double(step));
        g_variant_builder_add(&curve, "{sv}", "points",
                              g_variant_new_fixed_array(G_VARIANT_TYPE_DOUBLE, points->data,
                                                        points->len, sizeof(double)));
        g_array_unref(points);
        return g_variant_ref_sink(g_variant_builder_end(&curve));
    }
    if (field->kind == INPUT_BOOLEAN && g_variant_is_of_type(value, G_VARIANT_TYPE_BOOLEAN))
        return g_variant_ref(value);
    if (field->kind == INPUT_STRING && g_variant_is_of_type(value, G_VARIANT_TYPE_STRING))
        return g_variant_ref(value);
    if (field->kind == INPUT_CHOICE && g_variant_is_of_type(value, G_VARIANT_TYPE_STRING)) {
        g_auto(GStrv) choices = g_strsplit(field->choices, " ", -1);
        if (g_strv_contains((const char* const*)choices, g_variant_get_string(value, NULL)))
            return g_variant_ref(value);
    }
    if (field->kind == INPUT_DOUBLE) {
        double number;
        if (g_variant_is_of_type(value, G_VARIANT_TYPE_DOUBLE))
            number = g_variant_get_double(value);
        else if (g_variant_is_of_type(value, G_VARIANT_TYPE_INT64))
            number = (double)g_variant_get_int64(value);
        else
            return NULL;
        if (isfinite(number) && number >= field->minimum && number <= field->maximum)
            return g_variant_ref_sink(g_variant_new_double(number));
    }
    if (field->kind == INPUT_INTEGER) {
        double number;
        if (g_variant_is_of_type(value, G_VARIANT_TYPE_INT64))
            number = (double)g_variant_get_int64(value);
        else if (g_variant_is_of_type(value, G_VARIANT_TYPE_INT32))
            number = (double)g_variant_get_int32(value);
        else if (g_variant_is_of_type(value, G_VARIANT_TYPE_UINT32))
            number = (double)g_variant_get_uint32(value);
        else if (g_variant_is_of_type(value, G_VARIANT_TYPE_DOUBLE))
            number = g_variant_get_double(value);
        else
            return NULL;
        if (isfinite(number) && number >= field->minimum && number <= field->maximum &&
            number == floor(number))
            return g_variant_ref_sink(g_variant_new_int32((gint32)number));
    }
    if (field->kind == INPUT_MILLISECONDS) {
        gint64 milliseconds;
        if (g_variant_is_of_type(value, G_VARIANT_TYPE_INT64))
            milliseconds = g_variant_get_int64(value);
        else if (g_variant_is_of_type(value, G_VARIANT_TYPE_UINT32))
            milliseconds = g_variant_get_uint32(value);
        else
            return NULL;
        if (milliseconds >= field->minimum && milliseconds <= field->maximum)
            return g_variant_ref_sink(g_variant_new_uint32((guint32)milliseconds));
    }
    if (field->kind == INPUT_STRINGS && g_variant_is_of_type(value, G_VARIANT_TYPE("av"))) {
        g_auto(GStrv) strings = g_new0(char*, g_variant_n_children(value) + 1);
        for (gsize i = 0; i < g_variant_n_children(value); i++) {
            g_autoptr(GVariant) boxed = g_variant_get_child_value(value, i);
            g_autoptr(GVariant) item = g_variant_get_variant(boxed);
            if (!g_variant_is_of_type(item, G_VARIANT_TYPE_STRING))
                return NULL;
            strings[i] = g_variant_dup_string(item, NULL);
        }
        return g_variant_ref_sink(g_variant_new_strv((const char* const*)strings, -1));
    }
    return NULL;
}

static GVariant* normalize_input_accessibility(GVariant* accessibility, GError** error) {
    if (!g_variant_is_of_type(accessibility, G_VARIANT_TYPE_VARDICT)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "input.keyboard.accessibility must be a table");
        return NULL;
    }

    GVariantBuilder normalized;
    GVariantIter iter;
    const char* name;
    GVariant* value;
    g_variant_builder_init(&normalized, G_VARIANT_TYPE_VARDICT);
    g_variant_iter_init(&iter, accessibility);
    while (g_variant_iter_next(&iter, "{&sv}", &name, &value)) {
        g_autoptr(GVariant) member = value;
        if (input_is_inherit(member))
            continue;

        const InputField* field = find_input_field("keyboard-accessibility", name);
        if (field) {
            g_autoptr(GVariant) converted = normalize_input_value(field, member, error);
            if (!converted) {
                if (!error || !*error)
                    g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                                "input.keyboard.accessibility.%s: invalid value", name);
                g_variant_builder_clear(&normalized);
                return NULL;
            }
            g_variant_builder_add(&normalized, "{sv}", name, g_variant_ref(converted));
            continue;
        }

        if (!input_accessibility_section_known(name)) {
            g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                        "unknown input setting: input.keyboard.accessibility.%s", name);
            g_variant_builder_clear(&normalized);
            return NULL;
        }
        if (!g_variant_is_of_type(member, G_VARIANT_TYPE_VARDICT)) {
            g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                        "input.keyboard.accessibility.%s must be a table or inherit", name);
            g_variant_builder_clear(&normalized);
            return NULL;
        }

        GVariantBuilder section;
        GVariantIter section_iter;
        const char* field_name;
        GVariant* field_value;
        g_variant_builder_init(&section, G_VARIANT_TYPE_VARDICT);
        g_variant_iter_init(&section_iter, member);
        while (g_variant_iter_next(&section_iter, "{&sv}", &field_name, &field_value)) {
            g_autoptr(GVariant) setting = field_value;
            if (input_is_inherit(setting))
                continue;
            g_autofree char* qualified_name = g_strdup_printf("%s.%s", name, field_name);
            field = find_input_field("keyboard-accessibility", qualified_name);
            g_autoptr(GVariant) converted =
                field ? normalize_input_value(field, setting, error) : NULL;
            if (!converted) {
                if (!error || !*error)
                    g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                                "input.keyboard.accessibility.%s.%s is unsupported or invalid",
                                name, field_name);
                g_variant_builder_clear(&section);
                g_variant_builder_clear(&normalized);
                return NULL;
            }
            g_variant_builder_add(&section, "{sv}", field_name, g_variant_ref(converted));
        }
        g_autoptr(GVariant) converted_section = g_variant_ref_sink(g_variant_builder_end(&section));
        if (g_variant_n_children(converted_section) > 0)
            g_variant_builder_add(&normalized, "{sv}", name, g_variant_ref(converted_section));
    }
    return g_variant_ref_sink(g_variant_builder_end(&normalized));
}

GVariant* normalize_input_fields(const char* group, GVariant* fields, GError** error) {
    if (!g_variant_is_of_type(fields, G_VARIANT_TYPE_VARDICT))
        goto invalid_group;
    GVariantBuilder converted;
    GVariantIter iter;
    const char* name;
    GVariant* value;
    g_variant_builder_init(&converted, G_VARIANT_TYPE_VARDICT);
    g_variant_iter_init(&iter, fields);
    while (g_variant_iter_next(&iter, "{&sv}", &name, &value)) {
        g_autoptr(GVariant) current = value;
        const InputField* field = find_input_field(group, name);
        if (!field) {
            g_variant_builder_clear(&converted);
            g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                        "unknown input setting: input.%s.%s", group, name);
            return NULL;
        }
        if (g_variant_is_of_type(current, G_VARIANT_TYPE_STRING) &&
            g_str_equal(g_variant_get_string(current, NULL), "inherit"))
            continue;
        g_autoptr(GVariant) normalized = normalize_input_value(field, current, error);
        if (!normalized) {
            g_variant_builder_clear(&converted);
            if (!error || !*error)
                g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "input.%s.%s: invalid value", group, name);
            return NULL;
        }
        g_variant_builder_add(&converted, "{sv}", name, g_variant_ref(normalized));
    }
    return g_variant_ref_sink(g_variant_builder_end(&converted));

invalid_group:
    g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT, "input.%s must be a table", group);
    return NULL;
}

gboolean valid_input_device(const char* group, const char* device) {
    return g_regex_match_simple(g_str_equal(group, "tablets")
                                    ? "^[0-9a-fA-F]{4}:[0-9a-fA-F]{4}$"
                                    : "^(?:[0-9a-fA-F]+|default-[0-9a-fA-F]{4}:[0-9a-fA-F]{4})$",
                                device, G_REGEX_OPTIMIZE, 0);
}

static const char* native_orientation_name(MetaOrientation orientation) {
    switch (orientation) {
    case META_ORIENTATION_NORMAL:
        return "normal";
    case META_ORIENTATION_BOTTOM_UP:
        return "bottom-up";
    case META_ORIENTATION_LEFT_UP:
        return "left-up";
    case META_ORIENTATION_RIGHT_UP:
        return "right-up";
    case META_ORIENTATION_UNDEFINED:
    default:
        return "undefined";
    }
}

GVariant* native_orientation_lock_snapshot(GnoblinNativeControl* control, guint64 revision) {
    MetaOrientationManager* manager = control ? control->orientation_manager : NULL;
    gboolean available = manager && meta_orientation_manager_has_accelerometer(manager);
    const char* source = control && control->orientation_lock_runtime_override ? "runtime"
                         : control && control->orientation_lock_configured     ? "config"
                                                                               : "system";
    GVariantBuilder builder;
    g_variant_builder_init(&builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&builder, "{sv}", "available", g_variant_new_boolean(available));
    g_variant_builder_add(
        &builder, "{sv}", "locked",
        g_variant_new_boolean(available &&
                              meta_orientation_manager_get_orientation_locked(manager)));
    g_variant_builder_add(
        &builder, "{sv}", "orientation",
        g_variant_new_string(
            available ? native_orientation_name(meta_orientation_manager_get_orientation(manager))
                      : "undefined"));
    g_variant_builder_add(&builder, "{sv}", "source", g_variant_new_string(source));
    g_variant_builder_add(&builder, "{sv}", "revision", g_variant_new_uint64(revision));
    return g_variant_ref_sink(g_variant_builder_end(&builder));
}

void native_orientation_lock_publish(GnoblinNativeControl* control) {
    if (!control || control->stopping)
        return;

    g_autoptr(GVariant) candidate =
        native_orientation_lock_snapshot(control, control->orientation_lock_revision);
    gboolean initialized = control->orientation_lock_snapshot != NULL;
    gboolean changed =
        !initialized || !g_variant_equal(control->orientation_lock_snapshot, candidate);
    if (!changed)
        return;

    if (control->orientation_lock_revision < G_MAXUINT64)
        control->orientation_lock_revision++;
    g_clear_pointer(&control->orientation_lock_snapshot, g_variant_unref);
    control->orientation_lock_snapshot =
        native_orientation_lock_snapshot(control, control->orientation_lock_revision);
    native_publish_runtime_snapshot(control, "input-orientation-lock",
                                    control->orientation_lock_snapshot,
                                    control->orientation_lock_revision);

    /* The first value seeds the Lua worker; only later changes are events. */
    if (!initialized)
        return;

    guint64 sequence = ++control->event_sequence;
    gint64 time = g_get_monotonic_time();
    GVariantBuilder event_builder;
    GVariantIter fields;
    const char* field_name;
    GVariant* field_value;
    g_variant_builder_init(&event_builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&event_builder, "{sv}", "name",
                          g_variant_new_string("gnoblin.input.orientation-lock-changed"));
    g_variant_iter_init(&fields, control->orientation_lock_snapshot);
    while (g_variant_iter_next(&fields, "{&sv}", &field_name, &field_value)) {
        g_autoptr(GVariant) value = field_value;
        g_variant_builder_add(&event_builder, "{sv}", field_name, g_variant_ref(value));
    }
    g_variant_builder_add(&event_builder, "{sv}", "sequence", g_variant_new_uint64(sequence));
    g_variant_builder_add(&event_builder, "{sv}", "time", g_variant_new_int64(time));
    g_autoptr(GVariant) event_payload = g_variant_ref_sink(g_variant_builder_end(&event_builder));
    g_autoptr(JsonNode) json = json_from_variant(event_payload);
    if (JSON_NODE_HOLDS_OBJECT(json))
        publish_native_socket_event(control, json);
    native_runtime_dispatch_event(control, "gnoblin.input.orientation-lock-changed", event_payload);
}

GVariant* native_monitor_privacy_screen_snapshot(GnoblinNativeControl* control, guint64 revision) {
    MetaSettings* settings = meta_backend_get_settings(control->backend);
    const char* source = control->monitor_privacy_screen_runtime_override ? "runtime" : "system";
    gboolean configured = FALSE;
    g_autoptr(GVariant) document = gnoblin_runtime_cache_get_document(control->runtime_cache);
    g_autoptr(GVariant) preferences =
        document ? g_variant_lookup_value(document, "monitors", G_VARIANT_TYPE_VARDICT) : NULL;
    if (!control->monitor_privacy_screen_runtime_override && preferences &&
        g_variant_lookup(preferences, "privacy-screen", "b", &configured))
        source = "config";

    GVariantBuilder monitors;
    g_variant_builder_init(&monitors, G_VARIANT_TYPE("av"));
    GList* logical_monitors =
        control->monitor_manager
            ? meta_monitor_manager_get_logical_monitors(control->monitor_manager)
            : NULL;
    for (GList* logical_item = logical_monitors; logical_item; logical_item = logical_item->next) {
        MetaLogicalMonitor* logical_monitor = logical_item->data;
        GList* monitor_list = meta_logical_monitor_get_monitors(logical_monitor);
        for (GList* monitor_item = monitor_list; monitor_item; monitor_item = monitor_item->next) {
            MetaMonitor* monitor = monitor_item->data;
            if (!meta_monitor_is_active(monitor))
                continue;

            const char* connector = meta_monitor_get_connector(monitor);
            if (!connector || !*connector)
                continue;

            MetaPrivacyScreenState state = meta_monitor_get_privacy_screen_state(monitor);
            gboolean available = state != META_PRIVACY_SCREEN_UNAVAILABLE;
            GVariantBuilder record;
            g_variant_builder_init(&record, G_VARIANT_TYPE_VARDICT);
            g_variant_builder_add(&record, "{sv}", "id", g_variant_new_string(connector));
            g_variant_builder_add(&record, "{sv}", "available", g_variant_new_boolean(available));
            if (available) {
                g_variant_builder_add(
                    &record, "{sv}", "enabled",
                    g_variant_new_boolean(!!(state & META_PRIVACY_SCREEN_ENABLED)));
                g_variant_builder_add(
                    &record, "{sv}", "locked",
                    g_variant_new_boolean(!!(state & META_PRIVACY_SCREEN_LOCKED)));
            }
            g_variant_builder_add_value(&monitors,
                                        g_variant_new_variant(g_variant_builder_end(&record)));
        }
    }

    GVariantBuilder snapshot;
    g_variant_builder_init(&snapshot, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(
        &snapshot, "{sv}", "requested_enabled",
        g_variant_new_boolean(settings && meta_settings_is_privacy_screen_enabled(settings)));
    g_variant_builder_add(&snapshot, "{sv}", "source", g_variant_new_string(source));
    g_variant_builder_add(&snapshot, "{sv}", "revision", g_variant_new_uint64(revision));
    g_variant_builder_add(&snapshot, "{sv}", "monitors", g_variant_builder_end(&monitors));
    return g_variant_ref_sink(g_variant_builder_end(&snapshot));
}

void native_monitor_privacy_screen_publish(GnoblinNativeControl* control) {
    if (!control || control->stopping)
        return;

    g_autoptr(GVariant) candidate =
        native_monitor_privacy_screen_snapshot(control, control->monitor_privacy_screen_revision);
    gboolean initialized = control->monitor_privacy_screen_snapshot != NULL;
    if (initialized && g_variant_equal(control->monitor_privacy_screen_snapshot, candidate))
        return;

    if (control->monitor_privacy_screen_revision < G_MAXUINT64)
        control->monitor_privacy_screen_revision++;
    g_clear_pointer(&control->monitor_privacy_screen_snapshot, g_variant_unref);
    control->monitor_privacy_screen_snapshot =
        native_monitor_privacy_screen_snapshot(control, control->monitor_privacy_screen_revision);
    native_publish_runtime_snapshot(control, "monitor-privacy-screen",
                                    control->monitor_privacy_screen_snapshot,
                                    control->monitor_privacy_screen_revision);

    /* The first value seeds the Lua worker; only later changes are events. */
    if (!initialized)
        return;

    guint64 sequence = ++control->event_sequence;
    gint64 time = g_get_monotonic_time();
    GVariantBuilder event_builder;
    GVariantIter fields;
    const char* field_name;
    GVariant* field_value;
    g_variant_builder_init(&event_builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&event_builder, "{sv}", "name",
                          g_variant_new_string("gnoblin.monitor.privacy-screen-changed"));
    g_variant_iter_init(&fields, control->monitor_privacy_screen_snapshot);
    while (g_variant_iter_next(&fields, "{&sv}", &field_name, &field_value)) {
        g_autoptr(GVariant) value = field_value;
        g_variant_builder_add(&event_builder, "{sv}", field_name, g_variant_ref(value));
    }
    g_variant_builder_add(&event_builder, "{sv}", "sequence", g_variant_new_uint64(sequence));
    g_variant_builder_add(&event_builder, "{sv}", "time", g_variant_new_int64(time));
    g_autoptr(GVariant) event_payload = g_variant_ref_sink(g_variant_builder_end(&event_builder));
    g_autoptr(JsonNode) json = json_from_variant(event_payload);
    if (JSON_NODE_HOLDS_OBJECT(json))
        publish_native_socket_event(control, json);
    native_runtime_dispatch_event(control, "gnoblin.monitor.privacy-screen-changed", event_payload);
}

void native_orientation_manager_notified(GObject* manager, GParamSpec* property,
                                         gpointer user_data) {
    (void)manager;
    (void)property;
    GnoblinNativeControl* control = user_data;
    if (control && !control->orientation_lock_notifications_suppressed)
        native_orientation_lock_publish(control);
}

void native_orientation_manager_accelerometer_notified(GObject* manager, GParamSpec* property,
                                                       gpointer user_data) {
    native_orientation_manager_notified(manager, property, user_data);
}

void native_orientation_manager_orientation_changed(MetaOrientationManager* manager,
                                                    gpointer user_data) {
    (void)manager;
    GnoblinNativeControl* control = user_data;
    if (control && !control->orientation_lock_notifications_suppressed)
        native_orientation_lock_publish(control);
}
