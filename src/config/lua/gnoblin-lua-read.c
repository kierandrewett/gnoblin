#include "gnoblin-lua-internal.h"

static GVariant* legacy_workspace_list_from_lua(GVariant* workspaces, GError** error) {
    if (!workspaces || !g_variant_is_of_type(workspaces, G_VARIANT_TYPE("av"))) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                            "Lua workspace snapshot is invalid");
        return NULL;
    }

    GVariantBuilder workspace_records;
    g_variant_builder_init(&workspace_records, G_VARIANT_TYPE("aa{sv}"));
    for (gsize i = 0; i < g_variant_n_children(workspaces); i++) {
        g_autoptr(GVariant) boxed = g_variant_get_child_value(workspaces, i);
        g_autoptr(GVariant) workspace = g_variant_get_variant(boxed);
        if (!g_variant_is_of_type(workspace, G_VARIANT_TYPE_VARDICT)) {
            g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                                "Lua workspace record is invalid");
            return NULL;
        }

        GVariantBuilder record;
        g_variant_builder_init(&record, G_VARIANT_TYPE_VARDICT);
        GVariantIter iter;
        const char* key;
        GVariant* value;
        g_variant_iter_init(&iter, workspace);
        while (g_variant_iter_next(&iter, "{&sv}", &key, &value)) {
            g_autoptr(GVariant) field = value;
            if (g_str_equal(key, "revision"))
                continue;
            g_variant_builder_add(&record, "{sv}",
                                  g_str_equal(key, "window_count") ? "windows" : key, field);
        }
        g_variant_builder_add_value(&workspace_records, g_variant_builder_end(&record));
    }

    GVariantBuilder result;
    g_variant_builder_init(&result, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&result, "{sv}", "workspaces", g_variant_builder_end(&workspace_records));
    return g_variant_ref_sink(g_variant_builder_end(&result));
}

static gboolean legacy_window_list_arguments_valid(GVariant* arguments, GError** error) {
    if (!arguments || !g_variant_is_of_type(arguments, G_VARIANT_TYPE_VARDICT) ||
        g_variant_n_children(arguments) > 3) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                            "window.list accepts optional app_id, title, and focused filters");
        return FALSE;
    }

    GVariantIter iter;
    const char* key;
    GVariant* value;
    g_variant_iter_init(&iter, arguments);
    while (g_variant_iter_next(&iter, "{&sv}", &key, &value)) {
        g_autoptr(GVariant) field = value;
        if (g_str_equal(key, "focused")) {
            if (!g_variant_is_of_type(field, G_VARIANT_TYPE_BOOLEAN))
                goto invalid;
        } else if ((g_str_equal(key, "app_id") || g_str_equal(key, "title")) &&
                   g_variant_is_of_type(field, G_VARIANT_TYPE_STRING)) {
            continue;
        } else {
            goto invalid;
        }
    }
    return TRUE;

invalid:
    g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                        "window.list accepts optional app_id, title, and focused filters");
    return FALSE;
}

static gboolean legacy_window_match_arguments_valid(GVariant* arguments, GError** error) {
    if (!arguments || !g_variant_is_of_type(arguments, G_VARIANT_TYPE_VARDICT) ||
        g_variant_n_children(arguments) > 1) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                            "window.match accepts only an optional string window selector");
        return FALSE;
    }
    GVariantIter iter;
    const char* key;
    GVariant* value;
    g_variant_iter_init(&iter, arguments);
    while (g_variant_iter_next(&iter, "{&sv}", &key, &value)) {
        g_autoptr(GVariant) field = value;
        if (!g_str_equal(key, "window") || !g_variant_is_of_type(field, G_VARIANT_TYPE_STRING)) {
            g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                                "window.match accepts only an optional string window selector");
            return FALSE;
        }
    }
    return TRUE;
}

