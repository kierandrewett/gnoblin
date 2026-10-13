#include "gnoblinctl-internal.h"

static int lua_cli_event_subscription_unsubscribe(lua_State* state);

gboolean cli_lua_event_dispatch(CliLuaEventSubscription* subscription, JsonNode* event) {
    if (!subscription || !subscription->cli || !subscription->cli->lua_state || !event)
        return FALSE;
    Cli* cli = subscription->cli;
    lua_State* state = cli->lua_state;
    int stack_top = lua_gettop(state);
    subscription->dispatching = TRUE;
    if (subscription->once)
        subscription->once_fired = TRUE;
    if (JSON_NODE_HOLDS_OBJECT(event))
        cli_lua_location_request_track(subscription, json_node_get_object(event));
    lua_rawgeti(state, LUA_REGISTRYINDEX, subscription->callback_ref);
    json_to_lua(state, event);
    if (JSON_NODE_HOLDS_OBJECT(event)) {
        JsonObject* object = json_node_get_object(event);
        const char* event_name = member_string(object, "event", NULL);
        if (event_name) {
            lua_pushstring(state, event_name);
            lua_setfield(state, -2, "name");
        }
        const char* token = member_string(object, "focus_context", NULL);
        if (token && *token) {
            lua_cli_push_focus_context(state, subscription, token);
            lua_setfield(state, -2, "focus_context");
        }

        const char* menu_token = member_string(object, "menu_context", NULL);
        if (menu_token) {
            const char* event_name = member_string(object, "event", NULL);
            const char* menu_type = member_string(object, "menu_type", NULL);
            lua_pushnil(state);
            lua_setfield(state, -2, "menu_context");
            if (*menu_token && event_name && menu_type &&
                g_str_equal(event_name, "gnoblin.window.menu-requested") &&
                g_str_equal(menu_type, "wm")) {
                lua_cli_push_menu_context(state, subscription, menu_token);
                lua_setfield(state, -2, "menu_context");
            }
        }
    }
    gboolean succeeded = lua_pcall(state, 1, 0, 0) == LUA_OK;
    if (!succeeded) {
        const char* message = lua_tostring(state, -1);
        g_printerr("gnoblinctl lua: event callback for %s failed: %s\n", subscription->event_name,
                   message ? message : "unknown Lua error");
    }
    lua_settop(state, stack_top);
    subscription->dispatching = FALSE;
    if (subscription->once && subscription->active)
        cli_lua_event_subscription_detach(subscription);
    if (!subscription->active && subscription->callback_ref != LUA_NOREF) {
        luaL_unref(state, LUA_REGISTRYINDEX, subscription->callback_ref);
        subscription->callback_ref = LUA_NOREF;
    }
    return succeeded;
}

static gboolean cli_lua_event_process_pending(CliLuaEventSubscription* subscription) {
    while (subscription->active && !subscription->once_fired) {
        char* line_end = memchr(subscription->pending->str, '\n', subscription->pending->len);
        if (!line_end)
            return TRUE;
        g_autofree char* line =
            g_strndup(subscription->pending->str, line_end - subscription->pending->str);
        g_string_erase(subscription->pending, 0, line_end - subscription->pending->str + 1);
        g_autoptr(JsonParser) parser = json_parser_new();
        g_autoptr(GError) error = NULL;
        if (!json_parser_load_from_data(parser, line, -1, &error) ||
            !JSON_NODE_HOLDS_OBJECT(json_parser_get_root(parser))) {
            g_printerr("gnoblinctl lua: invalid event from compositor: %s\n",
                       error ? error->message : "expected an object");
            subscription->cli->lua_event_failed = TRUE;
            return FALSE;
        }
        JsonNode* event_node = json_parser_get_root(parser);
        JsonObject* event_object = json_node_get_object(event_node);
        const char* event_name = member_string(event_object, "event", "");
        if (g_str_equal(event_name, "error")) {
            g_printerr("gnoblinctl lua: event subscription failed: %s\n",
                       member_string(event_object, "message", "compositor rejected event"));
            subscription->cli->lua_event_failed = TRUE;
            return FALSE;
        }
        if (subscription->shortcut_owner_transport)
            cli_lua_shortcut_owner_dispatch(subscription, event_node);
        else if (g_str_equal(event_name, subscription->event_name))
            cli_lua_event_dispatch(subscription, event_node);
    }
    return TRUE;
}

