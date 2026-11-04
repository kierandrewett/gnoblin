#include "gnoblin-lua-internal.h"

int lua_config_path(lua_State* state) {
    if (lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.config.path takes no arguments");
    g_autofree char* path = gnoblin_config_path();
    lua_pushstring(state, path);
    return 1;
}

int lua_config_default(lua_State* state) {
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

int lua_gnoblin_version(lua_State* state) {
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

/* The table that holds the last component of `path`, created under `root` when absent. Leaves it
 * on the stack and returns that last component. */
static const char* push_api_parent(lua_State* state, int root, const char* path) {
    lua_pushvalue(state, root);
    const char* name = path;
    for (const char* dot; (dot = strchr(name, '.')); name = dot + 1) {
        g_autofree char* part = g_strndup(name, dot - name);
        lua_getfield(state, -1, part);
        if (!lua_istable(state, -1)) {
            lua_pop(state, 1);
            lua_newtable(state);
            lua_pushvalue(state, -1);
            lua_setfield(state, -3, part);
        }
        lua_remove(state, -2);
    }
    return name;
}

/* Positional Lua entry point for an implementation that takes one table of named fields, the
 * form socket clients send. Upvalue 1 is the implementation and upvalue 2 the API path. Each
 * argument goes to the field that its annotated parameter names; a trailing parameter called
 * `options` is a table whose fields are added instead. Nil arguments are left out. */
static int lua_positional_call(lua_State* state) {
    const char* path = lua_tostring(state, lua_upvalueindex(2));
    const char* names[8];
    gsize count = lua_api_parameter_names(path, names, G_N_ELEMENTS(names));
    int supplied = lua_gettop(state);
    if (supplied > (int)count)
        return luaL_error(state, "gnoblin.%s takes at most %d arguments", path, (int)count);
    lua_createtable(state, 0, (int)count);
    int table = lua_gettop(state);
    for (int i = 1; i <= supplied; i++) {
        if (lua_isnil(state, i))
            continue;
        if (!g_str_equal(names[i - 1], "options")) {
            lua_pushvalue(state, i);
            lua_setfield(state, table, names[i - 1]);
            continue;
        }
        if (!lua_istable(state, i))
            return luaL_error(state, "gnoblin.%s options must be a table", path);
        lua_pushnil(state);
        while (lua_next(state, i)) {
            if (lua_type(state, -2) != LUA_TSTRING)
                return luaL_error(state, "gnoblin.%s options take only named fields", path);
            lua_pushvalue(state, -2);
            if (lua_rawget(state, table) != LUA_TNIL)
                return luaL_error(state, "gnoblin.%s options cannot set %s", path,
                                  lua_tostring(state, -3));
            lua_pop(state, 1);
            lua_pushvalue(state, -2);
            lua_insert(state, -2);
            lua_rawset(state, table);
        }
    }
    lua_pushvalue(state, lua_upvalueindex(1));
    lua_pushvalue(state, table);
    lua_call(state, 1, LUA_MULTRET);
    return lua_gettop(state) - table;
}

#define LUA_CONFIG_STATE_KEY "gnoblin.config_state"

/* The LuaConfig of the runtime that owns `state`. */
LuaConfig* lua_state_config(lua_State* state) {
    lua_getfield(state, LUA_REGISTRYINDEX, LUA_CONFIG_STATE_KEY);
    LuaConfig* config = lua_touserdata(state, -1);
    lua_pop(state, 1);
    return config;
}

/* Whether the runtime of `state` is handling an event, a callback or a console or API call, as
 * opposed to loading config files. Changes to the configuration mean something else then: they are
 * committed to the live session. */
gboolean lua_in_runtime(lua_State* state) {
    LuaConfig* config = lua_state_config(state);
    return config && (config->dispatching || config->api_calling);
}

/* The document that config files and run-time calls build: the settings in the form the compositor
 * reads. Lua code cannot reach it. It reads the committed settings with gnoblin.settings, the
 * document built so far while the config loads with gnoblin.snapshot(), and changes it with
 * gnoblin.configure. */
#define LUA_DOCUMENT_KEY "gnoblin.document"

/* Push the document under construction; an empty one on first use. */
void push_document(lua_State* state) {
    if (lua_getfield(state, LUA_REGISTRYINDEX, LUA_DOCUMENT_KEY) != LUA_TTABLE) {
        lua_pop(state, 1);
        lua_newtable(state);
        lua_pushvalue(state, -1);
        lua_setfield(state, LUA_REGISTRYINDEX, LUA_DOCUMENT_KEY);
    }
}

/* Make `document` the one under construction, as when a failed change is undone. */
void set_document(lua_State* state, GVariant* document) {
    push_variant(state, document);
    lua_setfield(state, LUA_REGISTRYINDEX, LUA_DOCUMENT_KEY);
}

/* gnoblin.config.<name>: the three functions, and a clear error for anything else. */
static int lua_config_index(lua_State* state) {
    lua_pushvalue(state, 2);
    lua_rawget(state, lua_upvalueindex(1));
    if (!lua_isnil(state, -1))
        return 1;
    return luaL_error(state,
                      "gnoblin.config holds path(), default() and restore_default(), not the "
                      "settings. Read gnoblin.snapshot() while the config loads and "
                      "gnoblin.settings at run time.");
}

static int lua_config_newindex(lua_State* state) {
    return luaL_error(state,
                      "gnoblin.config cannot be assigned to. Declare settings with "
                      "gnoblin.configure {...}: it merges them while the config loads and, at run "
                      "time, returns an Operation that finishes when the change is committed. "
                      "gnoblin.settings.override(path, value) layers a value over one setting and "
                      "returns an Override you can revoke.");
}

static void install_api(lua_State* state, LuaConfig* config) {
    luaL_openlibs(state);
    const char* blocked[] = {"os", "io", "debug", "package", "dofile", "loadfile", NULL};
    for (guint i = 0; blocked[i]; i++) {
        lua_pushnil(state);
        lua_setglobal(state, blocked[i]);
    }
    lua_pushlightuserdata(state, config);
    lua_setfield(state, LUA_REGISTRYINDEX, LUA_CONFIG_STATE_KEY);
    lua_newtable(state);
    int gnoblin = lua_absindex(state, -1);
    /* gnoblin.config holds only the functions that act on the configuration files. The settings
     * are not in it: Lua reads them with gnoblin.snapshot() and gnoblin.settings and writes them
     * with gnoblin.configure, so no assignment changes them unseen. */
    lua_newtable(state);
    int config_table = lua_absindex(state, -1);
    lua_newtable(state);
    int config_metatable = lua_absindex(state, -1);
    lua_newtable(state);
    int config_methods = lua_absindex(state, -1);
    for (const LuaApiEntry* entry = lua_api; entry->path; entry++) {
        if (strchr(entry->path, ':'))
            continue; /* Record methods are added to each snapshot record. */
        const char* name = g_str_has_prefix(entry->path, "config.")
                               ? push_api_parent(state, config_methods, entry->path + 7)
                               : push_api_parent(state, gnoblin, entry->path);
        lua_pushlightuserdata(state, config);
        if (entry->function) {
            lua_pushcclosure(state, entry->function, 1);
        } else {
            lua_pushstring(state, entry->operation->name);
            lua_pushcclosure(state,
                             entry->operation->flags & GN_OP_WORKSPACE ? lua_workspace_action
                                                                       : lua_generic_api_action,
                             2);
        }
        if (entry->named_arguments) {
            lua_pushstring(state, entry->path);
            lua_pushcclosure(state, lua_positional_call, 2);
        }
        lua_setfield(state, -2, name);
        lua_pop(state, 1);
    }
    lua_pushvalue(state, config_methods);
    lua_pushcclosure(state, lua_config_index, 1);
    lua_setfield(state, config_metatable, "__index");
    lua_pushcfunction(state, lua_config_newindex);
    lua_setfield(state, config_metatable, "__newindex");
    lua_pushvalue(state, config_metatable);
    lua_setmetatable(state, config_table);
    lua_pushvalue(state, config_table);
    lua_setfield(state, gnoblin, "config");
    lua_settop(state, gnoblin);
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
            push_document(state);
            merge_table(state, -1, -2, NULL, TRUE);
            lua_pop(state, 2);
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
            push_document(state);
            merge_table(state, -1, -2, NULL, TRUE);
            lua_pop(state, 2);
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

/* Directories searched for system drop-ins, lowest priority first. XDG_DATA_DIRS lists the
 * highest priority first, so it is walked backwards; /etc/gnoblin/conf.d comes last. */
static GPtrArray* system_dropin_directories(void) {
    GPtrArray* directories = g_ptr_array_new_with_free_func(g_free);
    const char* value = g_getenv("XDG_DATA_DIRS");
    g_auto(GStrv) data_dirs =
        g_strsplit(value && *value ? value : "/usr/local/share:/usr/share", ":", -1);
    for (guint i = g_strv_length(data_dirs); i-- > 0;) {
        if (!g_path_is_absolute(data_dirs[i]))
            continue;
        char* directory = g_build_filename(data_dirs[i], "gnoblin", "conf.d", NULL);
        /* A repeated entry counts at its first, highest-priority position. */
        for (guint j = 0; j < directories->len; j++) {
            if (g_str_equal(g_ptr_array_index(directories, j), directory)) {
                g_ptr_array_remove_index(directories, j);
                break;
            }
        }
        g_ptr_array_add(directories, directory);
    }
    g_ptr_array_add(directories, g_strdup("/etc/gnoblin/conf.d"));
    return directories;
}

static gint compare_names(gconstpointer a, gconstpointer b) {
    return strcmp(*(const char* const*)a, *(const char* const*)b);
}

/* The visible regular *.lua files of one drop-in directory in byte order. A missing directory is
 * not an error; any other failure to read it is returned in `error`. */
static GPtrArray* dropin_files(const char* directory, GError** error) {
    g_autoptr(GPtrArray) files = g_ptr_array_new_with_free_func(g_free);
    g_autoptr(GError) local = NULL;
    g_autoptr(GDir) dir = g_dir_open(directory, 0, &local);
    if (!dir) {
        if (g_error_matches(local, G_FILE_ERROR, G_FILE_ERROR_NOENT) ||
            g_error_matches(local, G_FILE_ERROR, G_FILE_ERROR_NOTDIR))
            return g_steal_pointer(&files);
        g_propagate_prefixed_error(error, g_steal_pointer(&local), "%s: ", directory);
        return NULL;
    }
    const char* name;
    while ((name = g_dir_read_name(dir))) {
        if (name[0] == '.' || !g_str_has_suffix(name, ".lua"))
            continue;
        g_autofree char* path = g_build_filename(directory, name, NULL);
        if (g_file_test(path, G_FILE_TEST_IS_REGULAR))
            g_ptr_array_add(files, g_steal_pointer(&path));
    }
    g_ptr_array_sort(files, compare_names);
    return g_steal_pointer(&files);
}

gboolean gnoblin_config_has_system_dropins(void) {
    g_autoptr(GPtrArray) directories = system_dropin_directories();
    for (guint i = 0; i < directories->len; i++) {
        g_autoptr(GPtrArray) files = dropin_files(g_ptr_array_index(directories, i), NULL);
        if (files && files->len > 0)
            return TRUE;
    }
    return FALSE;
}

static void skip_dropin(LuaConfig* config, GError** error) {
    if (!config->skipped_files)
        config->skipped_files = g_ptr_array_new_with_free_func(g_free);
    g_ptr_array_add(config->skipped_files, g_strdup_printf("ignored file %s", (*error)->message));
    g_clear_error(error);
}

/* Run the packaged and site drop-ins, each in its own protected call so that a broken file
 * is reported but stops neither the other drop-ins nor the config that follows. Directories are
 * recorded even when absent, so a package installed later is noticed. */
static void load_system_dropins(lua_State* state, LuaConfig* config) {
    /* A drop-in is an ordinary file on disk, never part of the embedded tree. */
    gboolean embedded = config->embedded_defaults;
    config->embedded_defaults = FALSE;
    g_autoptr(GPtrArray) directories = system_dropin_directories();
    for (guint i = 0; i < directories->len; i++) {
        const char* directory = g_ptr_array_index(directories, i);
        if (config->directories) {
            gboolean known = FALSE;
            for (guint j = 0; j < config->directories->len && !known; j++)
                known = g_str_equal(g_ptr_array_index(config->directories, j), directory);
            if (!known)
                g_ptr_array_add(config->directories, g_strdup(directory));
        }
        g_autoptr(GError) error = NULL;
        g_autoptr(GPtrArray) files = dropin_files(directory, &error);
        if (!files) {
            skip_dropin(config, &error);
            continue;
        }
        for (guint j = 0; j < files->len; j++) {
            if (!evaluate_path(state, config, g_ptr_array_index(files, j), FALSE, &error))
                skip_dropin(config, &error);
        }
    }
    config->embedded_defaults = embedded;
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
        if (run->system_dropins)
            load_system_dropins(state, run->config);
    } else {
        if (run->system_dropins)
            load_system_dropins(state, run->config);
        if (!evaluate_path(state, run->config, run->path, TRUE, &run->error))
            return 0;
    }
    push_document(state);
    finish_named_entries(state, -1, "shortcuts");
    finish_named_entries(state, -1, "autostart");
    run->result = variant_from_lua(state, -1, 0, FALSE, 0, &run->error);
    lua_pop(state, 1);
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
