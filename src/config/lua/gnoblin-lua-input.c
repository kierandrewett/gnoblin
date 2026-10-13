#include "gnoblin-lua-internal.h"

static gboolean is_keybinding_action_name(const char* key) {
    if (!key || !*key)
        return FALSE;
    gboolean previous_underscore = TRUE;
    for (const unsigned char* p = (const unsigned char*)key; *p; p++) {
        if (*p == '_') {
            if (previous_underscore)
                return FALSE;
            previous_underscore = TRUE;
        } else if (g_ascii_islower(*p) || g_ascii_isdigit(*p))
            previous_underscore = FALSE;
        else
            return FALSE;
    }
    return !previous_underscore;
}

/* Deprecated enum spellings and their current names. A key can appear in several sections, so only
 * values that no other setting uses are listed. Old spellings stay valid for one release. */
static const char* deprecated_enum_replacement(const char* key, const char* value) {
    static const struct {
        const char* key;
        const char* old_value;
        const char* new_value;
    } aliases[] = {
        {"when", "normal", "unlocked"},
        {"when", "unlock-screen", "locked"},
        {"level", "default", "inherit"},
        {"focus-mode", "sloppy", "hover"},
        {"focus-mode", "mouse", "hover-strict"},
        {"focus-new-windows", "smart", "allow"},
        {"focus-new-windows", "strict", "prevent"},
        {"left-handed", "mouse", "follow-mouse"},
        {"tap-button-map", "lrm", "left-right-middle"},
        {"tap-button-map", "lmr", "left-middle-right"},
    };
    for (guint i = 0; i < G_N_ELEMENTS(aliases); i++) {
        if (!strcmp(key, aliases[i].key) && !strcmp(value, aliases[i].old_value))
            return aliases[i].new_value;
    }
    return NULL;
}

/* Public declarations use Lua identifiers. Keep keybinding action names in
 * snake_case until native control maps them to their GSettings keys. */
void push_settings(lua_State* state, int source, const char* parent, int depth,
                   int keybinding_depth) {
    if (depth > MAX_CONFIG_DEPTH)
        luaL_error(state, "Lua config nesting exceeds 64 levels");
    luaL_checkstack(state, 6, "settings nesting");
    source = lua_absindex(state, source);
    if (!lua_istable(state, source)) {
        lua_pushvalue(state, source);
        return;
    }

    /* Only declarations below keybindings.keyboard/keybindings.pointer are
     * callbacks. input.keyboard is device configuration, and a keybinding
     * group itself is a map of declarations, not a single binding.
     * Pointer bindings are declared as a Lua callback plus one binding, but
     * the compositor only needs a named array of accelerators.  Keep the
     * callback in the Lua runtime (registered by lua_configure()) and never
     * serialize it into the native settings document. */
    if (keybinding_depth == 3 && parent &&
        (g_str_equal(parent, "pointer") || g_str_equal(parent, "keyboard"))) {
        const char* group = parent;
        lua_getfield(state, source, "enable");
        if (!lua_isnil(state, -1) && !lua_isboolean(state, -1))
            luaL_error(state, "%s binding enable must be a boolean", group);
        gboolean disabled = lua_isboolean(state, -1) && !lua_toboolean(state, -1);
        lua_pop(state, 1);
        if (disabled) {
            lua_newtable(state);
            mark_array_table(state, -1);
            return;
        }
        lua_getfield(state, source, "binding");
        if (lua_type(state, -1) != LUA_TSTRING || !lua_rawlen(state, -1))
            luaL_error(state, "%s binding requires a nonempty binding string", group);
        lua_newtable(state);
        mark_array_table(state, -1);
        lua_pushvalue(state, -2);
        lua_rawseti(state, -2, 1);
        lua_remove(state, -2);
        return;
    }
    lua_newtable(state);
    int destination = lua_gettop(state);
    gboolean literal =
        parent && (!strcmp(parent, "shader-uniforms") || !strcmp(parent, "frame-renderers") ||
                   !strcmp(parent, "shortcuts") || !strcmp(parent, "autostart") ||
                   !strcmp(parent, "interfaces"));
    lua_pushnil(state);
    while (lua_next(state, source)) {
        gboolean keybinding_action = keybinding_depth == 2 && lua_type(state, -2) == LUA_TSTRING;
        if (keybinding_action && !is_keybinding_action_name(lua_tostring(state, -2)))
            luaL_error(state, "keybinding action names must use snake_case");
        if (lua_type(state, -2) == LUA_TSTRING && !literal && !keybinding_action) {
            char* key = g_strdup(lua_tostring(state, -2));
            g_strdelimit(key, "_", '-');
            lua_pushstring(state, key);
            g_free(key);
        } else {
            lua_pushvalue(state, -2);
        }
        lua_pushvalue(state, -1);
        lua_rawget(state, destination);
        if (!lua_isnil(state, -1))
            luaL_error(state, "duplicate setting after snake_case conversion");
        lua_pop(state, 1);
        const char* key = lua_type(state, -1) == LUA_TSTRING ? lua_tostring(state, -1) : NULL;
        gboolean callback_binding =
            keybinding_depth == 2 && parent &&
            (g_str_equal(parent, "pointer") || g_str_equal(parent, "keyboard"));
        int child_keybinding_depth = callback_binding                     ? 3
                                     : key && !strcmp(key, "keybindings") ? 1
                                     : keybinding_depth == 1              ? 2
                                                                          : 0;
        /* Rewrite deprecated enum spellings to their current names so snapshots and events report
         * the new ones. */
        if (key && lua_type(state, -2) == LUA_TSTRING) {
            const char* replacement = deprecated_enum_replacement(key, lua_tostring(state, -2));
            if (replacement) {
                /* The stack holds the source key, the value, then the converted key. Swap the
                 * value. */
                lua_pushstring(state, replacement);
                lua_replace(state, -3);
            }
        }
        push_settings(state, -2, callback_binding ? parent : key, depth + 1,
                      child_keybinding_depth);
        lua_rawset(state, destination);
        lua_pop(state, 1);
    }
    if (marked_array(state, source) && lua_getmetatable(state, source))
        lua_setmetatable(state, destination);
}