static gboolean cli_lua_event_process_pending_idle(gpointer data) {
    CliLuaEventSubscription* subscription = data;
    subscription->pending_source_id = 0;
    if (subscription->active && !cli_lua_event_process_pending(subscription))
        cli_lua_event_subscription_detach(subscription);
    return G_SOURCE_REMOVE;
}

static void cli_lua_event_fail(CliLuaEventSubscription* subscription, const char* message) {
    if (!subscription || !subscription->active)
        return;
    g_printerr("gnoblinctl lua: event subscription for %s ended: %s\n", subscription->event_name,
               message ? message : "compositor disconnected");
    subscription->cli->lua_event_failed = TRUE;
    cli_lua_event_subscription_detach(subscription);
}

static gboolean cli_lua_event_ready(gint fd, GIOCondition condition, gpointer data) {
    (void)fd;
    CliLuaEventSubscription* subscription = data;
    if (!subscription->active)
        return G_SOURCE_REMOVE;
    if (condition & G_IO_NVAL) {
        subscription->source_id = 0;
        cli_lua_event_fail(subscription, "invalid event socket");
        return G_SOURCE_REMOVE;
    }
    if (subscription->pending->len > 0 && !cli_lua_event_process_pending(subscription)) {
        subscription->source_id = 0;
        cli_lua_event_subscription_detach(subscription);
        return G_SOURCE_REMOVE;
    }
    while (subscription->active) {
        char chunk[4096];
        g_autoptr(GError) error = NULL;
        gssize count = g_socket_receive(g_socket_connection_get_socket(subscription->connection),
                                        chunk, sizeof chunk, NULL, &error);
        if (count < 0) {
            if (g_error_matches(error, G_IO_ERROR, G_IO_ERROR_WOULD_BLOCK)) {
                if (condition & (G_IO_HUP | G_IO_ERR)) {
                    subscription->source_id = 0;
                    cli_lua_event_fail(subscription, "compositor disconnected");
                    return G_SOURCE_REMOVE;
                }
                return G_SOURCE_CONTINUE;
            }
            subscription->source_id = 0;
            cli_lua_event_fail(subscription, error ? error->message : "socket read failed");
            return G_SOURCE_REMOVE;
        }
        if (count == 0) {
            subscription->source_id = 0;
            cli_lua_event_fail(subscription, "compositor disconnected");
            return G_SOURCE_REMOVE;
        }
        g_string_append_len(subscription->pending, chunk, count);
        if (subscription->pending->len > 4 * 1024 * 1024) {
            subscription->source_id = 0;
            cli_lua_event_fail(subscription, "event data exceeded 4 MiB");
            return G_SOURCE_REMOVE;
        }
        if (!cli_lua_event_process_pending(subscription)) {
            subscription->source_id = 0;
            cli_lua_event_subscription_detach(subscription);
            return G_SOURCE_REMOVE;
        }
    }
    return G_SOURCE_REMOVE;
}

