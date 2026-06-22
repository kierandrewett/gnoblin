/*
 * gnoblin-shell: input configuration -> org.gnome.desktop GSettings bridge.
 * See gnoblin-input.h.
 *
 * This program is free software; you can redistribute it and/or modify it
 * under the terms of the GNU General Public License as published by the Free
 * Software Foundation; either version 2 of the License, or (at your option)
 * any later version.
 */

#include "gnoblin-input.h"

#include <gio/gio.h>
#include <stdlib.h>
#include <string.h>

#include "gnoblin-config.h"
#include "gnoblin-input-spec.h"

#define INPUT "input"

/* g_settings_new() aborts if the schema is not installed, so look it up first
 * and return NULL (skip) when the desktop schemas are missing. */
static GSettings* settings_for(const char* schema_id) {
    GSettingsSchemaSource* source = g_settings_schema_source_get_default();
    g_autoptr(GSettingsSchema) schema = NULL;

    if (!source)
        return NULL;
    schema = g_settings_schema_source_lookup(source, schema_id, TRUE);
    if (!schema)
        return NULL;
    return g_settings_new_full(schema, NULL, NULL);
}

/* Whether `[input] key` is present in the config at all (so we only touch the
 * GSettings the user explicitly asked for). */
static gboolean has(const char* key) {
    g_autofree char* v = gnoblin_config_get_string(INPUT, key);
    return v != NULL;
}

/* Validate `value` against an enum-typed key's allowed nicks before writing
 * (g_settings_set_string would warn + no-op on a bad nick). */
static void set_enum_string(GSettings* s, const char* key, const char* value) {
    g_autoptr(GSettingsSchema) schema = NULL;
    g_autoptr(GSettingsSchemaKey) sk = NULL;
    g_autoptr(GVariant) probe = NULL;

    g_object_get(s, "settings-schema", &schema, NULL);
    if (!schema || !g_settings_schema_has_key(schema, key))
        return;
    sk = g_settings_schema_get_key(schema, key);
    probe = g_variant_new_string(value);
    if (g_settings_schema_key_range_check(sk, probe))
        g_settings_set_string(s, key, value);
    else
        g_warning("gnoblin: [input] '%s' = '%s' is not a valid value", key, value);
}