static int lua_shortcut_input_callback(lua_State* state) {
    lua_pushvalue(state, lua_upvalueindex(1));
    lua_pushvalue(state, 1);
    if (lua_pcall(state, 1, 1, 0) != LUA_OK)
        return lua_error(state);
    if (lua_isnil(state, -1)) {
        lua_pop(state, 1);
        lua_pushliteral(state, "consume");
    }
    return 1;
}

static void clear_configured_input_binding(lua_State* state, const char* group, const char* name) {
    int base = lua_gettop(state);
    lua_getglobal(state, "gnoblin");
    int api = lua_gettop(state);
    lua_getfield(state, api, "_configured_input_bindings");
    if (!lua_istable(state, -1))
        goto done;
    lua_getfield(state, -1, group);
    if (!lua_istable(state, -1))
        goto done;
    int group_index = lua_gettop(state);
    lua_getfield(state, group_index, name);
    const char* id = lua_tostring(state, -1);
    if (!id)
        goto done;
    lua_getfield(state, api, "_input_handlers");
    if (lua_istable(state, -1)) {
        lua_pushnil(state);
        lua_setfield(state, -2, id);
    }
    lua_pop(state, 1);
    lua_getfield(state, api, "config");
    lua_getfield(state, -1, "input-handlers");
    if (lua_istable(state, -1)) {
        int handlers = lua_gettop(state);
        lua_Integer count = lua_rawlen(state, handlers);
        for (lua_Integer i = 1; i <= count; i++) {
            lua_rawgeti(state, handlers, i);
            lua_getfield(state, -1, "id");
            gboolean matches =
                lua_type(state, -1) == LUA_TSTRING && g_str_equal(lua_tostring(state, -1), id);
            lua_pop(state, 2);
            if (!matches)
                continue;
            for (lua_Integer j = i; j < count; j++) {
                lua_rawgeti(state, handlers, j + 1);
                lua_rawseti(state, handlers, j);
            }
            lua_pushnil(state);
            lua_rawseti(state, handlers, count);
            break;
        }
    }
    lua_pushnil(state);
    lua_setfield(state, group_index, name);
done:
    lua_settop(state, base);
}

static void remember_configured_input_binding(lua_State* state, const char* group, const char* name,
                                              int id_index) {
    id_index = lua_absindex(state, id_index);
    lua_getglobal(state, "gnoblin");
    lua_getfield(state, -1, "_configured_input_bindings");
    if (!lua_istable(state, -1)) {
        lua_pop(state, 1);
        lua_newtable(state);
        lua_pushvalue(state, -1);
        lua_setfield(state, -3, "_configured_input_bindings");
    }
    lua_getfield(state, -1, group);
    if (!lua_istable(state, -1)) {
        lua_pop(state, 1);
        lua_newtable(state);
        lua_pushvalue(state, -1);
        lua_setfield(state, -3, group);
    }
    lua_pushvalue(state, id_index);
    lua_setfield(state, -2, name);
    lua_pop(state, 3);
}

