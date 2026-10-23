#include "gnoblin-lua-internal.h"

static int lua_config_path(lua_State* state) {
    if (lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.config.path takes no arguments");
    g_autofree char* path = gnoblin_config_path();
    lua_pushstring(state, path);
    return 1;
}

static int lua_config_default(lua_State* state) {
    if (lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.config.default takes no arguments");
    gsize length = 0;
    const char* source = gnoblin_config_default_lua(&length);
    lua_pushlstring(state, source, length);
    return 1;
}

static void version_identity_clear(VersionIdentity* identity) {
    g_clear_pointer(&identity->gnoblin, g_free);
    g_clear_pointer(&identity->gnome, g_free);
    g_clear_pointer(&identity->mutter, g_free);
    g_clear_pointer(&identity->git_remote, g_free);
    g_clear_pointer(&identity->git_sha, g_free);
    g_clear_pointer(&identity->build_id, g_free);
}

static char* sanitize_git_remote(const char* remote) {
    if (!remote || !*remote)
        return g_strdup("unknown");

    g_autoptr(GError) error = NULL;
    g_autoptr(GUri) uri = g_uri_parse(remote, G_URI_FLAGS_NONE, &error);
    if (uri && g_uri_get_scheme(uri))
        return g_uri_join(G_URI_FLAGS_NONE, g_uri_get_scheme(uri), NULL, g_uri_get_host(uri),
                          g_uri_get_port(uri), g_uri_get_path(uri), NULL, NULL);

    /* SSH's scp-like spelling has no URI scheme. Drop any user prefix rather
     * than exposing a possibly credential-bearing value. */
    const char* at = strchr(remote, '@');
    return g_strdup(at ? at + 1 : remote);
}

static char* executable_prefix(void) {
    g_autofree char* path = g_file_read_link("/proc/self/exe", NULL);
    if (!path)
        return NULL;
    g_autofree char* directory = g_path_get_dirname(path);
    return g_path_get_dirname(directory);
}

static void version_identity_load_key_file(GKeyFile* key_file, VersionIdentity* identity) {
    identity->gnoblin = g_key_file_get_string(key_file, "version", "gnoblin", NULL);
    identity->gnome = g_key_file_get_string(key_file, "version", "gnome", NULL);
    identity->mutter = g_key_file_get_string(key_file, "version", "mutter", NULL);
    identity->git_remote = g_key_file_get_string(key_file, "version", "git_remote", NULL);
    identity->git_sha = g_key_file_get_string(key_file, "version", "git_sha", NULL);
    identity->build_id = g_key_file_get_string(key_file, "version", "build_id", NULL);
}

static void load_installed_identity(VersionIdentity* identity) {
    g_autoptr(GPtrArray) candidates = g_ptr_array_new_with_free_func(g_free);
    const char* override = g_getenv("GNOBLIN_VERSION_METADATA_FILE");
    if (override && *override)
        g_ptr_array_add(candidates, g_strdup(override));
    g_autofree char* prefix = executable_prefix();
    if (prefix)
        g_ptr_array_add(candidates,
                        g_build_filename(prefix, "share", "gnoblin", "version.ini", NULL));
    const char* const* data_dirs = g_get_system_data_dirs();
    for (guint i = 0; data_dirs[i]; i++)
        g_ptr_array_add(candidates, g_build_filename(data_dirs[i], "gnoblin", "version.ini", NULL));

    for (guint i = 0; i < candidates->len; i++) {
        const char* path = g_ptr_array_index(candidates, i);
        g_autoptr(GKeyFile) key_file = g_key_file_new();
        if (!g_key_file_load_from_file(key_file, path, G_KEY_FILE_NONE, NULL))
            continue;
        version_identity_load_key_file(key_file, identity);
        return;
    }
}

static void lua_set_version_string(lua_State* state, const char* field, const char* value) {
    lua_pushstring(state, value && *value ? value : "unknown");
    lua_setfield(state, -2, field);
}

static int lua_gnoblin_version(lua_State* state) {
    if (lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.version takes no arguments");

    VersionIdentity identity = {0};
    load_installed_identity(&identity);
    g_autofree char* safe_remote = sanitize_git_remote(identity.git_remote);
    g_autofree char* api_version = NULL;
#ifdef GNOBLIN_NATIVE_CONTROL_API_MAJOR
    api_version = g_strdup_printf("%d.%d", GNOBLIN_NATIVE_CONTROL_API_MAJOR,
                                  GNOBLIN_NATIVE_CONTROL_API_MINOR);
#else
    api_version = g_strdup("unknown");
#endif

    lua_createtable(state, 0, 8);
    lua_set_version_string(state, "gnoblin", identity.gnoblin);
    lua_set_version_string(state, "gnome", identity.gnome);
    lua_set_version_string(state, "mutter", identity.mutter);
    lua_set_version_string(state, "lua", LUA_VERSION);
    lua_set_version_string(state, "api", api_version);
    lua_set_version_string(state, "git_remote", safe_remote);
    lua_set_version_string(state, "git_sha", identity.git_sha);
    lua_set_version_string(state, "build_id", identity.build_id);
    push_readonly_copy(state, -1);
    lua_remove(state, -2);
    version_identity_clear(&identity);
    return 1;
}

static void install_api(lua_State* state, LuaConfig* config) {
    luaL_openlibs(state);
    const char* blocked[] = {"os", "io", "debug", "package", "dofile", "loadfile", NULL};
    for (guint i = 0; blocked[i]; i++) {
        lua_pushnil(state);
        lua_setglobal(state, blocked[i]);
    }
    lua_newtable(state);
    lua_pushcfunction(state, lua_gnoblin_version);
    lua_setfield(state, -2, "version");
    lua_newtable(state);
    int config_table = lua_absindex(state, -1);
    lua_newtable(state);
    int config_metatable = lua_absindex(state, -1);
    lua_newtable(state);
    int config_methods = lua_absindex(state, -1);
    lua_pushcfunction(state, lua_config_path);
    lua_setfield(state, config_methods, "path");
    lua_pushcfunction(state, lua_config_default);
    lua_setfield(state, config_methods, "default");
    lua_pushlightuserdata(state, config);
    lua_pushliteral(state, "config.restore_default");
    lua_pushcclosure(state, lua_generic_api_action, 2);
    lua_setfield(state, config_methods, "restore_default");
    lua_setfield(state, config_metatable, "__index");
    lua_setmetatable(state, config_table);
    lua_setfield(state, -2, "config");
    for (guint i = 0; api_methods[i]; i++) {
        if (g_str_equal(api_methods[i], "config.restore_default"))
            continue; /* Config methods live on config's metatable, not in saved settings. */
        if (g_str_has_prefix(api_methods[i], "workspace."))
            continue; /* Lua exposes workspace operations through workspaces.*. */
        if (g_str_equal(api_methods[i], "command.run") ||
            g_str_equal(api_methods[i], "command.capture"))
            continue; /* Lua exposes command execution through commands.run() and capture(). */
        if (g_str_has_prefix(api_methods[i], "window."))
            continue; /* Lua exposes window operations on Window snapshots. */
        if (g_str_equal(api_methods[i], "input.select"))
            continue; /* Lua exposes source selection as input.select_source(). */
        if (g_str_equal(api_methods[i], "input.list") ||
            g_str_equal(api_methods[i], "input.current"))
            continue; /* These legacy names are snapshot reads, not operations. */
        if (g_str_equal(api_methods[i], "shortcut.capture") ||
            g_str_equal(api_methods[i], "shortcut.bind") ||
            g_str_equal(api_methods[i], "shortcut.unbind") ||
            g_str_equal(api_methods[i], "shortcut.session.end"))
            continue; /* Public collection API is gnoblin.shortcuts.capture. */
        if (g_str_equal(api_methods[i], "privacy.get"))
            continue; /* Exposed as the native snapshot read gnoblin.privacy.state(). */
        if (g_str_has_prefix(api_methods[i], "animation."))
            continue; /* Controls are gnoblin.animations.*; gnoblin.animation declares config. */
        const char* separator = strchr(api_methods[i], '.');
        g_autofree char* domain = g_strndup(api_methods[i], separator - api_methods[i]);
        const char* operation = separator + 1;
        lua_getfield(state, -1, domain);
        if (!lua_istable(state, -1)) {
            lua_pop(state, 1);
            lua_newtable(state);
            lua_pushvalue(state, -1);
            lua_setfield(state, -3, domain);
        }
        if (g_str_equal(api_methods[i], "session.status")) {
            lua_pushlightuserdata(state, config);
            lua_pushcclosure(state, lua_session_status, 1);
        } else if (g_str_equal(api_methods[i], "runtime.status")) {
            lua_pushcfunction(state, lua_runtime_status);
        } else if (g_str_equal(api_methods[i], "session.activity")) {
            lua_pushlightuserdata(state, config);
            lua_pushcclosure(state, lua_session_activity, 1);
        } else if (g_str_equal(api_methods[i], "session.class")) {
            lua_pushcfunction(state, lua_session_class);
        } else if (g_str_equal(api_methods[i], "input.text_target")) {
            lua_pushlightuserdata(state, config);
            lua_pushcclosure(state, lua_input_text_target, 1);
        } else {
            lua_pushlightuserdata(state, config);
            lua_pushstring(state, api_methods[i]);
            lua_pushcclosure(state,
                             g_str_has_prefix(api_methods[i], "workspace.")
                                 ? lua_workspace_action
                                 : lua_generic_api_action,
                             2);
        }
        lua_setfield(state, -2, operation);
        lua_pop(state, 1);
    }
    for (guint i = 0; i < api_aliases_count; i++) {
        const char* separator = strchr(api_aliases[i].public_name, '.');
        g_autofree char* domain =
            g_strndup(api_aliases[i].public_name, separator - api_aliases[i].public_name);
        const char* operation = separator + 1;
        lua_getfield(state, -1, domain);
        if (!lua_istable(state, -1)) {
            lua_pop(state, 1);
            lua_newtable(state);
            lua_pushvalue(state, -1);
            lua_setfield(state, -3, domain);
        }
        lua_pushlightuserdata(state, config);
        lua_pushstring(state, api_aliases[i].operation);
        lua_pushcclosure(state,
                         g_str_has_prefix(api_aliases[i].operation, "workspace.")
                             ? lua_workspace_action
                             : lua_generic_api_action,
                         2);
        lua_setfield(state, -2, operation);
        lua_pop(state, 1);
    }
    lua_getfield(state, -1, "animations");
    lua_pushlightuserdata(state, config);
    lua_pushcclosure(state, lua_animations_list, 1);
    lua_setfield(state, -2, "list");
    lua_pushlightuserdata(state, config);
    lua_pushcclosure(state, lua_animation_get, 1);
    lua_setfield(state, -2, "get");
    lua_pop(state, 1);
    lua_getfield(state, -1, "privacy");
    if (!lua_istable(state, -1)) {
        lua_pop(state, 1);
        lua_newtable(state);
        lua_pushvalue(state, -1);
        lua_setfield(state, -3, "privacy");
    }
    lua_pushlightuserdata(state, config);
    lua_pushcclosure(state, lua_privacy_state, 1);
    lua_setfield(state, -2, "state");
    lua_pop(state, 1);
    lua_newtable(state);
    lua_pushlightuserdata(state, config);
    lua_pushcclosure(state, lua_windows_list, 1);
    lua_setfield(state, -2, "list");
    lua_pushlightuserdata(state, config);
    lua_pushcclosure(state, lua_windows_focused, 1);
    lua_setfield(state, -2, "focused");
    lua_pushlightuserdata(state, config);
    lua_pushcclosure(state, lua_windows_by_id, 1);
    lua_setfield(state, -2, "by_id");
    lua_pushlightuserdata(state, config);
    lua_pushcclosure(state, lua_windows_snap_context, 1);
    lua_setfield(state, -2, "snap_context");
    lua_setfield(state, -2, "windows");
    lua_getfield(state, -1, "workspaces");
    lua_pushlightuserdata(state, config);
    lua_pushcclosure(state, lua_workspaces_list, 1);
    lua_setfield(state, -2, "list");
    lua_pushlightuserdata(state, config);
    lua_pushcclosure(state, lua_workspaces_active, 1);
    lua_setfield(state, -2, "active");
    lua_pushlightuserdata(state, config);
    lua_pushcclosure(state, lua_workspaces_by_id, 1);
    lua_setfield(state, -2, "by_id");
    lua_pop(state, 1);
    lua_newtable(state);
    lua_pushlightuserdata(state, config);
    lua_pushcclosure(state, lua_monitors_list, 1);
    lua_setfield(state, -2, "list");
    lua_pushlightuserdata(state, config);
    lua_pushcclosure(state, lua_monitors_primary, 1);
    lua_setfield(state, -2, "primary");
    lua_pushlightuserdata(state, config);
    lua_pushcclosure(state, lua_monitors_privacy_screen, 1);
    lua_setfield(state, -2, "privacy_screen");
    lua_pushlightuserdata(state, config);
    lua_pushcclosure(state, lua_monitors_set_privacy_screen, 1);
    lua_setfield(state, -2, "set_privacy_screen");
    lua_setfield(state, -2, "monitors");
    lua_newtable(state);
    lua_pushlightuserdata(state, config);
    lua_pushcclosure(state, lua_outputs_list, 1);
    lua_setfield(state, -2, "list");
    lua_pushlightuserdata(state, config);
    lua_pushcclosure(state, lua_outputs_primary, 1);
    lua_setfield(state, -2, "primary");
    lua_pushlightuserdata(state, config);
    lua_pushcclosure(state, lua_outputs_by_id, 1);
    lua_setfield(state, -2, "by_id");
    lua_pushlightuserdata(state, config);
    lua_pushcclosure(state, lua_outputs_renderer, 1);
    lua_setfield(state, -2, "renderer");
    lua_setfield(state, -2, "outputs");
    lua_newtable(state);
    lua_pushlightuserdata(state, config);
    lua_pushcclosure(state, lua_layers_list, 1);
    lua_setfield(state, -2, "list");
    lua_pushlightuserdata(state, config);
    lua_pushcclosure(state, lua_layers_animation_policy, 1);
    lua_setfield(state, -2, "animation_policy");
    lua_setfield(state, -2, "layers");
    lua_getfield(state, -1, "capabilities");
    if (!lua_istable(state, -1)) {
        lua_pop(state, 1);
        lua_newtable(state);
        lua_pushvalue(state, -1);
        lua_setfield(state, -3, "capabilities");
    }
    lua_pushlightuserdata(state, config);
    lua_pushcclosure(state, lua_capabilities_list, 1);
    lua_setfield(state, -2, "list");
    lua_pop(state, 1);
    lua_getfield(state, -1, "input");
    lua_pushlightuserdata(state, config);
    lua_pushcclosure(state, lua_input_devices, 1);
    lua_setfield(state, -2, "devices");
    lua_pushlightuserdata(state, config);
    lua_pushcclosure(state, lua_input_sources, 1);
    lua_setfield(state, -2, "sources");
    lua_pushlightuserdata(state, config);
    lua_pushcclosure(state, lua_input_current_source, 1);
    lua_setfield(state, -2, "current_source");
    lua_pushlightuserdata(state, config);
    lua_pushcclosure(state, lua_input_orientation_lock, 1);
    lua_setfield(state, -2, "orientation_lock");
    lua_pushlightuserdata(state, config);
    lua_pushcclosure(state, lua_input_on, 1);
    lua_setfield(state, -2, "on");
    lua_getfield(state, -1, "sources");
    lua_setfield(state, -2, "list");
    lua_getfield(state, -1, "current_source");
    lua_setfield(state, -2, "current");
    lua_pop(state, 1);
    lua_newtable(state);
    lua_pushlightuserdata(state, config);
    lua_pushcclosure(state, lua_appearance_color_scheme, 1);
    lua_setfield(state, -2, "color_scheme");
    lua_setfield(state, -2, "appearance");
    lua_newtable(state);
    lua_pushcfunction(state, lua_shortcut_actions);
    lua_setfield(state, -2, "actions");
    lua_pushlightuserdata(state, config);
    lua_pushcclosure(state, lua_shortcuts_list, 1);
    lua_setfield(state, -2, "list");
    lua_pushlightuserdata(state, config);
    lua_pushliteral(state, "shortcut.capture");
    lua_pushcclosure(state, lua_generic_api_action, 2);
    lua_setfield(state, -2, "capture");
    lua_pushlightuserdata(state, config);
    lua_pushliteral(state, "shortcut.bind");
    lua_pushcclosure(state, lua_generic_api_action, 2);
    lua_setfield(state, -2, "bind");
    lua_pushlightuserdata(state, config);
    lua_pushliteral(state, "shortcut.unbind");
    lua_pushcclosure(state, lua_generic_api_action, 2);
    lua_setfield(state, -2, "unbind");
    lua_pushlightuserdata(state, config);
    lua_pushliteral(state, "shortcut.session.end");
    lua_pushcclosure(state, lua_generic_api_action, 2);
    lua_setfield(state, -2, "end_session");
    lua_setfield(state, -2, "shortcuts");
    lua_newtable(state);
    lua_pushlightuserdata(state, config);
    lua_pushcclosure(state, lua_launches_list, 1);
    lua_setfield(state, -2, "list");
    lua_pushlightuserdata(state, config);
    lua_pushcclosure(state, lua_launches_snapshot, 1);
    lua_setfield(state, -2, "snapshot");
    lua_pushlightuserdata(state, config);
    lua_pushcclosure(state, lua_launches_begin, 1);
    lua_setfield(state, -2, "begin");
    lua_pushlightuserdata(state, config);
    lua_pushcclosure(state, lua_launches_finish, 1);
    lua_setfield(state, -2, "finish");
    lua_pushlightuserdata(state, config);
    lua_pushcclosure(state, lua_launches_finish, 1);
    lua_setfield(state, -2, "end");
    lua_setfield(state, -2, "launches");
    lua_pushcfunction(state, lua_on);
    lua_setfield(state, -2, "on");
    lua_newtable(state);
    int events = lua_absindex(state, -1);
    lua_pushcfunction(state, lua_on);
    lua_setfield(state, events, "on");
    lua_pushcfunction(state, lua_once);
    lua_setfield(state, events, "once");
    lua_newtable(state);
    lua_pushcfunction(state, lua_mutter_on);
    lua_setfield(state, -2, "on");
    lua_pushcfunction(state, lua_mutter_once);
    lua_setfield(state, -2, "once");
    lua_setfield(state, events, "mutter");
    lua_setfield(state, -2, "events");
    lua_newtable(state);
    lua_setfield(state, -2, "listeners");
    lua_newtable(state);
    lua_newtable(state);
    lua_pushlightuserdata(state, config);
    lua_pushcclosure(state, lua_configure_call, 1);
    lua_setfield(state, -2, "__call");
    lua_setmetatable(state, -2);
    install_named_view(state, "shortcuts");
    install_named_view(state, "autostart");
    lua_setfield(state, -2, "configure");
    lua_pushcfunction(state, lua_snapshot);
    lua_setfield(state, -2, "snapshot");
    const struct {
        const char *name, *section, *key;
        gboolean named;
    } declarations[] = {
        {"window_rule", "", "window-rules", FALSE},
        {"animation", "", "animations", TRUE},
    };
    for (guint i = 0; i < G_N_ELEMENTS(declarations); i++) {
        lua_pushstring(state, declarations[i].section);
        lua_pushstring(state, declarations[i].key);
        lua_pushboolean(state, declarations[i].named);
        lua_pushcclosure(state, lua_declare, 3);
        lua_setfield(state, -2, declarations[i].name);
    }
    lua_pushlightuserdata(state, config);
    lua_pushcclosure(state, lua_config_load, 1);
    lua_setfield(state, -2, "load");
    lua_pushcfunction(state, lua_array);
    lua_setfield(state, -2, "array");
    lua_newtable(state);
    lua_pushlightuserdata(state, config);
    lua_pushcclosure(state, lua_gnoblin_index, 1);
    lua_setfield(state, -2, "__index");
    lua_setmetatable(state, -2);
    lua_setglobal(state, "gnoblin");
    gnoblin_lua_extensions_install(state, config);
    lua_pushlightuserdata(state, config);
    lua_pushcclosure(state, lua_require, 1);
    lua_setglobal(state, "require");
}

gboolean evaluate_path(lua_State* state, LuaConfig* config, const char* given, gboolean root,
                       GError** error) {
    g_autofree char* path = g_canonicalize_filename(given, NULL);
    if (!g_str_has_suffix(path, ".lua")) {
        g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                    "%s: Gnoblin configuration uses Lua only. Convert this file to Lua and "
                    "give it a .lua filename.",
                    path);
        return FALSE;
    }
    add_path(config->paths, path);
    if (g_str_has_suffix(path, ".lua") && !g_file_test(path, G_FILE_TEST_EXISTS)) {
        if (root)
            return TRUE;
        g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_NOENT, "%s: config file does not exist",
                    path);
        return FALSE;
    }
    if (g_hash_table_contains(config->active, path)) {
        g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL, "%s: config cycle", path);
        return FALSE;
    }
    if (g_hash_table_size(config->active) >= MAX_CONFIG_FILES) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                            "config nesting exceeds 32 files");
        return FALSE;
    }
    g_hash_table_add(config->active, g_strdup(path));
    g_autofree char* old = g_steal_pointer(&config->current_path);
    config->current_path = g_strdup(path);
    int status = luaL_loadfilex(state, path, "t");
    if (status == LUA_OK)
        status = lua_pcall(state, 0, 1, 0);
    gboolean ok;
    if (status != LUA_OK) {
        g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL, "%s: %s", path, lua_error_text(state));
        lua_pop(state, 1);
        ok = FALSE;
    } else if (lua_isnil(state, -1)) {
        lua_pop(state, 1);
        ok = TRUE;
    } else if (!lua_istable(state, -1)) {
        lua_pop(state, 1);
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                            "Lua config must return a table or nil");
        ok = FALSE;
    } else {
        g_autoptr(GError) conversion = NULL;
        g_autoptr(GVariant) value = variant_from_lua(state, -1, 0, FALSE, FALSE, &conversion);
        ok = value != NULL && g_variant_is_of_type(value, G_VARIANT_TYPE_VARDICT);
        if (ok) {
            lua_getglobal(state, "gnoblin");
            lua_getfield(state, -1, "config");
            merge_table(state, -1, -3, NULL, TRUE);
            lua_pop(state, 3);
        } else {
            lua_pop(state, 1);
            if (conversion)
                g_propagate_error(error, g_steal_pointer(&conversion));
            else
                g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                                    "Lua config must return a table of settings");
        }
    }
    g_free(config->current_path);
    config->current_path = g_steal_pointer(&old);
    g_hash_table_remove(config->active, path);
    return ok;
}

