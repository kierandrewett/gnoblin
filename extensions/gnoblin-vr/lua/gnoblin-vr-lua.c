#include "gnoblin-lua-internal.h"

#include <math.h>
#include <string.h>

typedef struct {
    const char* name;
    const char* method;
    const char* const* fields;
    gboolean no_arguments;
} SpatialMethod;

static const char* const anchor_fields[] = {"id", "pose", "space_id", NULL};
static const char* const place_fields[] = {"surface_id", "anchor_id", "pose", "size", NULL};
static const char* const attach_fields[] = {"surface_id", "viewport_id", "pose", "size", NULL};
static const char* const move_fields[] = {"surface_id", "hit", "pose", NULL};
static const char* const surface_fields[] = {"surface_id", NULL};
static const char* const raise_fields[] = {"surface_id", NULL};
static const char* const pose_fields[] = {"surface_id", "pose", NULL};
static const char* const hit_fields[] = {"origin", "direction", NULL};
static const char* const head_fields[] = {"pose", NULL};
static const char* const point_fields[] = {"origin", "direction", "button", "state", NULL};

static gboolean spatial_string_field(lua_State* state, int table, const char* name,
                                     gboolean optional);

static gboolean spatial_number_field(lua_State* state, int table, const char* name,
                                     gboolean positive) {
    lua_getfield(state, table, name);
    gboolean valid = lua_type(state, -1) == LUA_TNUMBER;
    if (valid) {
        lua_Number value = lua_tonumber(state, -1);
        valid = isfinite(value) && (!positive || value > 0.0);
    }
    lua_pop(state, 1);
    return valid;
}

static gboolean spatial_vector(lua_State* state, int table, const char* name,
                               const char* const* fields, guint count) {
    lua_getfield(state, table, name);
    gboolean valid = lua_istable(state, -1);
    int vector = lua_absindex(state, -1);
    if (valid) {
        for (guint i = 0; i < count; i++)
            valid &= spatial_number_field(state, vector, fields[i], FALSE);
    }
    lua_pop(state, 1);
    return valid;
}

static gboolean spatial_pose(lua_State* state, int table) {
    static const char* const position[] = {"x", "y", "z"};
    static const char* const rotation[] = {"x", "y", "z", "w"};
    lua_getfield(state, table, "pose");
    gboolean valid = lua_istable(state, -1);
    int pose = lua_absindex(state, -1);
    if (valid) {
        valid = spatial_vector(state, pose, "position", position, G_N_ELEMENTS(position)) &&
                spatial_vector(state, pose, "rotation", rotation, G_N_ELEMENTS(rotation));
        if (valid) {
            lua_getfield(state, pose, "rotation");
            lua_getfield(state, -1, "x");
            lua_Number x = lua_tonumber(state, -1);
            lua_pop(state, 1);
            lua_getfield(state, -1, "y");
            lua_Number y = lua_tonumber(state, -1);
            lua_pop(state, 1);
            lua_getfield(state, -1, "z");
            lua_Number z = lua_tonumber(state, -1);
            lua_pop(state, 1);
            lua_getfield(state, -1, "w");
            lua_Number w = lua_tonumber(state, -1);
            lua_pop(state, 1);
            lua_pop(state, 1);
            lua_Number norm = x * x + y * y + z * z + w * w;
            valid = fabs(norm - 1.0) < 0.01;
        }
    }
    lua_pop(state, 1);
    return valid;
}

static gboolean spatial_size(lua_State* state, int table) {
    lua_getfield(state, table, "size");
    gboolean valid = lua_istable(state, -1);
    if (valid)
        valid = spatial_number_field(state, -1, "width", TRUE) &&
                spatial_number_field(state, -1, "height", TRUE);
    lua_pop(state, 1);
    return valid;
}

static gboolean spatial_hit(lua_State* state, int table) {
    lua_getfield(state, table, "hit");
    gboolean valid = lua_istable(state, -1);
    int hit = lua_absindex(state, -1);
    if (valid)
        valid = spatial_string_field(state, hit, "surface_id", FALSE) &&
                spatial_number_field(state, hit, "x", FALSE) &&
                spatial_number_field(state, hit, "y", FALSE) &&
                spatial_number_field(state, hit, "distance", TRUE);
    lua_pop(state, 1);
    return valid;
}

static const SpatialMethod spatial_methods[] = {
    {"snapshot", "vr.spatial.snapshot", NULL, TRUE},
    {"anchor", "vr.spatial.anchor", anchor_fields, FALSE},
    {"place", "vr.spatial.place", place_fields, FALSE},
    {"attach", "vr.spatial.attach", attach_fields, FALSE},
    {"surface", "vr.spatial.surface", surface_fields, FALSE},
    {"move", "vr.spatial.move", move_fields, FALSE},
    {"raise", "vr.spatial.raise", raise_fields, FALSE},
    {"update_pose", "vr.spatial.update_pose", pose_fields, FALSE},
    {"hit", "vr.spatial.hit", hit_fields, FALSE},
    {"head", "vr.spatial.head", head_fields, FALSE},
    {"point", "vr.spatial.point", point_fields, FALSE},
};

static gboolean spatial_string_field(lua_State* state, int table, const char* name,
                                     gboolean optional) {
    lua_getfield(state, table, name);
    gboolean valid = optional && lua_isnil(state, -1);
    if (lua_type(state, -1) == LUA_TSTRING) {
        size_t length = 0;
        const char* value = lua_tolstring(state, -1, &length);
        valid = length > 0 && length <= 128 && g_utf8_validate(value, length, NULL);
    }
    lua_pop(state, 1);
    return valid;
}