/* String keys of the table at INDEX in sorted order. Lua randomises its string hash seed for each
 * process, so lua_next order differs between a worker and its replacement; registering in that
 * order would make the input handler list differ and Mutter would reject the replacement's resume.
 * A key that is not a string raises ERROR_MESSAGE. */
static GPtrArray* sorted_string_keys(lua_State* state, int index, const char* error_message) {
    index = lua_absindex(state, index);
    GPtrArray* keys = g_ptr_array_new_with_free_func(g_free);
    lua_pushnil(state);
    while (lua_next(state, index)) {
        if (lua_type(state, -2) != LUA_TSTRING) {
            g_ptr_array_unref(keys);
            luaL_error(state, "%s", error_message);
        }
        g_ptr_array_add(keys, g_strdup(lua_tostring(state, -2)));
        lua_pop(state, 1);
    }
    g_ptr_array_sort(keys, compare_entry_names);
    return keys;
}

static void register_keyboard_binding_callbacks(lua_State* state, int config_index,
                                                LuaConfig* config) {
    config_index = lua_absindex(state, config_index);
    lua_getfield(state, config_index, "keybindings");
    if (!lua_istable(state, -1)) {
        lua_pop(state, 1);
        return;
    }
    lua_getfield(state, -1, "keyboard");
    lua_remove(state, -2);
    if (!lua_istable(state, -1)) {
        lua_pop(state, 1);
        return;
    }
    int bindings = lua_absindex(state, -1);
    g_autoptr(GPtrArray) names = sorted_string_keys(
        state, bindings, "keyboard binding declarations need snake_case names and tables");
    for (guint position = 0; position < names->len; position++) {
        const char* name = g_ptr_array_index(names, position);
        lua_getfield(state, bindings, name);
        if (!is_keybinding_action_name(name) || !lua_istable(state, -1)) {
            g_ptr_array_unref(g_steal_pointer(&names));
            luaL_error(state, "keyboard binding declarations need snake_case names and tables");
        }
        int declaration = lua_absindex(state, -1);
        lua_getfield(state, declaration, "enable");
        gboolean disabled = lua_isboolean(state, -1) && !lua_toboolean(state, -1);
        lua_pop(state, 1);
        clear_configured_input_binding(state, "keyboard", name);
        if (!disabled) {
            lua_getfield(state, declaration, "binding");
            if (lua_type(state, -1) != LUA_TSTRING || !lua_rawlen(state, -1))
                luaL_error(state, "keyboard binding '%s' requires a binding string", name);
            int binding = lua_absindex(state, -1);
            lua_getfield(state, declaration, "callback");
            if (!lua_isfunction(state, -1))
                luaL_error(state, "keyboard binding '%s' requires a callback function", name);
            int callback = lua_absindex(state, -1);
            lua_pushlightuserdata(state, config);
            lua_pushcclosure(state, lua_input_on, 1);
            lua_newtable(state);
            lua_pushliteral(state, "key");
            lua_setfield(state, -2, "type");
            lua_pushvalue(state, binding);
            lua_setfield(state, -2, "accelerator");
            lua_getfield(state, declaration, "trigger");
            const char* trigger = lua_tostring(state, -1);
            if (trigger && !g_str_equal(trigger, "press") && !g_str_equal(trigger, "release"))
                luaL_error(state, "keyboard binding '%s' trigger must be press or release", name);
            lua_pop(state, 1);
            lua_pushstring(state, trigger ? trigger : "press");
            lua_setfield(state, -2, "phase");
            lua_pushvalue(state, callback);
            lua_pushcclosure(state, lua_shortcut_input_callback, 1);
            lua_call(state, 2, 1);
            remember_configured_input_binding(state, "keyboard", name, -1);
            lua_pop(state, 3); /* handler id, callback, binding */
        }
        lua_pop(state, 1); /* declaration */
    }
    lua_pop(state, 1);
}

/* Register callbacks before converting the declaration into the compositor's
 * compact binding table.  This reads the user's table but never changes it. */