static GVariant* legacy_window_list_arguments_for_lua(GVariant* arguments) {
    gboolean focused = TRUE;
    if (!g_variant_lookup(arguments, "focused", "b", &focused) || focused)
        return g_variant_ref(arguments);

    GVariantBuilder filtered;
    g_variant_builder_init(&filtered, G_VARIANT_TYPE_VARDICT);
    GVariantIter iter;
    const char* key;
    GVariant* value;
    g_variant_iter_init(&iter, arguments);
    while (g_variant_iter_next(&iter, "{&sv}", &key, &value)) {
        g_autoptr(GVariant) field = value;
        if (!g_str_equal(key, "focused"))
            g_variant_builder_add(&filtered, "{sv}", key, field);
    }
    return g_variant_ref_sink(g_variant_builder_end(&filtered));
}

static GVariant* legacy_window_list_from_lua(GVariant* windows, GError** error) {
    static const struct {
        const char* lua_name;
        const char* legacy_name;
    } fields[] = {{"id", "id"},
                  {"title", "title"},
                  {"app_id", "appId"},
                  {"gtk_app_id", "gtkAppId"},
                  {"wm_class", "wmClass"},
                  {"rule_app_id", "ruleAppId"},
                  {"focused", "focused"},
                  {"minimized", "minimized"},
                  {"last_user_time", "lastUserTime"},
                  {"parent", "parent"},
                  {"workspace_id", "workspaceId"},
                  {"workspace_number", "workspaceNumber"},
                  {"monitor_index", "monitorIndex"},
                  {"monitor_id", "monitorId"},
                  {"monitor", "monitor"},
                  {"above", "above"},
                  {"sticky", "sticky"},
                  {"demands_attention", "demandsAttention"},
                  {"unresponsive", "unresponsive"},
                  {"closable", "closable"},
                  {"minimizable", "minimizable"},
                  {"maximizable", "maximizable"},
                  {"movable", "movable"},
                  {"resizable", "resizable"},
                  {"role", "role"},
                  {"type", "type"},
                  {"maximized", "maximized"},
                  {"fullscreen", "fullscreen"},
                  {"frame", "geometry"}};
    if (!windows || !g_variant_is_of_type(windows, G_VARIANT_TYPE("av"))) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                            "Lua window snapshot is invalid");
        return NULL;
    }

    GVariantBuilder window_records;
    g_variant_builder_init(&window_records, G_VARIANT_TYPE("aa{sv}"));
    for (gsize i = 0; i < g_variant_n_children(windows); i++) {
        g_autoptr(GVariant) boxed = g_variant_get_child_value(windows, i);
        g_autoptr(GVariant) window = g_variant_get_variant(boxed);
        if (!g_variant_is_of_type(window, G_VARIANT_TYPE_VARDICT)) {
            g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                                "Lua window record is invalid");
            return NULL;
        }

        GVariantBuilder record;
        g_variant_builder_init(&record, G_VARIANT_TYPE_VARDICT);
        for (guint field_index = 0; field_index < G_N_ELEMENTS(fields); field_index++) {
            g_autoptr(GVariant) value =
                g_variant_lookup_value(window, fields[field_index].lua_name, NULL);
            if (value)
                g_variant_builder_add(&record, "{sv}", fields[field_index].legacy_name, value);
        }
        g_variant_builder_add_value(&window_records, g_variant_builder_end(&record));
    }

    GVariantBuilder result;
    g_variant_builder_init(&result, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&result, "{sv}", "windows", g_variant_builder_end(&window_records));
    return g_variant_ref_sink(g_variant_builder_end(&result));
}

