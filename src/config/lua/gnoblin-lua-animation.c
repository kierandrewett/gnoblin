#include "gnoblin-lua-internal.h"

static gboolean animation_name_valid(const char* name) {
    gsize length = name ? strlen(name) : 0;
    if (length == 0 || length > 80)
        return FALSE;
    for (gsize i = 0; i < length; i++) {
        unsigned char character = (unsigned char)name[i];
        if (!g_ascii_isalnum(character) && character != '_' && character != '-')
            return FALSE;
    }
    return TRUE;
}

static GVariant* animation_records(LuaConfig* config) {
    return config && config->animation_snapshot
               ? g_variant_lookup_value(config->animation_snapshot, "animations",
                                        G_VARIANT_TYPE("aa{sv}"))
               : NULL;
}

static GVariant* policy_array_record(GVariant* array, gsize index) {
    if (!array || index >= g_variant_n_children(array))
        return NULL;
    GVariant* child = g_variant_get_child_value(array, index);
    if (g_variant_is_of_type(child, G_VARIANT_TYPE_VARIANT)) {
        GVariant* value = g_variant_get_variant(child);
        g_variant_unref(child);
        return value;
    }
    return child;
}

static gboolean policy_rule_matches(GVariant* match, const char* type, const char* app_id,
                                    const char* title, const char* layer, gboolean focused,
                                    GError** error) {
    GVariantIter iter;
    const char* key;
    GVariant* value;
    g_variant_iter_init(&iter, match);
    while (g_variant_iter_next(&iter, "{&sv}", &key, &value)) {
        gboolean matched = FALSE;
        if (g_str_equal(key, "type")) {
            matched = g_variant_is_of_type(value, G_VARIANT_TYPE_STRING) &&
                      g_str_equal(g_variant_get_string(value, NULL), type);
        } else if (g_str_equal(key, "focused")) {
            matched = g_variant_is_of_type(value, G_VARIANT_TYPE_BOOLEAN) &&
                      g_variant_get_boolean(value) == focused;
        } else if (g_str_equal(key, "app-id") || g_str_equal(key, "app_id") ||
                   g_str_equal(key, "title") || g_str_equal(key, "layer")) {
            const char* pattern = g_variant_is_of_type(value, G_VARIANT_TYPE_STRING)
                                      ? g_variant_get_string(value, NULL)
                                      : NULL;
            const char* candidate = g_str_equal(key, "app-id") || g_str_equal(key, "app_id")
                                        ? app_id
                                    : g_str_equal(key, "title") ? title
                                                                : layer;
            matched = pattern && candidate &&
                      gnoblin_config_window_pattern_match(pattern, candidate, error);
            if (error && *error) {
                g_variant_unref(value);
                return FALSE;
            }
        }
        /* Workspace and other match fields have no value in a namespace-only
         * query, matching the old shell bridge's synthetic layer record. */
        g_variant_unref(value);
        if (!matched)
            return FALSE;
    }
    return TRUE;
}

static gboolean builtin_animation_supports(const char* name, const char* event) {
    if (g_str_equal(name, "none"))
        return TRUE;
    if (g_str_equal(name, "fade") || g_str_equal(name, "gnome"))
        return g_str_equal(event, "minimize") || g_str_equal(event, "restore") ||
               g_str_equal(event, "open") || g_str_equal(event, "close") ||
               g_str_equal(event, "dialog-open") || g_str_equal(event, "dialog-close") ||
               g_str_equal(event, "layer-open") || g_str_equal(event, "layer-close");
    if (g_str_equal(name, "slide"))
        return g_str_equal(event, "layer-open") || g_str_equal(event, "layer-close");
    if (g_str_equal(name, "gnoblin-layer-open"))
        return g_str_equal(event, "layer-open");
    if (g_str_equal(name, "gnoblin-layer-close"))
        return g_str_equal(event, "layer-close");
    if (g_str_equal(name, "gnome-minimize"))
        return g_str_equal(event, "minimize");
    if (g_str_equal(name, "gnome-restore"))
        return g_str_equal(event, "restore");
    if (g_str_equal(name, "gnome-open"))
        return g_str_equal(event, "open");
    if (g_str_equal(name, "gnome-close"))
        return g_str_equal(event, "close");
    if (g_str_equal(name, "gnome-dialog-open"))
        return g_str_equal(event, "dialog-open");
    if (g_str_equal(name, "gnome-dialog-close"))
        return g_str_equal(event, "dialog-close");
    if (g_str_equal(name, "gnome-workspace-switch"))
        return g_str_equal(event, "workspace-switch");
    if (g_str_equal(name, "gnome-resize"))
        return g_str_equal(event, "resize");
    if (g_str_equal(name, "gnoblin-shadow-change"))
        return g_str_equal(event, "shadow-change");
    return FALSE;
}