static int spatial_method_call(lua_State* state) {
    const SpatialMethod* method = lua_touserdata(state, lua_upvalueindex(1));
    LuaConfig* config = lua_touserdata(state, lua_upvalueindex(2));
    if (method->no_arguments) {
        if (lua_gettop(state) != 0)
            return luaL_error(state, "gnoblin.spatial.%s takes no arguments", method->name);
    } else {
        if (lua_gettop(state) != 1 || !lua_istable(state, 1) ||
            !table_fields_allowed(state, 1, method->fields))
            return luaL_error(state, "gnoblin.spatial.%s requires a supported argument table",
                              method->name);
        const char* required[] = {"surface_id", "anchor_id", "viewport_id", NULL};
        for (guint i = 0; required[i]; i++) {
            gboolean used = FALSE;
            for (guint j = 0; method->fields[j]; j++)
                used |= g_str_equal(required[i], method->fields[j]);
            if (used &&
                !spatial_string_field(state, 1, required[i], g_str_equal(required[i], "anchor_id")))
                return luaL_error(state, "gnoblin.spatial.%s requires a valid %s", method->name,
                                  required[i]);
        }
        if (g_str_equal(method->name, "anchor") &&
            (!spatial_string_field(state, 1, "id", FALSE) || !spatial_pose(state, 1)))
            return luaL_error(state, "gnoblin.spatial.anchor requires a valid id and pose");
        if (g_str_equal(method->name, "place") &&
            !spatial_string_field(state, 1, "anchor_id", FALSE))
            return luaL_error(state, "gnoblin.spatial.place requires an anchor_id");
        if (g_str_equal(method->name, "surface") &&
            !spatial_string_field(state, 1, "surface_id", FALSE))
            return luaL_error(state, "gnoblin.spatial.surface requires a valid surface_id");
        if ((g_str_equal(method->name, "place") || g_str_equal(method->name, "attach") ||
             g_str_equal(method->name, "move") || g_str_equal(method->name, "update_pose") ||
             g_str_equal(method->name, "head")) &&
            !spatial_pose(state, 1))
            return luaL_error(state,
                              "gnoblin.spatial.%s requires a finite pose with a "
                              "normalized quaternion",
                              method->name);
        if ((g_str_equal(method->name, "place") || g_str_equal(method->name, "attach"))) {
            if (!spatial_size(state, 1))
                return luaL_error(state,
                                  "gnoblin.spatial.%s requires positive finite size "
                                  "dimensions",
                                  method->name);
        }
        if (g_str_equal(method->name, "point")) {
            lua_getfield(state, 1, "button");
            lua_getfield(state, 1, "state");
            const char* button = lua_isstring(state, -2) ? lua_tostring(state, -2) : NULL;
            const char* press = lua_isstring(state, -1) ? lua_tostring(state, -1) : NULL;
            gboolean valid = (!button && !press && lua_isnil(state, -2) && lua_isnil(state, -1)) ||
                             (button && press &&
                              (g_str_equal(button, "left") || g_str_equal(button, "right") ||
                               g_str_equal(button, "middle")) &&
                              (g_str_equal(press, "press") || g_str_equal(press, "release")));
            lua_pop(state, 2);
            if (!valid)
                return luaL_error(state, "gnoblin.spatial.point takes button (left, right or "
                                         "middle) together with state (press or release)");
        }
        if (g_str_equal(method->name, "hit") || g_str_equal(method->name, "point")) {
            static const char* const axes[] = {"x", "y", "z"};
            if (!spatial_vector(state, 1, "origin", axes, G_N_ELEMENTS(axes)) ||
                !spatial_vector(state, 1, "direction", axes, G_N_ELEMENTS(axes)))
                return luaL_error(state, "gnoblin.spatial.hit requires finite origin and "
                                         "direction vectors with x, y, z");
        }
        if (g_str_equal(method->name, "move") && !spatial_hit(state, 1))
            return luaL_error(state, "gnoblin.spatial.move requires a valid hit record");
    }

    lua_pushlightuserdata(state, config);
    lua_pushstring(state, method->method);
    lua_pushcclosure(state, lua_generic_api_action, 2);
    lua_insert(state, 1);
    lua_call(state, lua_gettop(state) - 1, 1);
    return 1;
}

static void gnoblin_vr_lua_install(lua_State* state, void* opaque_config) {
    LuaConfig* config = opaque_config;
    lua_getglobal(state, "gnoblin");
    if (!lua_istable(state, -1)) {
        lua_pop(state, 1);
        return;
    }
    lua_newtable(state);
    for (guint i = 0; i < G_N_ELEMENTS(spatial_methods); i++) {
        lua_pushlightuserdata(state, (gpointer)&spatial_methods[i]);
        lua_pushlightuserdata(state, config);
        lua_pushcclosure(state, spatial_method_call, 2);
        lua_setfield(state, -2, spatial_methods[i].name);
    }
    lua_setfield(state, -2, "spatial");
    lua_pop(state, 1);
}

/* The configuration core calls this through a weak symbol, so stock code links without this
 * extension's sources. */
void gnoblin_lua_extension_vr_install(lua_State* state, void* config) {
    gnoblin_lua_extension_register(gnoblin_vr_lua_install);
}