gboolean evaluate_embedded_path(lua_State* state, LuaConfig* config, const char* relative,
                                GError** error) {
    const GnoblinConfigDefaultFile* file = gnoblin_config_default_file(relative);
    if (!file) {
        g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_NOENT,
                    "embedded default config file does not exist: %s", relative);
        return FALSE;
    }
    g_autofree char* path = g_build_filename("/gnoblin-builtin", relative, NULL);
    add_path(config->paths, path);
    if (g_hash_table_contains(config->active, path)) {
        g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL, "%s: config cycle", path);
        return FALSE;
    }
    if (g_hash_table_size(config->active) >= MAX_CONFIG_FILES) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                            "config nesting exceeds 32 files");
        return FALSE;
    }
    g_hash_table_add(config->active, g_strdup(path));
    g_autofree char* old = g_steal_pointer(&config->current_path);
    config->current_path = g_strdup(path);
    int status = luaL_loadbufferx(state, file->contents, file->length, path, "t");
    if (status == LUA_OK)
        status = lua_pcall(state, 0, 1, 0);
    gboolean ok;
    if (status != LUA_OK) {
        g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL, "%s: %s", path, lua_error_text(state));
        lua_pop(state, 1);
        ok = FALSE;
    } else if (lua_isnil(state, -1)) {
        lua_pop(state, 1);
        ok = TRUE;
    } else if (!lua_istable(state, -1)) {
        lua_pop(state, 1);
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                            "Lua config must return a table or nil");
        ok = FALSE;
    } else {
        g_autoptr(GError) conversion = NULL;
        g_autoptr(GVariant) value = variant_from_lua(state, -1, 0, FALSE, FALSE, &conversion);
        ok = value != NULL && g_variant_is_of_type(value, G_VARIANT_TYPE_VARDICT);
        if (ok) {
            lua_getglobal(state, "gnoblin");
            lua_getfield(state, -1, "config");
            merge_table(state, -1, -3, NULL, TRUE);
            lua_pop(state, 3);
        } else {
            lua_pop(state, 1);
            if (conversion)
                g_propagate_error(error, g_steal_pointer(&conversion));
            else
                g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                                    "Lua config must return a table of settings");
        }
    }
    g_free(config->current_path);
    config->current_path = g_steal_pointer(&old);
    g_hash_table_remove(config->active, path);
    return ok;
}

