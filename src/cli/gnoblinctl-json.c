#include "gnoblinctl-internal.h"

JsonNode* lua_to_json(lua_State* state, int index, guint depth, GError** error) {
    if (depth > 64) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "Lua values may be nested at most 64 levels");
        return NULL;
    }
    index = lua_absindex(state, index);
    switch (lua_type(state, index)) {
    case LUA_TNIL: {
        return json_node_new(JSON_NODE_NULL);
    }
    case LUA_TBOOLEAN: {
        JsonNode* node = json_node_new(JSON_NODE_VALUE);
        json_node_set_boolean(node, lua_toboolean(state, index));
        return node;
    }
    case LUA_TNUMBER: {
        if (lua_isinteger(state, index)) {
            JsonNode* node = json_node_new(JSON_NODE_VALUE);
            json_node_set_int(node, lua_tointeger(state, index));
            return node;
        }
        lua_Number number = lua_tonumber(state, index);
        if (!isfinite(number)) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                                "Lua numbers must be finite");
            return NULL;
        }
        JsonNode* node = json_node_new(JSON_NODE_VALUE);
        json_node_set_double(node, number);
        return node;
    }
    case LUA_TSTRING: {
        size_t length = 0;
        const char* value = lua_tolstring(state, index, &length);
        if (!g_utf8_validate(value, length, NULL) || memchr(value, '\0', length)) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                                "Lua strings must be valid UTF-8 without NUL bytes");
            return NULL;
        }
        JsonNode* node = json_node_new(JSON_NODE_VALUE);
        json_node_set_string(node, value);
        return node;
    }
    case LUA_TTABLE: {
        guint array_length = lua_rawlen(state, index);
        guint entries = 0;
        gboolean array = array_length > 0;
        lua_pushnil(state);
        while (lua_next(state, index)) {
            entries++;
            if (!lua_isinteger(state, -2) || lua_tointeger(state, -2) < 1 ||
                lua_tointeger(state, -2) > array_length)
                array = FALSE;
            lua_pop(state, 1);
        }
        if (array && entries != array_length)
            array = FALSE;
        if (array) {
            JsonArray* values = json_array_new();
            for (guint i = 1; i <= array_length; i++) {
                lua_rawgeti(state, index, i);
                JsonNode* value = lua_to_json(state, -1, depth + 1, error);
                lua_pop(state, 1);
                if (!value) {
                    json_array_unref(values);
                    return NULL;
                }
                json_array_add_element(values, value);
            }
            JsonNode* node = json_node_new(JSON_NODE_ARRAY);
            json_node_take_array(node, values);
            return node;
        }

        JsonObject* object = json_object_new();
        lua_pushnil(state);
        while (lua_next(state, index)) {
            if (lua_type(state, -2) != LUA_TSTRING) {
                lua_pop(state, 2);
                json_object_unref(object);
                g_set_error_literal(
                    error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                    "Lua object keys must be strings; array keys must be dense integers");
                return NULL;
            }
            const char* key = lua_tostring(state, -2);
            JsonNode* value = lua_to_json(state, -1, depth + 1, error);
            if (!value) {
                lua_pop(state, 2);
                json_object_unref(object);
                return NULL;
            }
            json_object_set_member(object, key, value);
            lua_pop(state, 1);
        }
        JsonNode* node = json_node_new(JSON_NODE_OBJECT);
        json_node_take_object(node, object);
        return node;
    }
    case LUA_TUSERDATA: {
        if (!luaL_testudata(state, index, GNOBLINCTL_READONLY_TABLE_METATABLE))
            break;
        lua_getiuservalue(state, index, 1);
        JsonNode* node = lua_to_json(state, -1, depth + 1, error);
        lua_pop(state, 1);
        return node;
    }
    default:
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "Lua API arguments must contain only JSON values");
        return NULL;
    }
}

void json_to_lua(lua_State* state, JsonNode* node) {
    if (!node || JSON_NODE_HOLDS_NULL(node)) {
        lua_pushnil(state);
    } else if (JSON_NODE_HOLDS_OBJECT(node)) {
        JsonObject* object = json_node_get_object(node);
        lua_createtable(state, 0, json_object_get_size(object));
        GList* members = json_object_get_members(object);
        for (GList* item = members; item; item = item->next) {
            const char* key = item->data;
            json_to_lua(state, json_object_get_member(object, key));
            lua_setfield(state, -2, key);
        }
        g_list_free(members);
    } else if (JSON_NODE_HOLDS_ARRAY(node)) {
        JsonArray* array = json_node_get_array(node);
        guint length = json_array_get_length(array);
        lua_createtable(state, length, 0);
        for (guint i = 0; i < length; i++) {
            json_to_lua(state, json_array_get_element(array, i));
            lua_rawseti(state, -2, i + 1);
        }
    } else {
        GType type = json_node_get_value_type(node);
        if (type == G_TYPE_BOOLEAN)
            lua_pushboolean(state, json_node_get_boolean(node));
        else if (type == G_TYPE_INT || type == G_TYPE_INT64)
            lua_pushinteger(state, json_node_get_int(node));
        else if (type == G_TYPE_DOUBLE)
            lua_pushnumber(state, json_node_get_double(node));
        else if (type == G_TYPE_STRING)
            lua_pushstring(state, json_node_get_string(node));
        else
            lua_pushnil(state);
    }
}