static gboolean cli_lua_event_subscribe_socket(CliLuaEventSubscription* subscription,
                                               GError** error) {
    static const char* const shortcut_owner_events[] = {
        "gnoblin.shortcut.binding-activated", "gnoblin.shortcut.binding-deactivated",
        "gnoblin.shortcut.session.activated", "gnoblin.shortcut.session.key",
        "gnoblin.shortcut.session.ended",
    };
    Cli* cli = subscription->cli;
    g_autofree char* request_id = g_uuid_string_random();
    g_autoptr(JsonBuilder) builder = json_builder_new();
    json_builder_begin_object(builder);
    json_builder_set_member_name(builder, "op");
    json_builder_add_string_value(builder, "events");
    json_builder_set_member_name(builder, "id");
    json_builder_add_string_value(builder, request_id);
    json_builder_set_member_name(builder, "api_version");
    json_builder_begin_object(builder);
    json_builder_set_member_name(builder, "major");
    json_builder_add_int_value(builder, GNOBLIN_NATIVE_CONTROL_API_MAJOR);
    json_builder_set_member_name(builder, "minor");
    json_builder_add_int_value(builder, GNOBLIN_NATIVE_CONTROL_API_MINOR);
    json_builder_end_object(builder);
    json_builder_set_member_name(builder, "events");
    json_builder_begin_array(builder);
    if (subscription->shortcut_owner_transport) {
        for (guint i = 0; i < G_N_ELEMENTS(shortcut_owner_events); i++)
            json_builder_add_string_value(builder, shortcut_owner_events[i]);
    } else {
        json_builder_add_string_value(builder, subscription->event_name);
    }
    json_builder_end_array(builder);
    json_builder_end_object(builder);
    g_autoptr(JsonNode) request = json_builder_get_root(builder);
    g_autofree char* encoded = json_to_string(request, FALSE);
    g_autofree char* payload = g_strconcat(encoded, "\n", NULL);

    g_autoptr(GSocketClient) client = g_socket_client_new();
    g_socket_client_set_timeout(client, cli->timeout);
    g_autoptr(GSocketAddress) address = g_unix_socket_address_new(cli->socket_path);
    subscription->connection =
        g_socket_client_connect(client, G_SOCKET_CONNECTABLE(address), NULL, error);
    if (!subscription->connection)
        return FALSE;
    GSocket* socket = g_socket_connection_get_socket(subscription->connection);
    g_socket_set_timeout(socket, cli->timeout);
    if (!g_output_stream_write_all(
            g_io_stream_get_output_stream(G_IO_STREAM(subscription->connection)), payload,
            strlen(payload), NULL, NULL, error))
        return FALSE;

    gboolean subscribed = FALSE;
    while (!subscribed) {
        char chunk[4096];
        gssize count = g_socket_receive(socket, chunk, sizeof chunk, NULL, error);
        if (count <= 0) {
            if (count == 0 && (!error || !*error))
                g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_CLOSED,
                                    "compositor closed the event socket before acknowledging");
            return FALSE;
        }
        g_string_append_len(subscription->pending, chunk, count);
        if (subscription->pending->len > 4 * 1024 * 1024) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NO_SPACE,
                                "event subscription response is too large");
            return FALSE;
        }
        char* line_end = memchr(subscription->pending->str, '\n', subscription->pending->len);
        if (!line_end)
            continue;
        g_autofree char* line =
            g_strndup(subscription->pending->str, line_end - subscription->pending->str);
        g_string_erase(subscription->pending, 0, line_end - subscription->pending->str + 1);
        g_autoptr(JsonParser) parser = json_parser_new();
        if (!json_parser_load_from_data(parser, line, -1, error) ||
            !JSON_NODE_HOLDS_OBJECT(json_parser_get_root(parser))) {
            if (!error || !*error)
                g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                                    "invalid event subscription acknowledgement");
            return FALSE;
        }
        JsonObject* response = json_node_get_object(json_parser_get_root(parser));
        const char* response_event = member_string(response, "event", "");
        if (g_str_equal(response_event, "hello")) {
            if (json_object_get_int_member_with_default(response, "api_major", 0) !=
                GNOBLIN_NATIVE_CONTROL_API_MAJOR) {
                g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_SUPPORTED,
                                    "compositor socket uses an unsupported API major version");
                return FALSE;
            }
            continue;
        }
        if (g_str_equal(response_event, "error")) {
            g_set_error(error, G_IO_ERROR, G_IO_ERROR_FAILED, "%s",
                        member_string(response, "message", "compositor rejected subscription"));
            return FALSE;
        }
        JsonArray* accepted = member_array(response, "events");
        gboolean accepted_matches =
            accepted &&
            json_array_get_length(accepted) ==
                (subscription->shortcut_owner_transport ? G_N_ELEMENTS(shortcut_owner_events) : 1);
        for (guint i = 0; accepted_matches && i < json_array_get_length(accepted); i++) {
            JsonNode* accepted_node = json_array_get_element(accepted, i);
            const char* accepted_name =
                accepted_node && JSON_NODE_HOLDS_VALUE(accepted_node) &&
                        json_node_get_value_type(accepted_node) == G_TYPE_STRING
                    ? json_node_get_string(accepted_node)
                    : NULL;
            const char* expected_name = subscription->shortcut_owner_transport
                                            ? shortcut_owner_events[i]
                                            : subscription->event_name;
            accepted_matches = accepted_name && g_str_equal(accepted_name, expected_name);
        }
        if (!g_str_equal(response_event, "subscribed") || !accepted_matches) {
            g_set_error_literal(
                error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                "compositor returned an invalid event subscription acknowledgement");
            return FALSE;
        }
        subscribed = TRUE;
    }

    g_socket_set_timeout(socket, 0);
    g_socket_set_blocking(socket, FALSE);
    subscription->active = TRUE;
    if (!subscription->shortcut_owner_transport)
        cli->active_lua_event_subscriptions++;
    subscription->source_id = g_unix_fd_add_full(G_PRIORITY_DEFAULT, g_socket_get_fd(socket),
                                                 G_IO_IN | G_IO_HUP | G_IO_ERR | G_IO_NVAL,
                                                 cli_lua_event_ready, subscription, NULL);
    if (memchr(subscription->pending->str, '\n', subscription->pending->len)) {
        subscription->pending_source_id = g_idle_add_full(
            G_PRIORITY_DEFAULT_IDLE, cli_lua_event_process_pending_idle, subscription, NULL);
    }
    return TRUE;
}