void gnoblin_input_apply(void) {
    g_autoptr(GSettings) keyboard = settings_for("org.gnome.desktop.peripherals.keyboard");
    g_autoptr(GSettings) mouse = settings_for("org.gnome.desktop.peripherals.mouse");
    g_autoptr(GSettings) touchpad = settings_for("org.gnome.desktop.peripherals.touchpad");
    g_autoptr(GSettings) sources = settings_for("org.gnome.desktop.input-sources");
    g_autoptr(GSettings) wm = settings_for("org.gnome.desktop.wm.preferences");

    /* --- keyboard layout (xkb) --- */
    if (sources && has("keyboard-layout")) {
        g_autofree char* layout = gnoblin_config_get_string(INPUT, "keyboard-layout");
        g_autofree char* variant = gnoblin_config_get_string(INPUT, "keyboard-variant");
        g_autofree char* id = (variant && variant[0])
                                  ? g_strdup_printf("%s+%s", layout, variant)
                                  : g_strdup(layout);
        GVariantBuilder b;

        g_variant_builder_init(&b, G_VARIANT_TYPE("a(ss)"));
        g_variant_builder_add(&b, "(ss)", "xkb", id);
        g_settings_set_value(sources, "sources", g_variant_builder_end(&b));
    }
    if (sources && has("keyboard-options")) {
        g_auto(GStrv) opts = gnoblin_config_get_list(INPUT, "keyboard-options");
        g_settings_set_strv(sources, "xkb-options", (const char* const*)opts);
    }

    /* --- keyboard repeat --- */
    if (keyboard && has("repeat-rate")) {
        int rate = gnoblin_config_get_int(INPUT, "repeat-rate", 25);
        g_settings_set_boolean(keyboard, "repeat", rate > 0);
        if (rate > 0)
            g_settings_set_uint(keyboard, "repeat-interval", (guint)(1000 / rate));
    }
    if (keyboard && has("repeat-delay"))
        g_settings_set_uint(keyboard, "delay",
                            (guint)gnoblin_config_get_int(INPUT, "repeat-delay", 600));

    /* --- pointer / touchpad --- */
    if (has("natural-scroll")) {
        gboolean v = gnoblin_config_get_bool(INPUT, "natural-scroll", FALSE);
        if (touchpad)
            g_settings_set_boolean(touchpad, "natural-scroll", v);
        if (mouse)
            g_settings_set_boolean(mouse, "natural-scroll", v);
    }
    if (touchpad && has("tap-to-click"))
        g_settings_set_boolean(touchpad, "tap-to-click",
                               gnoblin_config_get_bool(INPUT, "tap-to-click", TRUE));
    if (touchpad && has("disable-while-typing"))
        g_settings_set_boolean(touchpad, "disable-while-typing",
                               gnoblin_config_get_bool(INPUT, "disable-while-typing", TRUE));
    if (has("accel-profile")) {
        g_autofree char* p = gnoblin_config_get_string(INPUT, "accel-profile");
        if (touchpad)
            set_enum_string(touchpad, "accel-profile", p);
        if (mouse)
            set_enum_string(mouse, "accel-profile", p);
    }
    if (has("pointer-speed")) {
        g_autofree char* s = gnoblin_config_get_string(INPUT, "pointer-speed");
        double v;

        if (gnoblin_input_parse_pointer_speed(s, &v)) {
            if (touchpad)
                g_settings_set_double(touchpad, "speed", v);
            if (mouse)
                g_settings_set_double(mouse, "speed", v);
        } else {
            g_warning("gnoblin: [input] pointer-speed = '%s' is not a valid number", s);
        }
    }

    /* --- focus-follows-mouse --- */
    if (wm && has("focus-follows-mouse")) {
        gboolean ffm = gnoblin_config_get_bool(INPUT, "focus-follows-mouse", FALSE);
        g_settings_set_string(wm, "focus-mode", ffm ? "sloppy" : "click");
    }

    /* Read the touched keys back and log them: mutter's input backend reads the
     * very same (in-process, with the memory backend) GSettings, so this both
     * confirms the writes landed and gives the headless test something to grep. */
    {
        g_autoptr(GString) summary = g_string_new("gnoblin-input: applied");

        if (sources && has("keyboard-layout")) {
            g_autoptr(GVariant) v = g_settings_get_value(sources, "sources");
            const char *t = NULL, *id = NULL;

            if (g_variant_n_children(v) > 0)
                g_variant_get_child(v, 0, "(&s&s)", &t, &id);
            g_string_append_printf(summary, " layout=%s", id ? id : "?");
        }
        if (keyboard && has("repeat-rate"))
            g_string_append_printf(summary, " repeat-interval=%u",
                                   g_settings_get_uint(keyboard, "repeat-interval"));
        if (keyboard && has("repeat-delay"))
            g_string_append_printf(summary, " delay=%u", g_settings_get_uint(keyboard, "delay"));
        if (touchpad && has("natural-scroll"))
            g_string_append_printf(summary, " natural-scroll=%d",
                                   g_settings_get_boolean(touchpad, "natural-scroll"));
        if (touchpad && has("tap-to-click"))
            g_string_append_printf(summary, " tap-to-click=%d",
                                   g_settings_get_boolean(touchpad, "tap-to-click"));
        if (touchpad && has("accel-profile")) {
            g_autofree char* p = g_settings_get_string(touchpad, "accel-profile");
            g_string_append_printf(summary, " accel-profile=%s", p);
        }
        if (wm && has("focus-follows-mouse")) {
            g_autofree char* m = g_settings_get_string(wm, "focus-mode");
            g_string_append_printf(summary, " focus-mode=%s", m);
        }
        g_message("%s", summary->str);
    }
}