static GVariant* legacy_window_match_from_lua(GVariant* windows, GVariant* arguments,
                                              GError** error) {
    if (!windows || !g_variant_is_of_type(windows, G_VARIANT_TYPE("av"))) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                            "Lua window snapshot is invalid");
        return NULL;
    }
    const char* selector = "active";
    g_variant_lookup(arguments, "window", "&s", &selector);
    GVariant* selected = NULL;
    for (gsize i = 0; i < g_variant_n_children(windows); i++) {
        g_autoptr(GVariant) boxed = g_variant_get_child_value(windows, i);
        g_autoptr(GVariant) window = g_variant_get_variant(boxed);
        if (!g_variant_is_of_type(window, G_VARIANT_TYPE_VARDICT)) {
            g_clear_pointer(&selected, g_variant_unref);
            g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                                "Lua window record is invalid");
            return NULL;
        }
        const char* id = NULL;
        gboolean focused = FALSE;
        g_variant_lookup(window, "id", "&s", &id);
        g_variant_lookup(window, "focused", "b", &focused);
        if ((g_str_equal(selector, "active") && focused) ||
            (!g_str_equal(selector, "active") && id && g_str_equal(selector, id))) {
            selected = g_variant_ref(window);
            break;
        }
    }
    if (!selected) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_NOENT, "window not found");
        return NULL;
    }

    const char* id = NULL;
    const char* title = "";
    const char* desktop_app_id = "";
    const char* gtk_app_id = "";
    const char* wm_class = "";
    const char* rule_app_id = "";
    gboolean focused = FALSE;
    g_variant_lookup(selected, "id", "&s", &id);
    g_variant_lookup(selected, "title", "&s", &title);
    g_variant_lookup(selected, "app_id", "&s", &desktop_app_id);
    g_variant_lookup(selected, "gtk_app_id", "&s", &gtk_app_id);
    g_variant_lookup(selected, "wm_class", "&s", &wm_class);
    g_variant_lookup(selected, "rule_app_id", "&s", &rule_app_id);
    g_variant_lookup(selected, "focused", "b", &focused);
    if (!id || !*id) {
        g_variant_unref(selected);
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                            "Lua window record has no stable ID");
        return NULL;
    }

    GVariantBuilder identity;
    g_variant_builder_init(&identity, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&identity, "{sv}", "desktop_app_id",
                          g_variant_new_string(desktop_app_id));
    g_variant_builder_add(&identity, "{sv}", "gtk_app_id", g_variant_new_string(gtk_app_id));
    g_variant_builder_add(&identity, "{sv}", "wm_class", g_variant_new_string(wm_class));
    g_variant_builder_add(&identity, "{sv}", "rule_app_id", g_variant_new_string(rule_app_id));

    GVariantBuilder match;
    g_variant_builder_init(&match, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&match, "{sv}", "type", g_variant_new_string("window"));
    g_variant_builder_add(&match, "{sv}", "title", g_variant_new_string(title));
    g_variant_builder_add(&match, "{sv}", "focused", g_variant_new_boolean(focused));
    if (*rule_app_id)
        g_variant_builder_add(&match, "{sv}", "app_id", g_variant_new_string(rule_app_id));

    GVariantBuilder result;
    g_variant_builder_init(&result, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&result, "{sv}", "id", g_variant_new_string(id));
    g_variant_builder_add(&result, "{sv}", "identity", g_variant_builder_end(&identity));
    g_variant_builder_add(&result, "{sv}", "match", g_variant_builder_end(&match));
    g_variant_unref(selected);
    return g_variant_ref_sink(g_variant_builder_end(&result));
}

