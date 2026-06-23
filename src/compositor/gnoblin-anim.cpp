/*
 * gnoblin-shell: config-driven animation settings.
 *
 * This program is free software; you can redistribute it and/or modify it
 * under the terms of the GNU General Public License as published by the Free
 * Software Foundation; either version 2 of the License, or (at your option)
 * any later version.
 */

#include "gnoblin-anim.h"

#include <math.h>
#include <string.h>

extern "C" {
#include <meta/prefs.h>
#include <meta/window.h>
}

#include "gnoblin-config.h"
#include "gnoblin-anim-spec.h"

static const struct {
    const char* name;
    ClutterAnimationMode mode;
} curves[] = {
    {"linear", CLUTTER_LINEAR},
    {"ease-in", CLUTTER_EASE_IN_QUAD},
    {"ease-out", CLUTTER_EASE_OUT_QUAD},
    {"ease-in-out", CLUTTER_EASE_IN_OUT_QUAD},
    {"ease-in-quad", CLUTTER_EASE_IN_QUAD},
    {"ease-out-quad", CLUTTER_EASE_OUT_QUAD},
    {"ease-in-out-quad", CLUTTER_EASE_IN_OUT_QUAD},
    {"ease-in-cubic", CLUTTER_EASE_IN_CUBIC},
    {"ease-out-cubic", CLUTTER_EASE_OUT_CUBIC},
    {"ease-in-out-cubic", CLUTTER_EASE_IN_OUT_CUBIC},
    {"ease-in-quart", CLUTTER_EASE_IN_QUART},
    {"ease-out-quart", CLUTTER_EASE_OUT_QUART},
    {"ease-in-out-quart", CLUTTER_EASE_IN_OUT_QUART},
    {"ease-in-quint", CLUTTER_EASE_IN_QUINT},
    {"ease-out-quint", CLUTTER_EASE_OUT_QUINT},
    {"ease-in-out-quint", CLUTTER_EASE_IN_OUT_QUINT},
    {"ease-in-sine", CLUTTER_EASE_IN_SINE},
    {"ease-out-sine", CLUTTER_EASE_OUT_SINE},
    {"ease-in-out-sine", CLUTTER_EASE_IN_OUT_SINE},
    {"ease-in-expo", CLUTTER_EASE_IN_EXPO},
    {"ease-out-expo", CLUTTER_EASE_OUT_EXPO},
    {"ease-in-out-expo", CLUTTER_EASE_IN_OUT_EXPO},
    {"ease-in-circ", CLUTTER_EASE_IN_CIRC},
    {"ease-out-circ", CLUTTER_EASE_OUT_CIRC},
    {"ease-in-out-circ", CLUTTER_EASE_IN_OUT_CIRC},
    {"ease-in-back", CLUTTER_EASE_IN_BACK},
    {"ease-out-back", CLUTTER_EASE_OUT_BACK},
    {"ease-in-out-back", CLUTTER_EASE_IN_OUT_BACK},
    {"ease-in-bounce", CLUTTER_EASE_IN_BOUNCE},
    {"ease-out-bounce", CLUTTER_EASE_OUT_BOUNCE},
    {"ease-in-out-bounce", CLUTTER_EASE_IN_OUT_BOUNCE},
    {"ease-in-elastic", CLUTTER_EASE_IN_ELASTIC},
    {"ease-out-elastic", CLUTTER_EASE_OUT_ELASTIC},
    {"ease-in-out-elastic", CLUTTER_EASE_IN_OUT_ELASTIC},
};

static ClutterAnimationMode curve_from_name(const char* name, ClutterAnimationMode fallback) {
    guint i;

    for (i = 0; i < G_N_ELEMENTS(curves); i++) {
        if (!g_ascii_strcasecmp(curves[i].name, name))
            return curves[i].mode;
    }

    g_warning("gnoblin: unknown animation curve '%s'", name);
    return fallback;
}

