/*
 * gnoblin-shell: the visual window switcher (Alt+Tab). See gnoblin-switcher.h.
 *
 * This program is free software; you can redistribute it and/or modify it
 * under the terms of the GNU General Public License as published by the Free
 * Software Foundation; either version 2 of the License, or (at your option)
 * any later version.
 */

#include "gnoblin-switcher.h"

extern "C" {
#include <clutter/clutter-pango.h>
#include <meta/compositor.h>
#include <meta/meta-window-actor.h>
#include <meta/meta-workspace-manager.h>
#include <meta/window.h>
#include <meta/workspace.h>
}

#include "gnoblin-anim.h"
#include "gnoblin-rounded.h"

/* Layout constants (logical px). A cell holds one thumbnail plus its margin. */
#define THUMB 168.0f
#define CELL (THUMB + 28.0f)
#define PAD 22.0f
#define TITLE_H 30.0f

typedef struct {
    ClutterActor* tile;      /* per-window container (highlight + clone) */
    ClutterActor* highlight; /* selection backing, shown only when selected */
    MetaWindow* window;
} Item;

static struct {
    MetaDisplay* display;
    ClutterActor* catcher; /* full-screen reactive grab host */
    ClutterActor* panel;   /* centred dark panel */
    ClutterText* title;    /* selected window's title */
    ClutterGrab* grab;
    GArray* items; /* of Item */
    int selected;
    gboolean open;
} sw;

static void switcher_close(gboolean commit, guint32 timestamp);

static void on_catcher_destroyed(ClutterActor* catcher, gpointer user_data) {
    if (sw.catcher == catcher)
        sw.catcher = NULL;
    sw.panel = NULL;
    sw.title = NULL;
    sw.open = FALSE;
    if (sw.grab) {
        clutter_grab_dismiss(sw.grab);
        g_clear_object(&sw.grab);
    }
    g_clear_pointer(&sw.items, g_array_unref);
    sw.display = NULL;
}

/* Update the highlight backing and the title label for the current selection. */
static void apply_selection(void) {
    guint i;

    for (i = 0; i < sw.items->len; i++) {
        Item* it = &g_array_index(sw.items, Item, i);
        clutter_actor_set_opacity(it->highlight, (int)i == sw.selected ? 255 : 0);
    }
    if (sw.selected >= 0 && (guint)sw.selected < sw.items->len) {
        Item* it = &g_array_index(sw.items, Item, sw.selected);
        const char* t = it->window ? meta_window_get_title(it->window) : NULL;
        ClutterActor* label = CLUTTER_ACTOR(sw.title);
        float tw = 0, min_w = 0;

        clutter_text_set_text(sw.title, t ? t : "");
        /* Re-centre the (variable-width) title under the row of thumbnails. */
        clutter_actor_get_preferred_width(label, -1, &min_w, &tw);
        clutter_actor_set_x(label, (clutter_actor_get_width(sw.panel) - tw) / 2.0f);
    }
}

static void advance(int dir) {
    int n = (int)sw.items->len;

    if (n <= 0)
        return;
    sw.selected = (sw.selected + dir + n) % n;
    apply_selection();
}

/* True for the modifier keys that, when released, commit a held-modifier
 * switch (Alt+Tab / Super+Tab). */
static gboolean is_switch_modifier(guint sym) {
    switch (sym) {
    case CLUTTER_KEY_Alt_L:
    case CLUTTER_KEY_Alt_R:
    case CLUTTER_KEY_Super_L:
    case CLUTTER_KEY_Super_R:
    case CLUTTER_KEY_Meta_L:
    case CLUTTER_KEY_Meta_R:
        return TRUE;
    default:
        return FALSE;
    }
}