gboolean cli_lua_shortcut_owner_ensure(Cli* cli, GError** error) {
    if (cli->lua_shortcut_owner && cli->lua_shortcut_owner->active &&
        cli->lua_shortcut_owner->connection)
        return TRUE;
    if (!cli->lua_event_subscriptions) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_CLOSED,
                            "Lua event transport is unavailable");
        return FALSE;
    }

    CliLuaEventSubscription* owner = g_new0(CliLuaEventSubscription, 1);
    owner->cli = cli;
    owner->event_name = g_strdup("gnoblin.shortcut");
    owner->pending = g_string_new(NULL);
    owner->callback_ref = LUA_NOREF;
    owner->shortcut_owner_transport = TRUE;
    cli->lua_shortcut_owner = owner;
    if (!cli_lua_event_subscribe_socket(owner, error)) {
        cli->lua_shortcut_owner = NULL;
        cli_lua_event_subscription_free(owner);
        return FALSE;
    }
    g_ptr_array_add(cli->lua_event_subscriptions, owner);
    return TRUE;
}

static gboolean cli_lua_is_owned_shortcut_event(const char* event_name) {
    static const char* const names[] = {
        "gnoblin.shortcut.binding-activated", "gnoblin.shortcut.binding-deactivated",
        "gnoblin.shortcut.session.activated", "gnoblin.shortcut.session.key",
        "gnoblin.shortcut.session.ended",
    };
    for (guint i = 0; i < G_N_ELEMENTS(names); i++)
        if (g_str_equal(event_name, names[i]))
            return TRUE;
    return FALSE;
}