static gboolean policy_animation_supports(GVariant* document, const char* name, const char* event) {
    g_autoptr(GVariant) animations = g_variant_lookup_value(document, "animations", NULL);
    for (gsize i = 0; animations && i < g_variant_n_children(animations); i++) {
        g_autoptr(GVariant) record = policy_array_record(animations, i);
        const char* record_name = NULL;
        const char* record_event = NULL;
        gboolean enabled = TRUE;
        if (!record || !g_variant_is_of_type(record, G_VARIANT_TYPE_VARDICT) ||
            !g_variant_lookup(record, "name", "&s", &record_name) ||
            !g_str_equal(record_name, name))
            continue;
        g_variant_lookup(record, "event", "&s", &record_event);
        g_variant_lookup(record, "enable", "b", &enabled);
        return enabled && g_strcmp0(record_event, event) == 0;
    }
    return builtin_animation_supports(name, event);
}

static GVariant* policy_layer_phase(GVariant* document, const char* namespace, gboolean opening,
                                    GError** error) {
    const char* event = opening ? "layer-open" : "layer-close";
    const char* alias = opening ? "in" : "out";
    const char* selected = "none";
    g_autoptr(GVariant) animations = g_variant_lookup_value(document, "animations", NULL);
    for (gsize i = 0; animations && i < g_variant_n_children(animations); i++) {
        g_autoptr(GVariant) record = policy_array_record(animations, i);
        const char* name = NULL;
        const char* registered_event = NULL;
        gboolean enabled = TRUE;
        if (!record || !g_variant_is_of_type(record, G_VARIANT_TYPE_VARDICT))
            continue;
        g_variant_lookup(record, "enable", "b", &enabled);
        if (enabled && g_variant_lookup(record, "name", "&s", &name) &&
            g_variant_lookup(record, "event", "&s", &registered_event) &&
            g_str_equal(registered_event, event)) {
            selected = name;
            break;
        }
    }
    g_autoptr(GVariant) selected_duration = NULL;
    g_autoptr(GVariant) selected_easing = NULL;
    g_autoptr(GVariant) rules = g_variant_lookup_value(document, "window-rules", NULL);
    for (gsize i = 0; rules && i < g_variant_n_children(rules); i++) {
        g_autoptr(GVariant) rule = policy_array_record(rules, i);
        if (!rule || !g_variant_is_of_type(rule, G_VARIANT_TYPE_VARDICT))
            continue;
        g_autoptr(GVariant) match = g_variant_lookup_value(rule, "match", NULL);
        if (!match || !g_variant_is_of_type(match, G_VARIANT_TYPE_VARDICT) ||
            !policy_rule_matches(match, "layer", "", "", namespace, FALSE, error)) {
            if (error && *error)
                return NULL;
            continue;
        }
        g_autoptr(GVariant) animation = g_variant_lookup_value(rule, "animation", NULL);
        if (!animation)
            continue;
        const char* name = NULL;
        gboolean has_override = FALSE;
        if (g_variant_is_of_type(animation, G_VARIANT_TYPE_STRING)) {
            name = g_variant_get_string(animation, NULL);
        } else if (g_variant_is_of_type(animation, G_VARIANT_TYPE_VARDICT)) {
            g_autoptr(GVariant) phase_name = g_variant_lookup_value(animation, event, NULL);
            if (!phase_name) {
                phase_name =
                    g_variant_lookup_value(animation, opening ? "layer_open" : "layer_close", NULL);
            }
            if (!phase_name)
                phase_name = g_variant_lookup_value(animation, alias, NULL);
            if (!phase_name || !g_variant_is_of_type(phase_name, G_VARIANT_TYPE_STRING))
                continue;
            name = g_variant_get_string(phase_name, NULL);
            has_override = TRUE;
        } else {
            continue;
        }
        if (!policy_animation_supports(document, name, event))
            continue;
        selected = name;
        g_clear_pointer(&selected_duration, g_variant_unref);
        g_clear_pointer(&selected_easing, g_variant_unref);
        if (has_override) {
            selected_duration = g_variant_lookup_value(animation, "duration", NULL);
            selected_easing = g_variant_lookup_value(animation, "ease", NULL);
            if (!selected_easing)
                selected_easing = g_variant_lookup_value(animation, "easing", NULL);
        }
    }

    GVariantBuilder phase;
    g_variant_builder_init(&phase, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&phase, "{sv}", "animation", g_variant_new_string(selected));
    if (selected_duration)
        g_variant_builder_add(&phase, "{sv}", "duration", selected_duration);
    if (selected_easing)
        g_variant_builder_add(&phase, "{sv}", "easing", selected_easing);
    return g_variant_ref_sink(g_variant_builder_end(&phase));
}