static gboolean on_catcher_event(ClutterActor* actor, ClutterEvent* event, gpointer user_data) {
    guint32 t = clutter_event_get_time(event);
    guint sym;

    switch (clutter_event_type(event)) {
    case CLUTTER_KEY_PRESS:
        sym = clutter_event_get_key_symbol(event);
        switch (sym) {
        case CLUTTER_KEY_Tab:
        case CLUTTER_KEY_Right:
        case CLUTTER_KEY_Down:
            advance((clutter_event_get_state(event) & CLUTTER_SHIFT_MASK) ? -1 : +1);
            return CLUTTER_EVENT_STOP;
        case CLUTTER_KEY_ISO_Left_Tab:
        case CLUTTER_KEY_Left:
        case CLUTTER_KEY_Up:
            advance(-1);
            return CLUTTER_EVENT_STOP;
        case CLUTTER_KEY_Return:
        case CLUTTER_KEY_KP_Enter:
        case CLUTTER_KEY_space:
            switcher_close(TRUE, t);
            return CLUTTER_EVENT_STOP;
        case CLUTTER_KEY_Escape:
            switcher_close(FALSE, t);
            return CLUTTER_EVENT_STOP;
        default:
            return CLUTTER_EVENT_STOP; /* swallow other keys while modal */
        }
    case CLUTTER_KEY_RELEASE:
        if (is_switch_modifier(clutter_event_get_key_symbol(event))) {
            switcher_close(TRUE, t);
            return CLUTTER_EVENT_STOP;
        }
        return CLUTTER_EVENT_STOP;
    case CLUTTER_BUTTON_PRESS:
        /* A click that reaches the catcher (not a tile) cancels. */
        switcher_close(FALSE, t);
        return CLUTTER_EVENT_STOP;
    default:
        return CLUTTER_EVENT_PROPAGATE;
    }
}

/* Click a thumbnail: select it and commit. */
static gboolean on_tile_clicked(ClutterActor* tile, ClutterEvent* event, gpointer user_data) {
    sw.selected = GPOINTER_TO_INT(user_data);
    switcher_close(TRUE, clutter_event_get_time(event));
    return CLUTTER_EVENT_STOP;
}

/* The windows to offer, in most-recently-used order (tab list order). */
static GList* mru_windows(MetaDisplay* display) {
    MetaWorkspaceManager* wm = meta_display_get_workspace_manager(display);
    MetaWorkspace* ws = meta_workspace_manager_get_active_workspace(wm);

    return meta_display_get_tab_list(display, META_TAB_LIST_NORMAL, ws);
}

