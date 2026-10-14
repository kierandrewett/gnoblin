#pragma once

#include <lua.h>

/* Optional statically linked extensions may add a private namespace beneath
 * gnoblin. They do not add stock API names or core Lua record types. */
typedef void (*GnoblinLuaExtensionInstallFunc)(lua_State* state, void* config);

void gnoblin_lua_extension_register(GnoblinLuaExtensionInstallFunc install);
void gnoblin_lua_extensions_install(lua_State* state, void* config);