static void register_pointer_binding_callbacks(lua_State* state, int config_index,
                                               LuaConfig* runtime_config) {
    config_index = lua_absindex(state, config_index);
    lua_getfield(state, config_index, "keybindings");
    if (lua_isnil(state, -1)) {
        lua_pop(state, 1);
        return;
    }
    if (!lua_istable(state, -1))
        luaL_error(state, "keybindings must be a table");
    lua_getfield(state, -1, "pointer");
    lua_remove(state, -2); /* keybindings */
    if (lua_isnil(state, -1)) {
        lua_pop(state, 1);
        return;
    }
    if (!lua_istable(state, -1))
        luaL_error(state, "keybindings.pointer must be a table");
    int bindings = lua_absindex(state, -1);
    g_autoptr(GPtrArray) names =
        sorted_string_keys(state, bindings, "pointer binding names must use snake_case");
    for (guint position = 0; position < names->len; position++) {
        const char* name = g_ptr_array_index(names, position);
        if (!is_keybinding_action_name(name)) {
            g_ptr_array_unref(g_steal_pointer(&names));
            luaL_error(state, "pointer binding names must use snake_case");
        }
        lua_getfield(state, bindings, name);
        if (!lua_istable(state, -1)) {
            /* Build the message in Lua memory first: NAME belongs to the array that is freed next.
             */
            luaL_where(state, 1);
            lua_pushfstring(state, "pointer binding '%s' must be a table", name);
            lua_concat(state, 2);
            g_ptr_array_unref(g_steal_pointer(&names));
            lua_error(state);
        }
        int declaration = lua_absindex(state, -1);
        lua_getfield(state, declaration, "enable");
        if (!lua_isnil(state, -1) && !lua_isboolean(state, -1))
            luaL_error(state, "pointer binding '%s' enable must be a boolean", name);
        gboolean disabled = lua_isboolean(state, -1) && !lua_toboolean(state, -1);
        lua_pop(state, 1);
        clear_configured_input_binding(state, "pointer", name);
        if (disabled) {
            lua_pop(state, 1); /* declaration */
            continue;
        }
        lua_getfield(state, declaration, "binding");
        if (lua_type(state, -1) != LUA_TSTRING || !lua_rawlen(state, -1))
            luaL_error(state, "pointer binding '%s' requires a nonempty binding string", name);
        lua_pop(state, 1);
        lua_getfield(state, declaration, "callback");
        if (lua_type(state, -1) != LUA_TFUNCTION)
            luaL_error(state, "pointer binding '%s' requires a callback function", name);
        lua_pop(state, 1);

        lua_pushlightuserdata(state, runtime_config);
        lua_pushcclosure(state, lua_input_on, 1);
        lua_newtable(state);
        lua_pushliteral(state, "button");
        lua_setfield(state, -2, "type");
        lua_getfield(state, declaration, "binding");
        lua_setfield(state, -2, "accelerator");
        lua_getfield(state, declaration, "trigger");
        const char* trigger = lua_tostring(state, -1);
        if (trigger && !g_str_equal(trigger, "press") && !g_str_equal(trigger, "release"))
            luaL_error(state, "pointer binding '%s' trigger must be press or release", name);
        lua_pop(state, 1);
        lua_pushstring(state, trigger ? trigger : "press");
        lua_setfield(state, -2, "phase");
        lua_getfield(state, declaration, "callback");
        lua_pushcclosure(state, lua_shortcut_input_callback, 1);
        lua_call(state, 2, 1);
        remember_configured_input_binding(state, "pointer", name, -1);
        lua_pop(state, 1); /* subscription */
        lua_pop(state, 1); /* declaration: lua_next needs only its key */
    }
    lua_pop(state, 1); /* pointer bindings */
}

static const char* const known_configure_sections[] = {
    "keybindings",  "window-management", "compositor",  "input",     "input-sources",
    "monitors",     "touchpad-gestures", "permissions", "location",  "prompts",
    "auth",         "portals",           "layer-shell", "protocols", "frame-renderers",
    "cursor",       "xwayland",          "shortcuts",   "autostart", "workspaces",
    "window-rules", "animations",        NULL,
};
static const char* const known_protocols[] = {
    "wlr-layer-shell",
    "ext-foreign-toplevel-list",
    "wlr-foreign-toplevel-management",
    "xdg-decoration",
    "kde-server-decoration",
    "window-frame-renderer",
    "wlr-screencopy",
    "ext-background-effect-v1",
    "blur-fade",
    "ext-data-control",
    "ext-idle-notify",
    "wlr-gamma-control",
    "wlr-output-power-management",
    "ext-session-lock",
    NULL,
};