static void switcher_open(MetaDisplay* display, gboolean backward) {
    MetaCompositor* compositor = meta_display_get_compositor(display);
    ClutterActor* stage = CLUTTER_ACTOR(meta_compositor_get_stage(compositor));
    CoglColor dim = (CoglColor)COGL_COLOR_INIT(0, 0, 0, 90);
    CoglColor panel_bg = (CoglColor)COGL_COLOR_INIT(30, 30, 36, 240);
    CoglColor hl = (CoglColor)COGL_COLOR_INIT(255, 255, 255, 38);
    CoglColor white = (CoglColor)COGL_COLOR_INIT(235, 235, 240, 255);
    g_autoptr(GList) windows = mru_windows(display);
    int n = g_list_length(windows);
    int sw_w, sh, i;
    float panel_w, panel_h, x;
    GList* l;

    if (n < 2)
        return; /* nothing to switch between */

    sw.display = display;
    sw.items = g_array_new(FALSE, TRUE, sizeof(Item));
    meta_display_get_size(display, &sw_w, &sh);

    sw.catcher = clutter_actor_new();
    clutter_actor_set_size(sw.catcher, sw_w, sh);
    clutter_actor_set_background_color(sw.catcher, &dim);
    clutter_actor_set_reactive(sw.catcher, TRUE);
    clutter_actor_add_child(stage, sw.catcher);
    g_signal_connect(sw.catcher, "destroy", G_CALLBACK(on_catcher_destroyed), NULL);
    g_signal_connect(sw.catcher, "event", G_CALLBACK(on_catcher_event), NULL);

    panel_w = n * CELL + 2 * PAD;
    panel_h = CELL + TITLE_H + 2 * PAD;
    sw.panel = clutter_actor_new();
    clutter_actor_set_size(sw.panel, panel_w, panel_h);
    clutter_actor_set_position(sw.panel, (sw_w - panel_w) / 2.0f, (sh - panel_h) / 2.0f);
    clutter_actor_set_background_color(sw.panel, &panel_bg);
    clutter_actor_add_child(sw.catcher, sw.panel);

    for (l = windows, i = 0; l; l = l->next, i++) {
        MetaWindow* w = META_WINDOW(l->data);
        MetaWindowActor* wa = META_WINDOW_ACTOR(meta_window_get_compositor_private(w));
        Item it = {};
        ClutterActor* tile = clutter_actor_new();
        ClutterActor* highlight = clutter_actor_new();

        x = PAD + i * CELL;
        clutter_actor_set_size(tile, CELL, CELL);
        clutter_actor_set_position(tile, x, PAD);
        clutter_actor_set_reactive(tile, TRUE);
        g_signal_connect(tile, "button-press-event", G_CALLBACK(on_tile_clicked),
                         GINT_TO_POINTER(i));
        clutter_actor_add_child(sw.panel, tile);

        clutter_actor_set_size(highlight, CELL, CELL);
        clutter_actor_set_background_color(highlight, &hl);
        clutter_actor_set_opacity(highlight, 0);
        clutter_actor_add_child(tile, highlight);

        if (wa) {
            ClutterActor* src = CLUTTER_ACTOR(wa);
            ClutterActor* clone = clutter_clone_new(src);
            float ww, wh, scale, tw, th;

            clutter_actor_get_size(src, &ww, &wh);
            ww = MAX(1.0f, ww);
            wh = MAX(1.0f, wh);
            scale = MIN(THUMB / ww, THUMB / wh);
            if (scale > 1.0f)
                scale = 1.0f;
            tw = ww * scale;
            th = wh * scale;
            clutter_actor_set_size(clone, ww, wh);
            clutter_actor_set_scale(clone, scale, scale);
            /* Centre the scaled clone in the cell (scale pivots at the origin). */
            clutter_actor_set_position(clone, (CELL - tw) / 2.0f, (CELL - th) / 2.0f);
            clutter_actor_add_child(tile, clone);
        }

        it.tile = tile;
        it.highlight = highlight;
        it.window = w;
        g_array_append_val(sw.items, it);
    }

    sw.title = CLUTTER_TEXT(clutter_text_new_full("Sans 11", "", &white));
    clutter_text_set_single_line_mode(sw.title, TRUE);
    clutter_text_set_ellipsize(sw.title, PANGO_ELLIPSIZE_END);
    /* y is fixed; apply_selection() re-centres x for the current title's width. */
    clutter_actor_set_y(CLUTTER_ACTOR(sw.title), PAD + CELL + (TITLE_H - 18) / 2.0f);
    clutter_actor_add_child(sw.panel, CLUTTER_ACTOR(sw.title));

    /* Match gnoblin's window rounding so the panel reads as part of the shell. */
    clutter_actor_add_effect(sw.panel, gnoblin_rounded_new(16.0f));

    /* Start on the next/previous window (index 0 is the current one). */
    sw.selected = backward ? n - 1 : (n > 1 ? 1 : 0);
    apply_selection();

    sw.grab = clutter_stage_grab(CLUTTER_STAGE(stage), sw.catcher);
    clutter_actor_grab_key_focus(sw.catcher);
    sw.open = TRUE;
}

static void switcher_close(gboolean commit, guint32 timestamp) {
    MetaWindow* target = NULL;

    if (!sw.open)
        return;
    sw.open = FALSE;

    if (commit && sw.selected >= 0 && (guint)sw.selected < sw.items->len)
        target = g_array_index(sw.items, Item, sw.selected).window;

    if (sw.grab) {
        clutter_grab_dismiss(sw.grab);
        g_clear_object(&sw.grab);
    }
    g_clear_pointer(&sw.catcher, clutter_actor_destroy); /* drops panel + clones */
    sw.panel = NULL;
    sw.title = NULL;
    g_clear_pointer(&sw.items, g_array_unref);
    sw.display = NULL;

    if (target)
        meta_window_activate(target, timestamp);
}

void gnoblin_switcher_cycle(MetaDisplay* display, gboolean backward, guint32 timestamp) {
    if (sw.open)
        advance(backward ? -1 : +1);
    else
        switcher_open(display, backward);
}