static GVariant* legacy_layer_list_from_lua(GVariant* layers, GError** error) {
    if (!layers || !g_variant_is_of_type(layers, G_VARIANT_TYPE("av"))) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                            "Lua layer snapshot is invalid");
        return NULL;
    }

    GVariantBuilder surfaces;
    g_variant_builder_init(&surfaces, G_VARIANT_TYPE("aa{sv}"));
    for (gsize i = 0; i < g_variant_n_children(layers); i++) {
        g_autoptr(GVariant) boxed = g_variant_get_child_value(layers, i);
        g_autoptr(GVariant) layer = g_variant_get_variant(boxed);
        if (!g_variant_is_of_type(layer, G_VARIANT_TYPE_VARDICT)) {
            g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                                "Lua layer record is invalid");
            return NULL;
        }

        g_autoptr(GVariant) id = g_variant_lookup_value(layer, "id", G_VARIANT_TYPE_STRING);
        g_autoptr(GVariant) namespace =
            g_variant_lookup_value(layer, "namespace", G_VARIANT_TYPE_STRING);
        if (!namespace)
            continue;
        if (!id) {
            g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                                "Lua layer record has no string ID");
            return NULL;
        }

        GVariantBuilder surface;
        g_variant_builder_init(&surface, G_VARIANT_TYPE_VARDICT);
        g_variant_builder_add(&surface, "{sv}", "id", id);
        g_variant_builder_add(&surface, "{sv}", "namespace", namespace);
        g_autoptr(GVariant) title = g_variant_lookup_value(layer, "title", G_VARIANT_TYPE_STRING);
        g_variant_builder_add(&surface, "{sv}", "title", title ? title : g_variant_new_string(""));
        g_variant_builder_add_value(&surfaces, g_variant_builder_end(&surface));
    }

    GVariantBuilder result;
    g_variant_builder_init(&result, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&result, "{sv}", "surfaces", g_variant_builder_end(&surfaces));
    return g_variant_ref_sink(g_variant_builder_end(&result));
}