static const char* window_type_name(MetaWindow* window) {
    if (!window)
        return NULL;

    switch (meta_window_get_window_type(window)) {
    case META_WINDOW_NORMAL:
        return "normal";
    case META_WINDOW_DIALOG:
        return "dialog";
    case META_WINDOW_MODAL_DIALOG:
        return "modal-dialog";
    case META_WINDOW_MENU:
        return "menu";
    case META_WINDOW_DROPDOWN_MENU:
        return "dropdown-menu";
    case META_WINDOW_POPUP_MENU:
        return "popup-menu";
    case META_WINDOW_UTILITY:
        return "utility";
    case META_WINDOW_COMBO:
        return "combo";
    case META_WINDOW_TOOLTIP:
        return "tooltip";
    case META_WINDOW_NOTIFICATION:
        return "notification";
    case META_WINDOW_DESKTOP:
        return "desktop";
    case META_WINDOW_DOCK:
        return "dock";
    case META_WINDOW_SPLASHSCREEN:
        return "splashscreen";
    case META_WINDOW_DND:
        return "dnd";
    default:
        return NULL;
    }
}

static void set_default_animation(GnoblinAnim* a, const char* effect, const char* window_type) {
    /* Per-effect defaults follow the motion principles: entrances ease-out,
     * exits ease-in, everything well under 300ms. */
    if (!g_strcmp0(effect, "open")) {
        /* A clearer grow-in than the old 1.5% nudge: ~7% scale-up over a longer
         * decelerating curve so the window visibly "arrives" (it still fades in),
         * while dialogs/menus stay subtle since they pop up constantly. */
        a->duration_ms = 200;
        a->mode = CLUTTER_EASE_OUT_QUINT;
        a->scale = 0.93;
        if (!g_strcmp0(window_type, "dialog") || !g_strcmp0(window_type, "modal-dialog") ||
            !g_strcmp0(window_type, "utility")) {
            a->duration_ms = 130;
            a->mode = CLUTTER_EASE_OUT_CUBIC;
            a->scale = 0.97;
        } else if (!g_strcmp0(window_type, "menu") ||
                   !g_strcmp0(window_type, "dropdown-menu") ||
                   !g_strcmp0(window_type, "popup-menu") ||
                   !g_strcmp0(window_type, "combo")) {
            a->duration_ms = 80;
            a->mode = CLUTTER_EASE_OUT_QUAD;
            a->scale = 0.995;
        }
    } else if (!g_strcmp0(effect, "close")) {
        /* Mirror the open: a clearer shrink-away (~6%) as it fades out. */
        a->duration_ms = 150;
        a->mode = CLUTTER_EASE_IN_CUBIC;
        a->scale = 0.94;
        if (!g_strcmp0(window_type, "dialog") || !g_strcmp0(window_type, "modal-dialog") ||
            !g_strcmp0(window_type, "utility")) {
            a->duration_ms = 100;
            a->scale = 0.97;
        } else if (!g_strcmp0(window_type, "menu") ||
                   !g_strcmp0(window_type, "dropdown-menu") ||
                   !g_strcmp0(window_type, "popup-menu") ||
                   !g_strcmp0(window_type, "combo")) {
            a->duration_ms = 60;
            a->mode = CLUTTER_EASE_IN_QUAD;
            a->scale = 0.995;
        }
    } else if (!g_strcmp0(effect, "minimize")) {
        a->duration_ms = 160;
        a->mode = CLUTTER_EASE_IN_CUBIC;
        a->scale = 0.0;
    } else if (!g_strcmp0(effect, "workspace")) {
        a->duration_ms = 250;
        a->mode = CLUTTER_EASE_OUT_QUINT; /* a long, decelerating slide */
        a->scale = 1.0;
    } else if (!g_strcmp0(effect, "tile")) {
        a->duration_ms = 140;
        a->mode = CLUTTER_EASE_OUT_QUAD; /* snappy snap-zone highlight */
        a->scale = 0.96;
    } else if (!g_strcmp0(effect, "overview")) {
        a->duration_ms = 250;
        a->mode = CLUTTER_EASE_OUT_QUINT; /* windows glide into the grid */
        a->scale = 1.0;
    } else if (!g_strcmp0(effect, "maximize") || !g_strcmp0(effect, "unmaximize")) {
        /* GNOME-style frame cross-fade: the shell plugin grows the live actor
         * between its restored and maximized frames while a freeze-frame of the
         * old contents fades out over the top, so the client's relayout never
         * shows as a stretch. A decelerating curve makes the window expand from
         * its restored frame and settle cleanly at the target bounds. */
        a->duration_ms = !g_strcmp0(effect, "unmaximize") ? 280 : 300;
        a->mode = CLUTTER_EASE_OUT_QUART;
        a->scale = 1.0;
    } else                               /* resize / move */
    {
        a->duration_ms = 160;
        a->mode = CLUTTER_EASE_OUT_QUINT;
        a->scale = 1.0;
    }
}

