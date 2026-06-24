/*
 * gnoblin-shell: the Overview (Activities). See gnoblin-overview.h.
 *
 * This program is free software; you can redistribute it and/or modify it
 * under the terms of the GNU General Public License as published by the Free
 * Software Foundation; either version 2 of the License, or (at your option)
 * any later version.
 */

#include "gnoblin-overview.h"

#include <math.h>

extern "C" {
#include <meta/compositor.h>
#include <meta/meta-window-actor.h>
#include <meta/meta-workspace-manager.h>
#include <meta/window.h>
#include <meta/workspace.h>
}

#include "gnoblin-anim.h"
#include "gnoblin-blur.h"
#include "gnoblin-config.h"

/* One live overview at a time. */
typedef struct {
    MetaDisplay* display;
    ClutterActor* overlay; /* opaque full-screen group: dark bg + window clones */
    ClutterGrab* grab;
    gboolean open;
} GnoblinOverview;

static GnoblinOverview ov;

static void overview_close(void);

static void on_overlay_destroyed(ClutterActor* overlay, gpointer user_data) {
    if (ov.overlay == overlay)
        ov.overlay = NULL;
    ov.open = FALSE;
    if (ov.grab) {
        clutter_grab_dismiss(ov.grab);
        g_clear_object(&ov.grab);
    }
    ov.display = NULL;
}

/* Click a thumbnail: focus its window and dismiss. */
static gboolean on_clone_clicked(ClutterActor* clone, ClutterEvent* event, gpointer user_data) {
    MetaWindow* window = (MetaWindow*)g_object_get_data(G_OBJECT(clone), "gnoblin-ov-window");

    if (window)
        meta_window_activate(window, clutter_event_get_time(event));
    overview_close();
    return CLUTTER_EVENT_STOP;
}

/* Escape or a click on the empty backdrop dismisses the overview. */
static gboolean on_overlay_event(ClutterActor* overlay, ClutterEvent* event, gpointer user_data) {
    switch (clutter_event_type(event)) {
    case CLUTTER_KEY_PRESS:
        if (clutter_event_get_key_symbol(event) == CLUTTER_KEY_Escape) {
            overview_close();
            return CLUTTER_EVENT_STOP;
        }
        return CLUTTER_EVENT_STOP; /* swallow keys while open */
    case CLUTTER_BUTTON_PRESS:
        overview_close();
        return CLUTTER_EVENT_STOP;
    default:
        return CLUTTER_EVENT_PROPAGATE;
    }
}

/* Collect the normal, non-minimized windows on the active workspace. */
static GList* visible_windows(MetaDisplay* display) {
    MetaCompositor* compositor = meta_display_get_compositor(display);
    MetaWorkspaceManager* wm = meta_display_get_workspace_manager(display);
    MetaWorkspace* ws = meta_workspace_manager_get_active_workspace(wm);
    GList* out = NULL;
    GList* l;

    for (l = meta_compositor_get_window_actors(compositor); l; l = l->next) {
        MetaWindowActor* wa = META_WINDOW_ACTOR(l->data);
        MetaWindow* w = meta_window_actor_get_meta_window(wa);

        if (!w || meta_window_get_window_type(w) != META_WINDOW_NORMAL)
            continue;
        if (meta_window_is_skip_taskbar(w) || meta_window_is_hidden(w))
            continue;
        if (!meta_window_located_on_workspace(w, ws))
            continue;
        out = g_list_prepend(out, wa);
    }
    return g_list_reverse(out);
}