int protected_eval(lua_State* state) {
    EvalRun* run = lua_touserdata(state, lua_upvalueindex(1));

    install_api(state, run->config);
    console_register_signatures(state);
    if (run->source) {
        if (luaL_loadbufferx(state, run->source, run->source_length, "@<built-in init.lua>", "t") !=
                LUA_OK ||
            lua_pcall(state, 0, 0, 0) != LUA_OK) {
            g_set_error(&run->error, G_FILE_ERROR, G_FILE_ERROR_INVAL, "%s", lua_error_text(state));
            lua_pop(state, 1);
            return 0;
        }
    } else if (run->config->embedded_defaults) {
        if (!evaluate_embedded_path(state, run->config, "init.lua", &run->error))
            return 0;
    } else if (!evaluate_path(state, run->config, run->path, TRUE, &run->error)) {
        return 0;
    }
    lua_getglobal(state, "gnoblin");
    lua_getfield(state, -1, "config");
    finish_named_entries(state, -1, "shortcuts");
    finish_named_entries(state, -1, "autostart");
    run->result = variant_from_lua(state, -1, 0, FALSE, 0, &run->error);
    lua_pop(state, 2);
    return 0;
}

GVariant* gnoblin_config_evaluate_file(const char* path, GPtrArray* paths, GPtrArray* directories,
                                       GError** error) {
    LuaConfig config = {.paths = paths ? paths : g_ptr_array_new_with_free_func(g_free),
                        .directories = directories,
                        .active = g_hash_table_new_full(g_str_hash, g_str_equal, g_free, NULL),
                        .modules = g_hash_table_new_full(g_str_hash, g_str_equal, g_free, NULL),
                        .current_path = g_canonicalize_filename(path, NULL)};
    lua_State* state = new_config_state(&config);
    EvalRun run = {.config = &config, .path = path};
    int steps = MAX_CONFIG_STEPS / 1000;
    if (!state) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_NOMEM,
                            "Lua config exceeded memory limit");
        goto out;
    }
    memcpy(lua_getextraspace(state), &steps, sizeof steps);
    lua_sethook(state, limit_hook, LUA_MASKCOUNT, 1000);
    lua_pushlightuserdata(state, &run);
    lua_pushcclosure(state, protected_eval, 1);
    if (lua_pcall(state, 0, 0, 0) != LUA_OK) {
        g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL, "%s", lua_error_text(state));
        lua_pop(state, 1);
    } else if (run.error) {
        g_propagate_error(error, g_steal_pointer(&run.error));
    }
out:
    if (state)
        lua_close(state);
    g_hash_table_unref(config.active);
    g_hash_table_unref(config.modules);
    g_free(config.current_path);
    if (!paths)
        g_ptr_array_free(config.paths, TRUE);
    return run.result;
}