static void apply_animation_spec(GnoblinAnim* a, const char* spec) {
    if (!spec)
        return;

    g_auto(GStrv) parts = g_strsplit(spec, ",", 3);

    if (parts[0]) {
        guint d;

        if (gnoblin_anim_parse_duration_ms(parts[0], &d))
            a->duration_ms = d;
    }
    if (parts[1])
        a->mode = curve_from_name(g_strstrip(parts[1]), a->mode);
    if (parts[2]) {
        double s;

        if (gnoblin_anim_parse_scale(parts[2], &s))
            a->scale = s;
    }
}

/* Debug knob: a global speed multiplier on every animation. `animation-speed`
 * is expressed as playback speed, so 0.1 runs everything at a tenth speed (10x
 * longer — handy for inspecting a transition frame by frame) and 2.0 runs it
 * twice as fast. Out-of-range or unparseable values fall back to normal speed. */
static double global_animation_speed(void) {
    /* `GNOBLIN_ANIM_SPEED` wins over config so it can be exported for a single
     * `just devkit` run without editing gnoblin.conf. */
    const char* env = g_getenv("GNOBLIN_ANIM_SPEED");
    g_autofree char* raw = NULL;
    char* end = NULL;
    double speed;

    if (env && *env) {
        speed = g_ascii_strtod(env, &end);
        if (end != env && speed > 0.0)
            return speed;
    }

    raw = gnoblin_config_get_string("animations", "animation-speed");
    if (!raw || !*raw)
        return 1.0;

    speed = g_ascii_strtod(raw, &end);
    if (end == raw || speed <= 0.0)
        return 1.0;

    return speed;
}

static GnoblinAnim gnoblin_anim_get_internal(const char* effect, const char* window_type) {
    GnoblinAnim a;
    g_autofree char* spec = NULL;

    set_default_animation(&a, effect, window_type);

    /* Default to the standard desktop preference (org.gnome.desktop.interface
     * enable-animations, which mutter tracks and GTK/other clients also honour),
     * so a single setting reduces motion everywhere — e.g. on a software-rendered
     * session like the devkit. gnoblin's own `[animations] enabled` still wins. */
    a.enabled = gnoblin_config_get_bool("animations", "enabled", meta_prefs_get_gnome_animations());

    /* `<effect> = DURATION, CURVE[, SCALE]` overrides the shared default. */
    spec = gnoblin_config_get_string("animations", effect);
    apply_animation_spec(&a, spec);

    /* `<effect>.<window-type>` refines by Mutter window type. This is close to
     * Hyprland's targeted animation model while keeping gnoblin's config small:
     * open.menu, close.dialog, open.normal, etc. */
    if (window_type && *window_type) {
        g_autofree char* typed_key = g_strdup_printf("%s.%s", effect, window_type);
        g_autofree char* typed_spec = gnoblin_config_get_string("animations", typed_key);

        apply_animation_spec(&a, typed_spec);
    }

    /* Apply the global speed scale last so every effect and per-type override is
     * stretched/compressed uniformly. Clamp so slow-mo can't round to 0 (which
     * would read as "disabled") and fast-forward stays sane. */
    if (a.duration_ms > 0) {
        double speed = global_animation_speed();

        if (speed != 1.0) {
            long scaled = lround((double)a.duration_ms / speed);

            a.duration_ms = (guint)CLAMP(scaled, 1L, 600000L);
        }
    }

    if (a.duration_ms == 0)
        a.enabled = FALSE;

    return a;
}

GnoblinAnim gnoblin_anim_get(const char* effect) {
    return gnoblin_anim_get_internal(effect, NULL);
}

GnoblinAnim gnoblin_anim_get_for_window(const char* effect, MetaWindow* window) {
    return gnoblin_anim_get_internal(effect, window_type_name(window));
}