int lua_cli_events_subscribe(lua_State* state) {
    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    gboolean once = lua_toboolean(state, lua_upvalueindex(2));
    gboolean mutter_namespace = lua_toboolean(state, lua_upvalueindex(3));
    if (lua_gettop(state) != 2 || lua_type(state, 1) != LUA_TSTRING ||
        lua_type(state, 2) != LUA_TFUNCTION)
        return luaL_error(state, "gnoblin.events.%s requires an event name and callback",
                          once ? "once" : "on");
    size_t event_length = 0;
    const char* event_name = lua_tolstring(state, 1, &event_length);
    if (!event_name || event_length == 0 || event_length > 128 ||
        !g_utf8_validate(event_name, event_length, NULL))
        return luaL_error(state, "event name must be 1 to 128 bytes of valid UTF-8");
    if (mutter_namespace && !g_str_has_prefix(event_name, "mutter."))
        return luaL_error(state, "Mutter event name must start with 'mutter.'");

    CliLuaEventSubscription* subscription = g_new0(CliLuaEventSubscription, 1);
    subscription->cli = cli;
    subscription->event_name = g_strndup(event_name, event_length);
    subscription->pending = g_string_new(NULL);
    subscription->location_requests =
        g_hash_table_new_full(g_int64_hash, g_int64_equal, g_free, g_free);
    subscription->callback_ref = LUA_NOREF;
    subscription->once = once;
    lua_pushvalue(state, 2);
    subscription->callback_ref = luaL_ref(state, LUA_REGISTRYINDEX);

    g_autoptr(GError) error = NULL;
    if (cli_lua_is_owned_shortcut_event(event_name)) {
        if (!cli_lua_shortcut_owner_ensure(cli, &error)) {
            cli_lua_event_subscription_free(subscription);
            return luaL_error(state, "could not subscribe to %s: %s", event_name,
                              error ? error->message : "unknown error");
        }
        subscription->shortcut_owner = TRUE;
        subscription->connection = g_object_ref(cli->lua_shortcut_owner->connection);
        subscription->active = TRUE;
        cli->active_lua_event_subscriptions++;
        if (cli->lua_shortcut_owner->pending->len > 0 &&
            !cli->lua_shortcut_owner->pending_source_id)
            cli->lua_shortcut_owner->pending_source_id =
                g_idle_add_full(G_PRIORITY_DEFAULT_IDLE, cli_lua_event_process_pending_idle,
                                cli->lua_shortcut_owner, NULL);
    } else if (!cli_lua_event_subscribe_socket(subscription, &error)) {
        cli_lua_event_subscription_free(subscription);
        return luaL_error(state, "could not subscribe to %s: %s", event_name,
                          error ? error->message : "unknown error");
    }
    g_ptr_array_add(cli->lua_event_subscriptions, subscription);
    LuaCliEventSubscriptionHandle* handle = lua_newuserdatauv(state, sizeof *handle, 0);
    handle->subscription = subscription;
    luaL_getmetatable(state, GNOBLINCTL_EVENT_SUBSCRIPTION_METATABLE);
    lua_setmetatable(state, -2);
    return 1;
}

static int lua_cli_event_subscription_unsubscribe(lua_State* state) {
    LuaCliEventSubscriptionHandle* handle =
        luaL_checkudata(state, 1, GNOBLINCTL_EVENT_SUBSCRIPTION_METATABLE);
    if (lua_gettop(state) != 1)
        return luaL_error(state, "Subscription:unsubscribe takes no arguments");
    cli_lua_event_subscription_detach(handle->subscription);
    return 0;
}

static int lua_cli_event_subscription_index(lua_State* state) {
    luaL_checkudata(state, 1, GNOBLINCTL_EVENT_SUBSCRIPTION_METATABLE);
    const char* field = luaL_checkstring(state, 2);
    if (g_str_equal(field, "unsubscribe")) {
        lua_pushcfunction(state, lua_cli_event_subscription_unsubscribe);
        return 1;
    }
    return 0;
}

static int lua_cli_event_subscription_newindex(lua_State* state) {
    return luaL_error(state, "Subscription fields are read-only");
}

static int lua_cli_event_subscription_tostring(lua_State* state) {
    LuaCliEventSubscriptionHandle* handle =
        luaL_checkudata(state, 1, GNOBLINCTL_EVENT_SUBSCRIPTION_METATABLE);
    CliLuaEventSubscription* subscription = handle->subscription;
    lua_pushfstring(state, "Subscription<%s:%s>",
                    subscription && subscription->active ? "active" : "ended",
                    subscription ? subscription->event_name : "unknown");
    return 1;
}