static GVariant* layer_animation_policy(LuaConfig* config, const char* namespace, GError** error) {
    if (!config || !config->settings_document) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_FAILED,
                            "Gnoblin Lua configuration is unavailable");
        return NULL;
    }
    g_autoptr(GVariant) enter =
        policy_layer_phase(config->settings_document, namespace, TRUE, error);
    if (!enter)
        return NULL;
    g_autoptr(GVariant) exit =
        policy_layer_phase(config->settings_document, namespace, FALSE, error);
    if (!exit)
        return NULL;

    /* The former policy adapter resolved window shadow against the default
     * window identity, distinct from the queried layer namespace. */
    g_autoptr(GVariant) window_shadow = g_variant_ref_sink(g_variant_new_boolean(FALSE));
    g_autoptr(GVariant) window_rules =
        g_variant_lookup_value(config->settings_document, "window-rules", NULL);
    for (gsize i = 0; window_rules && i < g_variant_n_children(window_rules); i++) {
        g_autoptr(GVariant) rule = policy_array_record(window_rules, i);
        if (!rule || !g_variant_is_of_type(rule, G_VARIANT_TYPE_VARDICT))
            continue;
        g_autoptr(GVariant) match = g_variant_lookup_value(rule, "match", NULL);
        if (!match || !g_variant_is_of_type(match, G_VARIANT_TYPE_VARDICT) ||
            !policy_rule_matches(match, "window", "", "", NULL, TRUE, error)) {
            if (error && *error)
                return NULL;
            continue;
        }
        g_autoptr(GVariant) corners = g_variant_lookup_value(rule, "corners", NULL);
        g_autoptr(GVariant) shadow =
            corners ? g_variant_lookup_value(corners, "shadow", NULL) : NULL;
        if (shadow) {
            g_clear_pointer(&window_shadow, g_variant_unref);
            window_shadow = g_variant_ref(shadow);
        }
    }

    GVariantBuilder policy;
    g_variant_builder_init(&policy, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&policy, "{sv}", "namespace", g_variant_new_string(namespace));
    g_variant_builder_add(&policy, "{sv}", "enter", enter);
    g_variant_builder_add(&policy, "{sv}", "exit", exit);
    g_variant_builder_add(&policy, "{sv}", "window_shadow", window_shadow);
    g_variant_builder_add(&policy, "{sv}", "revision",
                          g_variant_new_uint64(config->settings_revision));
    return g_variant_ref_sink(g_variant_builder_end(&policy));
}

int lua_layers_animation_policy(lua_State* state) {
    LuaConfig* config = lua_touserdata(state, lua_upvalueindex(1));
    if (lua_gettop(state) != 1 || lua_type(state, 1) != LUA_TSTRING)
        return luaL_error(state, "gnoblin.layers.animation_policy requires one namespace string");
    const char* namespace = lua_tostring(state, 1);
    gsize length = strlen(namespace);
    if (length == 0 || length > 128 || !g_utf8_validate(namespace, length, NULL))
        return luaL_error(state, "layer namespace must be 1 to 128 bytes of UTF-8");
    g_autoptr(GError) error = NULL;
    g_autoptr(GVariant) result = layer_animation_policy(config, namespace, &error);
    if (!result)
        return luaL_error(state, "%s", error ? error->message : "could not resolve layer policy");
    push_variant(state, result);
    push_readonly_copy(state, -1);
    lua_remove(state, -2);
    return 1;
}

int lua_animations_list(lua_State* state) {
    LuaConfig* config = lua_touserdata(state, lua_upvalueindex(1));
    if (lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.animations.list takes no arguments");
    g_autoptr(GVariant) records = animation_records(config);
    if (!records)
        return luaL_error(state, "native animation snapshot is unavailable");
    push_variant(state, records);
    push_readonly_copy(state, -1);
    lua_remove(state, -2);
    return 1;
}

/* Read the current native animation registry without queuing a runtime action. */
int lua_animation_get(lua_State* state) {
    LuaConfig* config = lua_touserdata(state, lua_upvalueindex(1));
    if (lua_gettop(state) != 1 || lua_type(state, 1) != LUA_TSTRING)
        return luaL_error(state, "gnoblin.animations.get requires one animation name");
    const char* name = lua_tostring(state, 1);
    if (!animation_name_valid(name))
        return luaL_error(
            state, "animation name must be 1 to 80 ASCII letters, digits, underscores, or hyphens");
    g_autoptr(GVariant) records = animation_records(config);
    if (!records)
        return luaL_error(state, "native animation snapshot is unavailable");
    for (gsize i = 0; i < g_variant_n_children(records); i++) {
        g_autoptr(GVariant) record = g_variant_get_child_value(records, i);
        const char* record_name = NULL;
        if (!g_variant_lookup(record, "name", "&s", &record_name) ||
            !g_str_equal(record_name, name))
            continue;
        push_variant(state, record);
        push_readonly_copy(state, -1);
        lua_remove(state, -2);
        return 1;
    }
    lua_pushnil(state);
    return 1;
}
