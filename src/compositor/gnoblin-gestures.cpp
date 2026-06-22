/*
 * gnoblin-shell: touchpad gestures -> the action dispatcher. See
 * gnoblin-gestures.h.
 *
 * This program is free software; you can redistribute it and/or modify it
 * under the terms of the GNU General Public License as published by the Free
 * Software Foundation; either version 2 of the License, or (at your option)
 * any later version.
 */

#include "gnoblin-gestures.h"

#include <math.h>

extern "C" {
#include <meta/compositor.h>
}

#include "gnoblin-actions.h"
#include "gnoblin-config.h"

/* A swipe must travel at least this far (accumulated px) to count, so a stray
 * brush of the touchpad doesn't fire an action. */
#define SWIPE_THRESHOLD 40.0

/* Built-in defaults, used when the [gestures] section doesn't map a key. Three
 * fingers: switch workspaces / open the Overview, like GNOME. */
static const struct {
    const char* key;
    const char* action;
    const char* arg;
} defaults[] = {
    {"swipe-3-left", "workspace", "next"},
    {"swipe-3-right", "workspace", "prev"},
    {"swipe-3-up", "overview", ""},
    {"swipe-4-up", "overview", ""},
    {"swipe-3-down", "overview", ""},
};

void gnoblin_gestures_trigger(MetaDisplay* display, const char* key, guint32 timestamp) {
    g_autofree char* spec = gnoblin_config_get_string("gestures", key);
    const char* action = NULL;
    g_autofree char* arg = NULL;

    if (spec) {
        char* trimmed = g_strstrip(spec);

        /* `none` or an empty value explicitly disables a gesture. */
        if (trimmed[0] == '\0' || !g_strcmp0(trimmed, "none"))
            return;

        g_auto(GStrv) parts = g_strsplit(trimmed, " ", 2);
        if (parts[0] && parts[0][0]) {
            gnoblin_actions_dispatch(display, parts[0],
                                     parts[1] ? g_strstrip(parts[1]) : NULL, NULL, timestamp);
            return;
        }
    }

    for (guint i = 0; i < G_N_ELEMENTS(defaults); i++) {
        if (!g_strcmp0(defaults[i].key, key)) {
            action = defaults[i].action;
            arg = g_strdup(defaults[i].arg);
            break;
        }
    }
    if (action)
        gnoblin_actions_dispatch(display, action, (arg && arg[0]) ? arg : NULL, NULL, timestamp);
}

static gboolean on_captured_event(ClutterActor* stage, ClutterEvent* event, gpointer user_data) {
    MetaDisplay* display = META_DISPLAY(user_data);
    static double acc_dx, acc_dy; /* one touchpad gesture is active at a time */

    switch (clutter_event_type(event)) {
    case CLUTTER_TOUCHPAD_SWIPE: {
        ClutterTouchpadGesturePhase phase = clutter_event_get_gesture_phase(event);
        guint fingers = clutter_event_get_touchpad_gesture_finger_count(event);

        if (phase == CLUTTER_TOUCHPAD_GESTURE_PHASE_BEGIN) {
            acc_dx = acc_dy = 0;
        } else if (phase == CLUTTER_TOUCHPAD_GESTURE_PHASE_UPDATE) {
            double dx = 0, dy = 0;
            clutter_event_get_gesture_motion_delta(event, &dx, &dy);
            acc_dx += dx;
            acc_dy += dy;
        } else if (phase == CLUTTER_TOUCHPAD_GESTURE_PHASE_END) {
            const char* dir = NULL;

            if (fabs(acc_dx) > fabs(acc_dy)) {
                if (fabs(acc_dx) > SWIPE_THRESHOLD)
                    dir = acc_dx < 0 ? "left" : "right";
            } else {
                if (fabs(acc_dy) > SWIPE_THRESHOLD)
                    dir = acc_dy < 0 ? "up" : "down";
            }
            if (dir) {
                g_autofree char* key = g_strdup_printf("swipe-%u-%s", fingers, dir);
                gnoblin_gestures_trigger(display, key, clutter_event_get_time(event));
            }
            acc_dx = acc_dy = 0;
        }
        return CLUTTER_EVENT_PROPAGATE;
    }
    case CLUTTER_TOUCHPAD_PINCH:
        if (clutter_event_get_gesture_phase(event) == CLUTTER_TOUCHPAD_GESTURE_PHASE_END) {
            double scale = clutter_event_get_gesture_pinch_scale(event);
            guint fingers = clutter_event_get_touchpad_gesture_finger_count(event);

            if (fabs(scale - 1.0) > 0.1) {
                g_autofree char* key =
                    g_strdup_printf("pinch-%u-%s", fingers, scale < 1.0 ? "in" : "out");
                gnoblin_gestures_trigger(display, key, clutter_event_get_time(event));
            }
        }
        return CLUTTER_EVENT_PROPAGATE;
    default:
        return CLUTTER_EVENT_PROPAGATE;
    }
}

void gnoblin_gestures_init(MetaDisplay* display) {
    MetaCompositor* compositor = meta_display_get_compositor(display);
    ClutterActor* stage = CLUTTER_ACTOR(meta_compositor_get_stage(compositor));

    /* The stage-wide event hook is opt-outable (`[gestures] enabled = false`)
     * so it can be ruled out if it's ever suspected of affecting input. The
     * `gesture` action keeps working regardless. */
    if (!gnoblin_config_get_bool("gestures", "enabled", TRUE))
        return;

    g_signal_connect(stage, "captured-event", G_CALLBACK(on_captured_event), display);
}