static void overview_open(MetaDisplay* display) {
    MetaCompositor* compositor = meta_display_get_compositor(display);
    ClutterActor* stage = CLUTTER_ACTOR(meta_compositor_get_stage(compositor));
    GnoblinAnim anim = gnoblin_anim_get("overview");
    int blur_radius = gnoblin_config_get_int("appearance", "blur", 0);
    /* Opt-in blur: show a blurred clone of the desktop behind a translucent tint
     * (GNOME-style). Default (radius 0) keeps an opaque backdrop. */
    CoglColor backdrop = blur_radius > 0 ? (CoglColor)COGL_COLOR_INIT(16, 16, 20, 130)
                                         : (CoglColor)COGL_COLOR_INIT(16, 16, 20, 255);
    g_autoptr(GList) windows = visible_windows(display);
    int n = g_list_length(windows);
    int sw, sh, cols, rows, top = 48;
    float cell_w, cell_h;
    GList* l;
    int i;

    ov.display = display;
    meta_display_get_size(display, &sw, &sh);

    ov.overlay = clutter_actor_new();
    clutter_actor_set_size(ov.overlay, sw, sh);
    clutter_actor_set_reactive(ov.overlay, TRUE);
    clutter_actor_add_child(stage, ov.overlay); /* added last == on top of everything */
    g_signal_connect(ov.overlay, "destroy", G_CALLBACK(on_overlay_destroyed), NULL);
    g_signal_connect(ov.overlay, "event", G_CALLBACK(on_overlay_event), NULL);

    if (blur_radius > 0) {
        /* Bottom layer: a blurred clone of the live window group (wallpaper +
         * windows). The overlay's own background colour above it is the tint. */
        ClutterActor* wg = CLUTTER_ACTOR(meta_compositor_get_window_group(compositor));
        ClutterActor* backdrop_clone = clutter_clone_new(wg);

        clutter_actor_set_size(backdrop_clone, sw, sh);
        clutter_actor_add_effect(backdrop_clone, gnoblin_blur_new((float)blur_radius));
        clutter_actor_add_child(ov.overlay, backdrop_clone);

        /* The tint sits above the blurred clone but below the thumbnails. */
        ClutterActor* tint = clutter_actor_new();
        clutter_actor_set_size(tint, sw, sh);
        clutter_actor_set_background_color(tint, &backdrop);
        clutter_actor_add_child(ov.overlay, tint);
    } else {
        clutter_actor_set_background_color(ov.overlay, &backdrop);
    }

    cols = n > 0 ? (int)ceil(sqrt((double)n)) : 1;
    rows = n > 0 ? (n + cols - 1) / cols : 1;
    cell_w = (float)sw / cols;
    cell_h = (float)(sh - top) / rows;

    for (l = windows, i = 0; l; l = l->next, i++) {
        MetaWindowActor* wa = META_WINDOW_ACTOR(l->data);
        MetaWindow* w = meta_window_actor_get_meta_window(wa);
        ClutterActor* src = CLUTTER_ACTOR(wa);
        ClutterActor* clone = clutter_clone_new(src);
        float win_w, win_h, rx = 0, ry = 0, scale, cx, cy;
        int col = i % cols, row = i / cols;

        clutter_actor_get_size(src, &win_w, &win_h);
        clutter_actor_get_transformed_position(src, &rx, &ry);
        win_w = MAX(1.0f, win_w);
        win_h = MAX(1.0f, win_h);

        clutter_actor_set_size(clone, win_w, win_h);
        clutter_actor_set_pivot_point(clone, 0.5f, 0.5f);
        clutter_actor_set_position(clone, rx, ry); /* start at the real window */
        clutter_actor_set_reactive(clone, TRUE);
        g_object_set_data(G_OBJECT(clone), "gnoblin-ov-window", w);
        g_signal_connect(clone, "button-press-event", G_CALLBACK(on_clone_clicked), NULL);
        clutter_actor_add_child(ov.overlay, clone);

        /* Fit the thumbnail into its cell (with a margin), centred. With a centre
         * pivot, scaling keeps the centre fixed, so we just place the centre at
         * the cell centre. */
        scale = MIN(cell_w * 0.86f / win_w, cell_h * 0.86f / win_h);
        if (scale > 1.0f)
            scale = 1.0f;
        cx = col * cell_w + cell_w / 2.0f;
        cy = top + row * cell_h + cell_h / 2.0f;

        clutter_actor_save_easing_state(clone);
        clutter_actor_set_easing_mode(clone, anim.mode);
        clutter_actor_set_easing_duration(clone, anim.enabled ? anim.duration_ms : 0);
        clutter_actor_set_position(clone, cx - win_w / 2.0f, cy - win_h / 2.0f);
        clutter_actor_set_scale(clone, scale, scale);
        clutter_actor_restore_easing_state(clone);
    }

    ov.grab = clutter_stage_grab(CLUTTER_STAGE(stage), ov.overlay);
    clutter_actor_grab_key_focus(ov.overlay);
    ov.open = TRUE;
}

static void overview_close(void) {
    if (!ov.open)
        return;
    ov.open = FALSE;

    if (ov.grab) {
        clutter_grab_dismiss(ov.grab);
        g_clear_object(&ov.grab);
    }
    /* Destroying the overlay drops every clone; the real windows (which were
     * occluded behind the opaque backdrop) reappear. */
    g_clear_pointer(&ov.overlay, clutter_actor_destroy);
    ov.display = NULL;
}

void gnoblin_overview_toggle(MetaDisplay* display) {
    if (ov.open)
        overview_close();
    else
        overview_open(display);
}