GVariant* gnoblin_config_read_api(const char* method, GVariant* arguments, GError** error) {
    static const char* read_methods[] = {
        "version",
        "windows.list",
        "window.list",
        "capabilities.list",
        "focus.history",
        "settings",
        "focus.policy",
        "config.path",
        "config.default",
        "session.activity",
        "session.status",
        "runtime.status",
        "launch.status",
        "window.match",
        "workspace.list",
        "layer.list",
        "monitor.list",
        "output.list",
        "layer.animation_policy",
        "workspaces.list",
        "monitors.list",
        "outputs.list",
        "outputs.renderer",
        "monitors.privacy_screen",
        "layers.list",
        "launches.list",
        "launches.snapshot",
        "shortcuts.list",
        "shortcuts.actions",
        "privacy.state",
        "input.devices",
        "input.sources",
        "input.current_source",
        "input.orientation_lock",
        "appearance.color_scheme",
        NULL,
    };
    gboolean known = FALSE;
    for (guint i = 0; method && read_methods[i]; i++)
        known |= g_str_equal(read_methods[i], method);
    if (!known || !arguments || !g_variant_is_of_type(arguments, G_VARIANT_TYPE_VARDICT)) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                            "Gnoblin API read requires a supported method and argument table");
        return NULL;
    }
    if (pending_runtime || !active_runtime || !active_runtime->config.settings_document) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_FAILED,
                            "Gnoblin Lua API is unavailable before configuration is active");
        return NULL;
    }

    gboolean accepts_arguments =
        g_str_equal(method, "focus.history") || g_str_equal(method, "windows.list") ||
        g_str_equal(method, "window.list") || g_str_equal(method, "layer.animation_policy") ||
        g_str_equal(method, "window.match") || g_str_equal(method, "layers.list") ||
        g_str_equal(method, "shortcuts.actions");
    if (!accepts_arguments && g_variant_n_children(arguments) != 0) {
        g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                    "Lua API read '%s' does not accept arguments", method);
        return NULL;
    }
    if (g_str_equal(method, "layer.animation_policy")) {
        const char* namespace = NULL;
        if (g_variant_n_children(arguments) != 1 ||
            !g_variant_lookup(arguments, "namespace", "&s", &namespace) || !namespace) {
            g_set_error_literal(
                error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                "Lua API read 'layer.animation_policy' requires a namespace string");
            return NULL;
        }
        gsize namespace_length = strlen(namespace);
        if (namespace_length == 0 || namespace_length > 128 ||
            !g_utf8_validate(namespace, namespace_length, NULL)) {
            g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                                "layer namespace must be 1 to 128 bytes of UTF-8");
            return NULL;
        }
    }
    const char* shortcut_action_group = NULL;
    if (g_str_equal(method, "shortcuts.actions") &&
        (g_variant_n_children(arguments) > 1 ||
         (g_variant_n_children(arguments) == 1 &&
          !g_variant_lookup(arguments, "group", "&s", &shortcut_action_group)))) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                            "shortcuts.actions accepts an optional group string");
        return NULL;
    }
    if (g_str_equal(method, "window.list") && !legacy_window_list_arguments_valid(arguments, error))
        return NULL;
    if (g_str_equal(method, "window.match") &&
        !legacy_window_match_arguments_valid(arguments, error))
        return NULL;

    LuaConfig* config = &active_runtime->config;
    if (config->dispatching || config->api_calling) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_FAILED,
                            "Gnoblin Lua API reads cannot be dispatched reentrantly");
        return NULL;
    }

    lua_State* state = active_runtime->state;
    int base = lua_gettop(state);
    guint action_start = config->runtime_actions->len;
    config->api_calling = TRUE;

    lua_getglobal(state, "gnoblin");
    if (g_str_equal(method, "version")) {
        lua_getfield(state, -1, "version");
        lua_remove(state, -2);
    } else if (g_str_equal(method, "capabilities.list")) {
        lua_getfield(state, -1, "capabilities");
        lua_getfield(state, -1, "list");
        lua_remove(state, -2);
        lua_remove(state, -2);
    } else if (g_str_equal(method, "focus.history")) {
        lua_getfield(state, -1, "focus");
        lua_getfield(state, -1, "history");
        lua_remove(state, -2);
        lua_remove(state, -2);
    } else if (g_str_equal(method, "focus.policy")) {
        lua_getfield(state, -1, "focus");
        lua_getfield(state, -1, "policy");
        lua_remove(state, -2);
        lua_remove(state, -2);
    } else if (g_str_equal(method, "config.path") || g_str_equal(method, "config.default")) {
        lua_getfield(state, -1, "config");
        lua_getfield(state, -1, g_str_equal(method, "config.path") ? "path" : "default");
        lua_remove(state, -2);
        lua_remove(state, -2);
    } else if (g_str_equal(method, "windows.list") || g_str_equal(method, "window.list") ||
               g_str_equal(method, "window.match")) {
        lua_getfield(state, -1, "windows");
        lua_getfield(state, -1, "list");
        lua_remove(state, -2);
        lua_remove(state, -2);
    } else if (g_str_equal(method, "settings")) {
        lua_getfield(state, -1, "settings");
        lua_remove(state, -2);
    } else if (g_str_equal(method, "session.activity") || g_str_equal(method, "session.status")) {
        lua_getfield(state, -1, "session");
        lua_getfield(state, -1, g_str_equal(method, "session.status") ? "status" : "activity");
        lua_remove(state, -2);
        lua_remove(state, -2);
    } else if (g_str_equal(method, "runtime.status")) {
        lua_getfield(state, -1, "runtime");
        lua_getfield(state, -1, "status");
        lua_remove(state, -2);
        lua_remove(state, -2);
    } else if (g_str_equal(method, "layer.animation_policy")) {
        lua_getfield(state, -1, "layers");
        lua_getfield(state, -1, "animation_policy");
        lua_remove(state, -2);
        lua_remove(state, -2);
    } else if (g_str_equal(method, "workspace.list") || g_str_equal(method, "workspaces.list")) {
        lua_getfield(state, -1, "workspaces");
        lua_getfield(state, -1, "list");
        lua_remove(state, -2);
        lua_remove(state, -2);
    } else if (g_str_equal(method, "monitor.list") || g_str_equal(method, "monitors.list")) {
        lua_getfield(state, -1, "monitors");
        lua_getfield(state, -1, "list");
        lua_remove(state, -2);
        lua_remove(state, -2);
    } else if (g_str_equal(method, "outputs.list") || g_str_equal(method, "output.list")) {
        lua_getfield(state, -1, "outputs");
        lua_getfield(state, -1, "list");
        lua_remove(state, -2);
        lua_remove(state, -2);
    } else if (g_str_equal(method, "outputs.renderer")) {
        lua_getfield(state, -1, "outputs");
        lua_getfield(state, -1, "renderer");
        lua_remove(state, -2);
        lua_remove(state, -2);
    } else if (g_str_equal(method, "monitors.privacy_screen")) {
        lua_getfield(state, -1, "monitors");
        lua_getfield(state, -1, "privacy_screen");
        lua_remove(state, -2);
        lua_remove(state, -2);
    } else if (g_str_equal(method, "layers.list") || g_str_equal(method, "layer.list")) {
        lua_getfield(state, -1, "layers");
        lua_getfield(state, -1, "list");
        lua_remove(state, -2);
        lua_remove(state, -2);
    } else if (g_str_equal(method, "launches.list")) {
        lua_getfield(state, -1, "launches");
        lua_getfield(state, -1, "list");
        lua_remove(state, -2);
        lua_remove(state, -2);
    } else if (g_str_equal(method, "launches.snapshot") || g_str_equal(method, "launch.status")) {
        lua_getfield(state, -1, "launches");
        lua_getfield(state, -1, "snapshot");
        lua_remove(state, -2);
        lua_remove(state, -2);
    } else if (g_str_equal(method, "shortcuts.list")) {
        lua_getfield(state, -1, "shortcuts");
        lua_getfield(state, -1, "list");
        lua_remove(state, -2);
        lua_remove(state, -2);
    } else if (g_str_equal(method, "shortcuts.actions")) {
        lua_getfield(state, -1, "shortcuts");
        lua_getfield(state, -1, "actions");
        lua_remove(state, -2);
        lua_remove(state, -2);
    } else if (g_str_equal(method, "privacy.state")) {
        lua_getfield(state, -1, "privacy");
        lua_getfield(state, -1, "state");
        lua_remove(state, -2);
        lua_remove(state, -2);
    } else if (g_str_equal(method, "appearance.color_scheme")) {
        lua_getfield(state, -1, "appearance");
        lua_getfield(state, -1, "color_scheme");
        lua_remove(state, -2);
        lua_remove(state, -2);
    } else if (g_str_equal(method, "input.devices") || g_str_equal(method, "input.sources") ||
               g_str_equal(method, "input.current_source") ||
               g_str_equal(method, "input.orientation_lock")) {
        lua_getfield(state, -1, "input");
        const char* field = g_str_equal(method, "input.devices")          ? "devices"
                            : g_str_equal(method, "input.sources")        ? "sources"
                            : g_str_equal(method, "input.current_source") ? "current_source"
                                                                          : "orientation_lock";
        lua_getfield(state, -1, field);
        lua_remove(state, -2);
        lua_remove(state, -2);
    } else { /* focus.policy */
        lua_getfield(state, -1, "focus");
        lua_getfield(state, -1, "policy");
        lua_remove(state, -2);
        lua_remove(state, -2);
    }

    gboolean is_function = !g_str_equal(method, "settings");
    int argument_count = 0;
    if (!lua_isfunction(state, -1) && is_function) {
        g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL, "Lua API read '%s' is not registered",
                    method);
        lua_settop(state, base);
        config->api_calling = FALSE;
        return NULL;
    }
    if (is_function) {
        if (g_str_equal(method, "layer.animation_policy")) {
            const char* namespace = NULL;
            g_variant_lookup(arguments, "namespace", "&s", &namespace);
            lua_pushstring(state, namespace);
            argument_count = 1;
        } else if (g_str_equal(method, "shortcuts.actions") && shortcut_action_group) {
            lua_pushstring(state, shortcut_action_group);
            argument_count = 1;
        } else if (g_variant_n_children(arguments) > 0 && !g_str_equal(method, "window.match")) {
            g_autoptr(GVariant) legacy_window_arguments =
                g_str_equal(method, "window.list") ? legacy_window_list_arguments_for_lua(arguments)
                                                   : NULL;
            push_variant(state, legacy_window_arguments ? legacy_window_arguments : arguments);
            argument_count = 1;
        }
        if (lua_pcall(state, argument_count, 1, 0) != LUA_OK) {
            g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL, "Lua API read '%s' failed: %s",
                        method, lua_error_text(state));
            lua_settop(state, base);
            config->api_calling = FALSE;
            g_ptr_array_set_size(config->runtime_actions, action_start);
            return NULL;
        }
    }

    GVariant* result = NULL;
    if (g_str_equal(method, "appearance.color_scheme")) {
        GVariantBuilder response;
        g_variant_builder_init(&response, G_VARIANT_TYPE_VARDICT);
        if (lua_isnil(state, -1)) {
            g_variant_builder_add(&response, "{sv}", "available", g_variant_new_boolean(FALSE));
            result = g_variant_ref_sink(g_variant_builder_end(&response));
        } else if (lua_type(state, -1) != LUA_TSTRING) {
            g_variant_builder_clear(&response);
            g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                                "Lua appearance color scheme must be a string or nil");
        } else {
            const char* color_scheme = lua_tostring(state, -1);
            if (!g_str_equal(color_scheme, "default") &&
                !g_str_equal(color_scheme, "prefer-dark") &&
                !g_str_equal(color_scheme, "prefer-light")) {
                g_variant_builder_clear(&response);
                g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                                    "Lua appearance color scheme is unsupported");
            } else {
                g_variant_builder_add(&response, "{sv}", "available", g_variant_new_boolean(TRUE));
                g_variant_builder_add(&response, "{sv}", "color_scheme",
                                      g_variant_new_string(color_scheme));
                result = g_variant_ref_sink(g_variant_builder_end(&response));
            }
        }
    } else if (g_str_equal(method, "input.current_source") && lua_isnil(state, -1)) {
        GVariantBuilder response;
        g_variant_builder_init(&response, G_VARIANT_TYPE_VARDICT);
        g_variant_builder_add(&response, "{sv}", "available", g_variant_new_boolean(FALSE));
        g_variant_builder_add(&response, "{sv}", "revision",
                              g_variant_new_int64((gint64)config->input_source_revision));
        result = g_variant_ref_sink(g_variant_builder_end(&response));
    } else {
        gboolean force_array =
            g_str_equal(method, "input.devices") || g_str_equal(method, "input.sources");
        g_autoptr(GVariant) value = variant_from_lua(state, -1, 0, force_array, 0, error);
        if (value && g_str_equal(method, "workspace.list")) {
            result = legacy_workspace_list_from_lua(value, error);
        } else if (value && g_str_equal(method, "window.list")) {
            result = legacy_window_list_from_lua(value, error);
        } else if (value && g_str_equal(method, "window.match")) {
            result = legacy_window_match_from_lua(value, arguments, error);
        } else if (value && g_str_equal(method, "layer.list")) {
            result = legacy_layer_list_from_lua(value, error);
        } else if (value && g_str_equal(method, "monitor.list")) {
            if (!g_variant_is_of_type(value, G_VARIANT_TYPE("av"))) {
                g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                                    "Lua monitor snapshot is invalid");
            } else {
                GVariantBuilder monitors;
                gboolean valid_records = TRUE;
                g_variant_builder_init(&monitors, G_VARIANT_TYPE("aa{sv}"));
                for (gsize i = 0; i < g_variant_n_children(value); i++) {
                    g_autoptr(GVariant) boxed = g_variant_get_child_value(value, i);
                    g_autoptr(GVariant) monitor = g_variant_get_variant(boxed);
                    if (!g_variant_is_of_type(monitor, G_VARIANT_TYPE_VARDICT)) {
                        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                                            "Lua monitor record is invalid");
                        valid_records = FALSE;
                        break;
                    }
                    GVariantBuilder record;
                    g_variant_builder_init(&record, G_VARIANT_TYPE_VARDICT);
                    static const struct {
                        const char* source;
                        const GVariantType* type;
                    } fields[] = {
                        {"x", G_VARIANT_TYPE_INT32},         {"y", G_VARIANT_TYPE_INT32},
                        {"width", G_VARIANT_TYPE_INT32},     {"height", G_VARIANT_TYPE_INT32},
                        {"primary", G_VARIANT_TYPE_BOOLEAN}, {"scale", G_VARIANT_TYPE_DOUBLE},
                    };
                    g_autoptr(GVariant) index =
                        g_variant_lookup_value(monitor, "index", G_VARIANT_TYPE_INT32);
                    if (!index) {
                        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                                            "Lua monitor record has no integer index");
                        g_variant_builder_clear(&record);
                        valid_records = FALSE;
                        break;
                    }
                    g_variant_builder_add(&record, "{sv}", "id", index);
                    gboolean valid = TRUE;
                    for (guint field = 0; field < G_N_ELEMENTS(fields); field++) {
                        g_autoptr(GVariant) field_value = g_variant_lookup_value(
                            monitor, fields[field].source, fields[field].type);
                        if (!field_value) {
                            valid = FALSE;
                            break;
                        }
                        g_variant_builder_add(&record, "{sv}", fields[field].source, field_value);
                    }
                    if (!valid) {
                        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                                            "Lua monitor record is missing a legacy field");
                        g_variant_builder_clear(&record);
                        valid_records = FALSE;
                        break;
                    }
                    g_variant_builder_add_value(&monitors, g_variant_builder_end(&record));
                }
                if (valid_records) {
                    GVariantBuilder response;
                    g_variant_builder_init(&response, G_VARIANT_TYPE_VARDICT);
                    g_variant_builder_add(&response, "{sv}", "monitors",
                                          g_variant_builder_end(&monitors));
                    result = g_variant_ref_sink(g_variant_builder_end(&response));
                } else {
                    g_variant_builder_clear(&monitors);
                }
            }
        } else if (value && (force_array || g_str_equal(method, "input.current_source"))) {
            GVariantBuilder response;
            g_variant_builder_init(&response, G_VARIANT_TYPE_VARDICT);
            if (g_str_equal(method, "input.devices") || g_str_equal(method, "input.sources")) {
                g_variant_builder_add(&response, "{sv}",
                                      g_str_equal(method, "input.devices") ? "devices" : "sources",
                                      g_variant_ref(value));
                guint64 revision = g_str_equal(method, "input.devices")
                                       ? config->input_device_revision
                                       : config->input_source_revision;
                g_variant_builder_add(&response, "{sv}", "revision",
                                      g_variant_new_int64((gint64)revision));
            } else {
                g_variant_builder_add(&response, "{sv}", "available", g_variant_new_boolean(TRUE));
                g_variant_builder_add(&response, "{sv}", "source", g_variant_ref(value));
                g_variant_builder_add(&response, "{sv}", "revision",
                                      g_variant_new_int64((gint64)config->input_source_revision));
            }
            result = g_variant_ref_sink(g_variant_builder_end(&response));
        } else if (value) {
            result = g_variant_ref(value);
        }
    }
    lua_settop(state, base);
    config->api_calling = FALSE;
    if (config->runtime_actions->len != action_start) {
        g_ptr_array_set_size(config->runtime_actions, action_start);
        g_clear_pointer(&result, g_variant_unref);
        if (error && !*error)
            g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_FAILED,
                                "Gnoblin API read unexpectedly queued a mutation");
        return NULL;
    }
    return result ? g_variant_ref_sink(result) : NULL;
}