static int lua_cli_focus_context_gc(lua_State* state) {
    LuaCliFocusContext* context = luaL_testudata(state, 1, GNOBLINCTL_FOCUS_CONTEXT_METATABLE);
    if (!context || lua_rawlen(state, 1) != sizeof(LuaCliFocusContext) ||
        context->magic != LUA_CLI_FOCUS_CONTEXT_MAGIC)
        return 0;
    g_clear_pointer(&context->token, g_free);
    context->subscription = NULL;
    context->magic = 0;
    return 0;
}

static int lua_cli_focus_context_newindex(lua_State* state) {
    return luaL_error(state, "FocusContext values are opaque and read-only");
}

static int lua_cli_focus_context_tostring(lua_State* state) {
    LuaCliFocusContext* context = luaL_testudata(state, 1, GNOBLINCTL_FOCUS_CONTEXT_METATABLE);
    if (!context || lua_rawlen(state, 1) != sizeof(LuaCliFocusContext) ||
        context->magic != LUA_CLI_FOCUS_CONTEXT_MAGIC) {
        lua_pushliteral(state, "FocusContext<invalid>");
        return 1;
    }
    lua_pushfstring(state, "FocusContext<%s>", context->consumed ? "consumed" : "available");
    return 1;
}

void register_lua_cli_focus_context(lua_State* state) {
    if (!luaL_newmetatable(state, GNOBLINCTL_FOCUS_CONTEXT_METATABLE)) {
        lua_pop(state, 1);
        return;
    }
    lua_pushcfunction(state, lua_cli_focus_context_gc);
    lua_setfield(state, -2, "__gc");
    lua_pushcfunction(state, lua_cli_focus_context_newindex);
    lua_setfield(state, -2, "__newindex");
    lua_pushcfunction(state, lua_cli_focus_context_tostring);
    lua_setfield(state, -2, "__tostring");
    lua_pop(state, 1);
}

static int lua_cli_menu_context_index(lua_State* state) {
    LuaCliMenuContext* context = lua_cli_test_menu_context(state, 1);
    if (!context)
        return 0;
    const char* field = luaL_checkstring(state, 2);
    if (g_str_equal(field, "begin_move")) {
        lua_pushcfunction(state, lua_cli_menu_context_begin_move);
        return 1;
    }
    if (g_str_equal(field, "begin_resize")) {
        lua_pushcfunction(state, lua_cli_menu_context_begin_resize);
        return 1;
    }
    return 0;
}

static int lua_cli_menu_context_gc(lua_State* state) {
    LuaCliMenuContext* context = luaL_testudata(state, 1, GNOBLINCTL_MENU_CONTEXT_METATABLE);
    if (!context || lua_rawlen(state, 1) != sizeof(LuaCliMenuContext) ||
        context->magic != LUA_CLI_MENU_CONTEXT_MAGIC)
        return 0;
    g_clear_pointer(&context->token, g_free);
    context->subscription = NULL;
    context->magic = 0;
    return 0;
}

static int lua_cli_menu_context_newindex(lua_State* state) {
    return luaL_error(state, "MenuContext values are opaque and read-only");
}

static int lua_cli_menu_context_tostring(lua_State* state) {
    LuaCliMenuContext* context = luaL_testudata(state, 1, GNOBLINCTL_MENU_CONTEXT_METATABLE);
    if (!context || lua_rawlen(state, 1) != sizeof(LuaCliMenuContext) ||
        context->magic != LUA_CLI_MENU_CONTEXT_MAGIC) {
        lua_pushliteral(state, "MenuContext<invalid>");
        return 1;
    }
    lua_pushfstring(state, "MenuContext<%s>", context->consumed ? "consumed" : "available");
    return 1;
}