static const char* const unknown_section_warning =
    "gnoblin.configure: unknown section \"%s\" is ignored; check the spelling against "
    "the documented sections";
static const char* const unknown_protocol_warning =
    "gnoblin.configure: unknown protocol \"%s\" is ignored; check the spelling "
    "against the documented protocol names";

/* Warn about a top-level key that gnoblin.configure does not know. A typo such as window_managment
 * otherwise does nothing and gives no feedback. This is a warning, not an error, so a config
 * written for a newer Gnoblin still loads. The names are the documented sections, after
 * push_settings has turned underscores into dashes. */
static void warn_unknown_configure_sections(lua_State* state, int table) {
    table = lua_absindex(state, table);
    lua_pushnil(state);
    while (lua_next(state, table)) {
        const char* section = lua_type(state, -2) == LUA_TSTRING ? lua_tostring(state, -2) : NULL;
        if (section && !g_strv_contains(known_configure_sections, section))
            g_warning(unknown_section_warning, section);
        if (section && !strcmp(section, "protocols") && lua_istable(state, -1)) {
            lua_pushnil(state);
            while (lua_next(state, -2)) {
                if (lua_type(state, -2) == LUA_TSTRING &&
                    !g_strv_contains(known_protocols, lua_tostring(state, -2)))
                    g_warning(unknown_protocol_warning, lua_tostring(state, -2));
                lua_pop(state, 1);
            }
        }
        lua_pop(state, 1);
    }
}

/* Public diagnostics are derived from the accepted document, never merged into it.
 * Bound both count and key length so config typos cannot inflate reload packets or notices. */
static void add_document_warning(GPtrArray* warnings, gboolean* omitted, const char* format,
                                 const char* name) {
    if (warnings->len >= 64) {
        *omitted = TRUE;
        return;
    }
    g_autofree char* prefix = g_strndup(name, 256);
    g_autofree char* valid = g_utf8_make_valid(prefix, -1);
    g_autofree char* display = g_strconcat(valid, strlen(name) > 256 ? "..." : "", NULL);
    g_ptr_array_add(warnings, g_strdup_printf(format, display));
}

GVariant* gnoblin_config_document_warnings(GVariant* document) {
    g_autoptr(GPtrArray) warnings = g_ptr_array_new_with_free_func(g_free);
    gboolean omitted = FALSE;
    GVariantIter sections;
    const char* name;
    GVariant* value;
    g_variant_iter_init(&sections, document);
    while (g_variant_iter_next(&sections, "{&sv}", &name, &value)) {
        g_autoptr(GVariant) section = value;
        /* input-handlers is generated by input.on and callback keybindings. */
        if (!g_strv_contains(known_configure_sections, name) &&
            !g_str_equal(name, "input-handlers"))
            add_document_warning(warnings, &omitted, unknown_section_warning, name);
        if (g_str_equal(name, "protocols") &&
            g_variant_is_of_type(section, G_VARIANT_TYPE_VARDICT)) {
            GVariantIter protocols;
            const char* protocol;
            GVariant* enabled;
            g_variant_iter_init(&protocols, section);
            while (g_variant_iter_next(&protocols, "{&sv}", &protocol, &enabled)) {
                if (!g_strv_contains(known_protocols, protocol))
                    add_document_warning(warnings, &omitted, unknown_protocol_warning, protocol);
                g_variant_unref(enabled);
            }
        }
    }
    g_ptr_array_sort(warnings, compare_entry_names);
    if (omitted)
        g_ptr_array_add(warnings, g_strdup("Further configuration warnings were omitted."));
    return g_variant_ref_sink(
        g_variant_new_strv((const char* const*)warnings->pdata, warnings->len));
}

static int lua_configure(lua_State* state) {
    LuaConfig* config = lua_touserdata(state, lua_upvalueindex(1));
    luaL_checktype(state, 1, LUA_TTABLE);
    if (config && config->input_dispatching)
        return luaL_error(state,
                          "configuration changes are unavailable in an input policy handler");
    register_pointer_binding_callbacks(state, 1, config);
    register_keyboard_binding_callbacks(state, 1, config);
    push_settings(state, 1, NULL, 0, 0);
    lua_replace(state, 1);
    warn_unknown_configure_sections(state, 1);
    return lua_set(state);
}

