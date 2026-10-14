#include "gnoblin-lua-extension.h"

#include <glib.h>

static GPtrArray* installers;

void gnoblin_lua_extension_register(GnoblinLuaExtensionInstallFunc install) {
    g_return_if_fail(install != NULL);
    if (!installers)
        installers = g_ptr_array_new();
    for (guint i = 0; i < installers->len; i++)
        if (g_ptr_array_index(installers, i) == install)
            return;
    g_ptr_array_add(installers, install);
}

void gnoblin_lua_extensions_install(lua_State* state, void* config) {
    /* Statically linked extensions register here. The VR extension supplies this hook
     * only when its sources are linked into the same binaries; a weak reference keeps
     * builds that omit the extension linking cleanly. */
    extern void gnoblin_lua_extension_vr_install(lua_State * state, void* config)
        __attribute__((weak));
    if (gnoblin_lua_extension_vr_install)
        gnoblin_lua_extension_vr_install(state, config);
    if (!installers)
        return;
    for (guint i = 0; i < installers->len; i++) {
        GnoblinLuaExtensionInstallFunc install = g_ptr_array_index(installers, i);
        install(state, config);
    }
}
