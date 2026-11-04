/* The runtime overrides the Lua API offers, shared by the Lua runtime and the compositor.
 *
 * `gnoblin.settings.override(path, value)` layers a value over one committed setting and
 * `Window:set_effect(name, value)` layers an effect over one window. Both sides need the same
 * answer to "can this change at runtime?", so the list lives here, in a header with static inline
 * functions only. The Lua runtime includes it from src/config/lua; the control manifest copies it
 * into Mutter as core/gnoblin-overrides.h.
 *
 * Section and key names are the document's (kebab case, "window-management"). The public Lua
 * names are the same with underscores. */
#pragma once

#include <glib.h>
#include <string.h>

#define GNOBLIN_OVERRIDE_SETTING_LIST                                                              \
    "window_management.*, compositor.*, cursor.theme, cursor.size, "                               \
    "layer_shell.preserve_active_window, monitors.privacy_screen, input.orientation_lock"

#define GNOBLIN_OVERRIDE_EFFECT_LIST                                                               \
    "opacity, blur, blur_ignore_shadows, shader, shader_uniforms, frame, corners"

/* Convert a public name ("window_management") to the document's ("window-management"). The
 * caller frees the result. */
static inline char* gnoblin_override_document_name(const char* name) {
    char* copy = g_strdup(name);
    return g_strdelimit(copy, "_", '-');
}

/* NULL when `section` and `key` name a setting that can change at runtime. Otherwise the reason
 * it cannot, for an error message; the caller frees it. A key that only exists in the section the
 * list allows as a whole is not checked here: the configuration validator checks it. */
static inline char* gnoblin_override_setting_refusal(const char* section, const char* key) {
    static const struct {
        const char* section;
        const char* key; /* NULL: every key the validator accepts */
    } allowed[] = {
        {"window-management", NULL},
        {"compositor", NULL},
        {"cursor", "theme"},
        {"cursor", "size"},
        {"layer-shell", "preserve-active-window"},
        {"monitors", "privacy-screen"},
        {"input", "orientation-lock"},
    };
    gboolean known_section = FALSE;
    for (gsize i = 0; i < G_N_ELEMENTS(allowed); i++) {
        if (!g_str_equal(allowed[i].section, section))
            continue;
        known_section = TRUE;
        if (!allowed[i].key || g_str_equal(allowed[i].key, key))
            return NULL;
    }
    if (known_section)
        return g_strdup_printf("only %s can change in the %s section; other keys are read when "
                               "the configuration loads",
                               g_str_equal(section, "cursor")  ? "cursor.theme and cursor.size"
                               : g_str_equal(section, "input") ? "input.orientation_lock"
                               : g_str_equal(section, "monitors")
                                   ? "monitors.privacy_screen"
                                   : "layer_shell.preserve_active_window",
                               section);
    static const struct {
        const char* section;
        const char* reason;
    } refused[] = {
        {"xwayland", "Xwayland settings need an Xwayland restart"},
        {"shortcuts", "shortcuts are registered when the configuration loads; use "
                      "gnoblin.shortcuts.bind for a shortcut that lasts for this runtime"},
        {"keybindings", "keybindings are registered when the configuration loads"},
        {"touchpad-gestures", "gestures are registered when the configuration loads"},
        {"window-rules", "window rules are matched when a window appears; use Window:set_effect "
                         "for the effects of one window"},
        {"workspaces", "the workspace list is fixed when the configuration loads; use "
                       "gnoblin.workspaces.create"},
        {"autostart", "autostart entries run once, when the session starts"},
        {"protocols", "protocols are advertised when a client connects after a reload"},
        {"frame-renderers", "frame renderers are loaded with the configuration"},
        {"animations", "animations are declared when the configuration loads"},
    };
    for (gsize i = 0; i < G_N_ELEMENTS(refused); i++)
        if (g_str_equal(refused[i].section, section))
            return g_strdup(refused[i].reason);
    return g_strdup_printf("the %s section is read only when the configuration loads", section);
}

/* Whether `name` (public, underscore form) is a window effect that window rules support. */
static inline gboolean gnoblin_override_effect_known(const char* name) {
    static const char* const effects[] = {
        "opacity", "blur", "blur_ignore_shadows", "shader", "shader_uniforms", "frame",
        "corners", NULL};
    for (gsize i = 0; effects[i]; i++)
        if (g_str_equal(effects[i], name))
            return TRUE;
    return FALSE;
}