int lua_configure_call(lua_State* state) {
    lua_remove(state, 1);
    return lua_configure(state);
}

/* Input handlers are deliberately registered separately from ordinary event
 * listeners: the native matcher receives only immutable metadata while the
 * function remains in this Lua runtime. */
int lua_input_on(lua_State* state) {
    LuaConfig* config = lua_touserdata(state, lua_upvalueindex(1));
    if (!config || lua_gettop(state) != 2 || !lua_istable(state, 1) || !lua_isfunction(state, 2))
        return luaL_error(state, "gnoblin.input.on requires a match table and handler function");
    if (config->input_dispatching)
        return luaL_error(state, "input handlers cannot be registered from an input callback");
    lua_getfield(state, 1, "type");
    const char* type = lua_tostring(state, -1);
    static const char* const types[] = {"key",   "button", "motion",  "scroll",
                                        "touch", "tablet", "gesture", NULL};
    gboolean valid_type = FALSE;
    for (guint i = 0; types[i]; i++)
        valid_type |= type && g_str_equal(type, types[i]);
    lua_pop(state, 1);
    if (!valid_type)
        return luaL_error(
            state,
            "input handler type must be key, button, motion, scroll, touch, tablet, or gesture");
    if (config->next_input_handler_id == G_MAXUINT)
        return luaL_error(state, "input handler ID space is exhausted");
    g_autofree char* id = g_strdup_printf("input-%u", ++config->next_input_handler_id);

    lua_getglobal(state, "gnoblin");
    lua_getfield(state, -1, "_input_handlers");
    if (!lua_istable(state, -1)) {
        lua_pop(state, 1);
        lua_newtable(state);
        lua_pushvalue(state, -1);
        lua_setfield(state, -3, "_input_handlers");
    }
    lua_pushvalue(state, 2);
    lua_setfield(state, -2, id);
    lua_pop(state, 2); /* handler registry, gnoblin */

    lua_getglobal(state, "gnoblin");
    lua_getfield(state, -1, "config");
    lua_getfield(state, -1, "input-handlers");
    if (!lua_istable(state, -1)) {
        lua_pop(state, 1);
        lua_newtable(state);
        mark_array_table(state, -1);
        lua_pushvalue(state, -1);
        lua_setfield(state, -3, "input-handlers");
    }
    int handlers = lua_absindex(state, -1);
    lua_newtable(state);
    lua_pushstring(state, id);
    lua_setfield(state, -2, "id");
    push_settings(state, 1, NULL, 0, 0);
    lua_setfield(state, -2, "match");
    lua_rawseti(state, handlers, lua_rawlen(state, handlers) + 1);
    lua_pop(state, 3); /* handlers, config, gnoblin */
    lua_pushstring(state, id);
    return 1;
}

void push_config_list(lua_State* state) {
    const char* section = lua_tostring(state, lua_upvalueindex(1));
    const char* key = lua_tostring(state, lua_upvalueindex(2));
    lua_getglobal(state, "gnoblin");
    lua_getfield(state, -1, "config");
    if (!lua_istable(state, -1))
        luaL_error(state, "gnoblin.config must be a table");
    if (section[0]) {
        lua_getfield(state, -1, section);
        if (lua_isnil(state, -1)) {
            lua_pop(state, 1);
            lua_newtable(state);
            lua_pushvalue(state, -1);
            lua_setfield(state, -3, section);
        }
        if (!lua_istable(state, -1))
            luaL_error(state, "%s must be a table", section);
    }
    lua_getfield(state, -1, key);
    if (lua_isnil(state, -1)) {
        lua_pop(state, 1);
        lua_newtable(state);
        lua_pushvalue(state, -1);
        lua_setfield(state, -3, key);
    }
    if (!lua_istable(state, -1))
        luaL_error(state, "%s must be a list", key);
    int list = lua_gettop(state);
    lua_Integer count = lua_rawlen(state, list), entries = 0;
    lua_pushnil(state);
    while (lua_next(state, list)) {
        if (!lua_isinteger(state, -2) || lua_tointeger(state, -2) < 1 ||
            lua_tointeger(state, -2) > count || !lua_istable(state, -1))
            luaL_error(state, "%s must be a dense list of tables", key);
        entries++;
        lua_pop(state, 1);
    }
    if (entries != count)
        luaL_error(state, "%s must be a dense list of tables", key);
}