void register_lua_cli_menu_context(lua_State* state) {
    if (!luaL_newmetatable(state, GNOBLINCTL_MENU_CONTEXT_METATABLE)) {
        lua_pop(state, 1);
        return;
    }
    lua_pushcfunction(state, lua_cli_menu_context_index);
    lua_setfield(state, -2, "__index");
    lua_pushcfunction(state, lua_cli_menu_context_gc);
    lua_setfield(state, -2, "__gc");
    lua_pushcfunction(state, lua_cli_menu_context_newindex);
    lua_setfield(state, -2, "__newindex");
    lua_pushcfunction(state, lua_cli_menu_context_tostring);
    lua_setfield(state, -2, "__tostring");
    lua_pop(state, 1);
}

void register_lua_cli_snap_context(lua_State* state) {
    if (!luaL_newmetatable(state, GNOBLINCTL_SNAP_CONTEXT_METATABLE)) {
        lua_pop(state, 1);
        return;
    }
    lua_pushcfunction(state, lua_cli_window_index);
    lua_setfield(state, -2, "__index");
    lua_pushcfunction(state, lua_cli_snap_context_newindex);
    lua_setfield(state, -2, "__newindex");
    lua_pushcfunction(state, lua_cli_window_len);
    lua_setfield(state, -2, "__len");
    lua_pushcfunction(state, lua_cli_window_pairs);
    lua_setfield(state, -2, "__pairs");
    lua_pushcfunction(state, lua_cli_snap_context_gc);
    lua_setfield(state, -2, "__gc");
    lua_pushcfunction(state, lua_cli_snap_context_tostring);
    lua_setfield(state, -2, "__tostring");
    lua_pushliteral(state, "SnapContext");
    lua_setfield(state, -2, "__metatable");
    lua_pop(state, 1);
}

void register_lua_cli_text_target(lua_State* state) {
    if (!luaL_newmetatable(state, GNOBLINCTL_TEXT_TARGET_METATABLE)) {
        lua_pop(state, 1);
        return;
    }
    lua_pushcfunction(state, lua_cli_window_index);
    lua_setfield(state, -2, "__index");
    lua_pushcfunction(state, lua_cli_text_target_newindex);
    lua_setfield(state, -2, "__newindex");
    lua_pushcfunction(state, lua_cli_window_len);
    lua_setfield(state, -2, "__len");
    lua_pushcfunction(state, lua_cli_window_pairs);
    lua_setfield(state, -2, "__pairs");
    lua_pushcfunction(state, lua_cli_text_target_gc);
    lua_setfield(state, -2, "__gc");
    lua_pushcfunction(state, lua_cli_text_target_tostring);
    lua_setfield(state, -2, "__tostring");
    lua_pushliteral(state, "TextTarget");
    lua_setfield(state, -2, "__metatable");
    lua_pop(state, 1);
}

void register_lua_cli_event_subscription(lua_State* state) {
    if (!luaL_newmetatable(state, GNOBLINCTL_EVENT_SUBSCRIPTION_METATABLE)) {
        lua_pop(state, 1);
        return;
    }
    lua_pushcfunction(state, lua_cli_event_subscription_index);
    lua_setfield(state, -2, "__index");
    lua_pushcfunction(state, lua_cli_event_subscription_newindex);
    lua_setfield(state, -2, "__newindex");
    lua_pushcfunction(state, lua_cli_event_subscription_tostring);
    lua_setfield(state, -2, "__tostring");
    lua_pop(state, 1);
}

gboolean cli_lua_event_loop(Cli* cli) {
    while ((cli->active_lua_event_subscriptions > 0 ||
            (cli->lua_shortcut_bindings && g_hash_table_size(cli->lua_shortcut_bindings) > 0)) &&
           !cli->lua_event_failed)
        g_main_loop_run(cli->lua_event_loop);
    return !cli->lua_event_failed;
}

void cli_lua_events_clear(Cli* cli) {
    if (!cli->lua_event_subscriptions)
        return;
    g_ptr_array_unref(cli->lua_event_subscriptions);
    cli->lua_event_subscriptions = NULL;
    cli->active_lua_event_subscriptions = 0;
    cli->lua_shortcut_owner = NULL;
    g_clear_pointer(&cli->lua_shortcut_bindings, g_hash_table_unref);
    g_clear_pointer(&cli->lua_event_loop, g_main_loop_unref);
}
