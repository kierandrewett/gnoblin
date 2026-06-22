/*
 * gnoblin-shell: the window-action API — gnoblin's public "dispatcher".
 *
 * This program is free software; you can redistribute it and/or modify it
 * under the terms of the GNU General Public License as published by the Free
 * Software Foundation; either version 2 of the License, or (at your option)
 * any later version.
 */

#include "gnoblin-actions.h"

#include <stdio.h>
#include <string.h>

extern "C" {
#include <meta/compositor.h>
#include <meta/display.h>
#include <meta/meta-enums.h>
#include <meta/meta-window-actor.h>
#include <meta/meta-workspace-manager.h>
#include <meta/window.h>
#include <meta/workspace.h>
}

#include "gnoblin-actions-spec.h"
#include "gnoblin-config.h"
#include "gnoblin-control.h"
#include "gnoblin-gestures.h"
#include "gnoblin-lock.h"
#include "gnoblin-overview.h"
#include "gnoblin-roles.h"
#include "gnoblin-switcher.h"

/* ---- snap regions: fractions (x, y, w, h) of the monitor work area ---- */

typedef struct {
    const char* name;
    double x, y, w, h;
} SnapRegion;

static const SnapRegion builtin_regions[] = {
    {"left", 0.0, 0.0, 0.5, 1.0},        {"right", 0.5, 0.0, 0.5, 1.0},
    {"top", 0.0, 0.0, 1.0, 0.5},         {"bottom", 0.0, 0.5, 1.0, 0.5},
    {"top-left", 0.0, 0.0, 0.5, 0.5},    {"top-right", 0.5, 0.0, 0.5, 0.5},
    {"bottom-left", 0.0, 0.5, 0.5, 0.5}, {"bottom-right", 0.5, 0.5, 0.5, 0.5},
    {"center", 0.15, 0.15, 0.7, 0.7},    {"full", 0.0, 0.0, 1.0, 1.0},
};

/* Resolve a snap region name to fractions. The config can override a built-in
 * or add a new one with `snap-<name> = "x y w h"` (fractions). */
static gboolean resolve_region(const char* name, double* x, double* y, double* w, double* h) {
    g_autofree char* override = gnoblin_config_get_string("snap", name);
    guint i;

    if (override) {
        if (gnoblin_actions_parse_snap_region(override, x, y, w, h))
            return TRUE;
        g_warning("gnoblin: invalid snap region '%s' = '%s'", name, override);
    }

    for (i = 0; i < G_N_ELEMENTS(builtin_regions); i++) {
        if (g_strcmp0(builtin_regions[i].name, name) == 0) {
            *x = builtin_regions[i].x;
            *y = builtin_regions[i].y;
            *w = builtin_regions[i].w;
            *h = builtin_regions[i].h;
            return TRUE;
        }
    }

    return FALSE;
}

/* Place a window at fractions (fx,fy,fw,fh) of its monitor work area, inset by
 * the configured `[appearance] gaps` so tiled windows don't touch each other or
 * the screen edge. The gap is applied on every side (so adjacent tiles leave
 * 2*gap between them, gap to the screen edge). */
static void place_fraction(MetaWindow* window, double fx, double fy, double fw, double fh) {
    MtkRectangle wa;
    int gap = gnoblin_config_get_int("appearance", "gaps", 0);
    int x, y, w, h;

    meta_window_get_work_area_current_monitor(window, &wa);
    if (meta_window_is_maximized(window))
        meta_window_unmaximize(window);
    if (meta_window_is_fullscreen(window))
        meta_window_unmake_fullscreen(window);

    x = wa.x + (int)(fx * wa.width);
    y = wa.y + (int)(fy * wa.height);
    w = (int)(fw * wa.width);
    h = (int)(fh * wa.height);
    if (gap > 0) {
        x += gap;
        y += gap;
        w = MAX(1, w - 2 * gap);
        h = MAX(1, h - 2 * gap);
    }
    meta_window_move_resize_frame(window, TRUE, x, y, w, h);
}

static void snap_window(MetaWindow* window, const char* region) {
    double fx, fy, fw, fh;

    if (!region || !resolve_region(region, &fx, &fy, &fw, &fh)) {
        g_warning("gnoblin: unknown snap region '%s'", region ? region : "");
        return;
    }
    place_fraction(window, fx, fy, fw, fh);
}

/* Keyboard half-tiling with KDE-style cycling: repeated `tile left` cycles the
 * window through left-half -> left-two-thirds -> left-one-third (and likewise
 * right/up/down). The cycle step is remembered per window. */
static void tile_window(MetaWindow* window, const char* dirname) {
    static GQuark tile_quark;
    static const double sizes[3] = {0.5, 2.0 / 3.0, 1.0 / 3.0};
    int dir;
    int* state;
    int step;
    double s, fx = 0, fy = 0, fw = 1, fh = 1;

    if (!g_strcmp0(dirname, "left"))
        dir = 0;
    else if (!g_strcmp0(dirname, "right"))
        dir = 1;
    else if (!g_strcmp0(dirname, "up"))
        dir = 2;
    else if (!g_strcmp0(dirname, "down"))
        dir = 3;
    else {
        g_warning("gnoblin: tile direction must be left/right/up/down, got '%s'",
                  dirname ? dirname : "");
        return;
    }

    if (G_UNLIKELY(tile_quark == 0))
        tile_quark = g_quark_from_static_string("gnoblin-tile-state");
    /* state packs (dir<<8 | step); 0 = unset. */
    state = (int*)g_object_get_qdata(G_OBJECT(window), tile_quark);
    if (!state) {
        state = g_new0(int, 1);
        *state = -1;
        g_object_set_qdata_full(G_OBJECT(window), tile_quark, state, g_free);
    }
    step = (*state >> 8) == dir ? ((*state & 0xff) + 1) % 3 : 0;
    *state = (dir << 8) | step;

    s = sizes[step];
    switch (dir) {
    case 0: fw = s; break;                  /* left  */
    case 1: fx = 1.0 - s; fw = s; break;    /* right */
    case 2: fh = s; break;                  /* up    */
    case 3: fy = 1.0 - s; fh = s; break;    /* down  */
    }
    place_fraction(window, fx, fy, fw, fh);
}

/* Switch to a workspace by `arg`: "next", "prev", or a 1-based index. */
static void switch_workspace(MetaDisplay* display, const char* arg, guint32 timestamp) {
    MetaWorkspaceManager* wm = meta_display_get_workspace_manager(display);
    int n = meta_workspace_manager_get_n_workspaces(wm);
    int cur = meta_workspace_manager_get_active_workspace_index(wm);
    int target;

    if (!arg)
        return;
    if (!g_strcmp0(arg, "next"))
        target = (cur + 1) % n;
    else if (!g_strcmp0(arg, "prev"))
        target = (cur - 1 + n) % n;
    else if (!gnoblin_actions_parse_workspace_index(arg, &target)) {
        g_warning("gnoblin: workspace action needs a 1-based workspace index, got '%s'", arg);
        return;
    }

    if (target >= 0 && target < n) {
        MetaWorkspace* ws = meta_workspace_manager_get_workspace_by_index(wm, target);
        if (ws)
            meta_workspace_activate(ws, timestamp);
    }
}

/* Focus the next/prev window in the alt-tab order on the active workspace. */
static void cycle_focus(MetaDisplay* display, MetaWindow* current, const char* arg,
                        guint32 timestamp) {
    MetaWorkspaceManager* wm = meta_display_get_workspace_manager(display);
    MetaWorkspace* ws = meta_workspace_manager_get_active_workspace(wm);
    gboolean backward = !g_strcmp0(arg, "prev");
    MetaWindow* next =
        meta_display_get_tab_next(display, META_TAB_LIST_NORMAL, ws, current, backward);

    if (next)
        meta_window_activate(next, timestamp);
}

/* Find the MetaWindowActor backing a window (mutter has no public accessor). */
static MetaWindowActor* actor_for_window(MetaDisplay* display, MetaWindow* window) {
    MetaCompositor* compositor = meta_display_get_compositor(display);
    GList* l;

    for (l = meta_compositor_get_window_actors(compositor); l; l = l->next) {
        MetaWindowActor* wa = META_WINDOW_ACTOR(l->data);
        if (meta_window_actor_get_meta_window(wa) == window)
            return wa;
    }
    return NULL;
}

/* Move `window` to the next/prev monitor (wrapping), or a 0-based index. */
static void move_to_monitor(MetaDisplay* display, MetaWindow* window, const char* arg) {
    int n = meta_display_get_n_monitors(display);
    int cur = meta_window_get_monitor(window);
    int target;

    if (n <= 1 || !arg)
        return;
    if (!g_strcmp0(arg, "next"))
        target = (cur + 1) % n;
    else if (!g_strcmp0(arg, "prev"))
        target = (cur - 1 + n) % n;
    else if (!gnoblin_actions_parse_monitor_index(arg, &target)) {
        g_warning("gnoblin: move-to-monitor action needs a 0-based monitor index, got '%s'", arg);
        return;
    }

    if (target >= 0 && target < n)
        meta_window_move_to_monitor(window, target);
}

/* Centre `window` on its current monitor's work area. */
static void center_window(MetaWindow* window) {
    MtkRectangle r, wa;

    if (meta_window_is_maximized(window))
        meta_window_unmaximize(window);
    meta_window_get_frame_rect(window, &r);
    meta_window_get_work_area_current_monitor(window, &wa);
    meta_window_move_resize_frame(window, TRUE, wa.x + (wa.width - r.width) / 2,
                                  wa.y + (wa.height - r.height) / 2, r.width, r.height);
}

/* Move keyboard focus to the most-recent window on the next/prev monitor. */
static void focus_monitor(MetaDisplay* display, const char* arg, guint32 timestamp) {
    int n = meta_display_get_n_monitors(display);
    MetaWindow* cur = meta_display_get_focus_window(display);
    int from = cur ? meta_window_get_monitor(cur) : meta_display_get_current_monitor(display);
    gboolean prev = !g_strcmp0(arg, "prev");
    int target;
    MetaWorkspaceManager* wm = meta_display_get_workspace_manager(display);
    MetaWorkspace* ws = meta_workspace_manager_get_active_workspace(wm);
    GList* tab;
    GList* l;

    if (n <= 1)
        return;
    target = ((from + (prev ? -1 : 1)) % n + n) % n;
    tab = meta_display_get_tab_list(display, META_TAB_LIST_NORMAL, ws);
    for (l = tab; l; l = l->next) {
        MetaWindow* w = META_WINDOW(l->data);
        if (meta_window_get_monitor(w) == target) {
            meta_window_activate(w, timestamp);
            break;
        }
    }
    g_list_free(tab);
}

/* Swap `window`'s geometry with the next window in alt-tab order (floating
 * "swap" — handy without a tiling layout). */
static void swap_window(MetaDisplay* display, MetaWindow* window, guint32 timestamp) {
    MetaWorkspaceManager* wm = meta_display_get_workspace_manager(display);
    MetaWorkspace* ws = meta_workspace_manager_get_active_workspace(wm);
    MetaWindow* other = meta_display_get_tab_next(display, META_TAB_LIST_NORMAL, ws, window, FALSE);
    MtkRectangle a, b;

    if (!other || other == window)
        return;
    meta_window_get_frame_rect(window, &a);
    meta_window_get_frame_rect(other, &b);
    meta_window_move_resize_frame(window, TRUE, b.x, b.y, b.width, b.height);
    meta_window_move_resize_frame(other, TRUE, a.x, a.y, a.width, a.height);
}

/* ---- scratchpad (Hyprland-style named special workspaces) ----
 *
 * A scratchpad is a named off-screen holding pen. A window is tagged with its
 * scratchpad name (qdata) and minimized to hide it (minimized windows are
 * is_hidden, so they drop out of the switcher/overview). `scratchpad <name>`
 * toggles the whole named set: if any of its windows are showing, hide them all;
 * otherwise pull them onto the active workspace, raised + focused. */
static GQuark scratch_quark(void) {
    static GQuark q;
    if (!q)
        q = g_quark_from_static_string("gnoblin-scratchpad");
    return q;
}

static const char* scratch_name_of(MetaWindow* w) {
    return (const char*)g_object_get_qdata(G_OBJECT(w), scratch_quark());
}

/* Tag `window` for scratchpad `name` (default "scratch") and hide it. */
static void scratchpad_stash(MetaWindow* window, const char* name) {
    g_object_set_qdata_full(G_OBJECT(window), scratch_quark(),
                            g_strdup(name && name[0] ? name : "scratch"), g_free);
    meta_window_minimize(window);
}

static void scratchpad_toggle(MetaDisplay* display, const char* name, guint32 timestamp) {
    MetaWorkspaceManager* wm = meta_display_get_workspace_manager(display);
    MetaWorkspace* active = meta_workspace_manager_get_active_workspace(wm);
    const char* want = name && name[0] ? name : "scratch";
    g_autoptr(GList) all = NULL; /* shallow: list owns nothing */
    GList* l;
    GList* members = NULL;
    gboolean any_shown = FALSE;
    MetaWindow* first = NULL;

    all = meta_display_list_all_windows(display);
    for (l = all; l; l = l->next) {
        MetaWindow* w = META_WINDOW(l->data);
        if (g_strcmp0(scratch_name_of(w), want) != 0)
            continue;
        members = g_list_prepend(members, w);
        if (!meta_window_is_hidden(w))
            any_shown = TRUE;
    }

    for (l = members; l; l = l->next) {
        MetaWindow* w = META_WINDOW(l->data);
        if (any_shown) {
            meta_window_minimize(w); /* hide the whole set */
        } else {
            meta_window_change_workspace(w, active); /* follow me to the active ws */
            meta_window_unminimize(w);
            meta_window_make_above(w); /* float it over as an overlay */
            first = w;
        }
    }
    if (first)
        meta_window_activate(first, timestamp);
    g_list_free(members);
}

/* ---- actions ---- */

static const char* const action_names[] = {
    "close",
    "maximize",
    "minimize",
    "fullscreen",
    "always-on-top",
    "always-on-visible-workspace",
    "move",
    "resize",
    "snap",
    "tile",
    "center",
    "workspace",
    "move-to-workspace",
    "move-to-monitor",
    "focus",
    "switcher",
    "focus-monitor",
    "swap",
    "opacity",
    "overview",
    "gesture",
    "scratchpad",
    "scratchpad-stash",
    "spawn",
    "lock",
    "window-menu",
    NULL,
};

const char* const* gnoblin_actions_list(void) {
    return action_names;
}

void gnoblin_actions_dispatch(MetaDisplay* display, const char* action, const char* arg,
                              MetaWindow* target, guint32 timestamp) {
    MetaWindow* window = target ? target : meta_display_get_focus_window(display);

    if (!action || action[0] == '\0')
        return;
    if (gnoblin_control_is_locked() && g_strcmp0(action, "lock"))
        return;
    if (timestamp == 0)
        timestamp = meta_display_get_current_time_roundtrip(display);

    /* Actions that act on the session, not a specific window. */
    if (!g_strcmp0(action, "workspace")) {
        switch_workspace(display, arg, timestamp);
        return;
    }
    if (!g_strcmp0(action, "focus")) {
        cycle_focus(display, window, arg, timestamp);
        return;
    }
    if (!g_strcmp0(action, "switcher")) {
        gnoblin_switcher_cycle(display, !g_strcmp0(arg, "prev"), timestamp);
        return;
    }
    if (!g_strcmp0(action, "focus-monitor")) {
        focus_monitor(display, arg, timestamp);
        return;
    }
    if (!g_strcmp0(action, "spawn")) {
        g_autoptr(GError) error = NULL;

        /* A gnoblin on-demand client (launcher/osd) lands on the active monitor
         * via the inherited GNOBLIN_ACTIVE_OUTPUT; other commands ignore it. */
        gnoblin_export_active_output(display);
        if (arg && !g_spawn_command_line_async(arg, &error))
            g_warning("gnoblin: spawn '%s' failed: %s", arg, error->message);
        return;
    }
    if (!g_strcmp0(action, "lock")) {
        gnoblin_lock_engage(display);
        return;
    }
    if (!g_strcmp0(action, "overview")) {
        gnoblin_overview_toggle(display);
        return;
    }
    if (!g_strcmp0(action, "scratchpad")) {
        scratchpad_toggle(display, arg, timestamp);
        return;
    }
    if (!g_strcmp0(action, "gesture")) {
        if (arg)
            gnoblin_gestures_trigger(display, arg, timestamp);
        return;
    }

    if (!window)
        return;

    if (!g_strcmp0(action, "scratchpad-stash")) {
        scratchpad_stash(window, arg);
        return;
    }

    if (!g_strcmp0(action, "close"))
        meta_window_delete(window, timestamp);
    else if (!g_strcmp0(action, "maximize")) {
        if (meta_window_is_maximized(window))
            meta_window_unmaximize(window);
        else
            meta_window_maximize(window);
    } else if (!g_strcmp0(action, "minimize"))
        meta_window_minimize(window);
    else if (!g_strcmp0(action, "fullscreen")) {
        if (meta_window_is_fullscreen(window))
            meta_window_unmake_fullscreen(window);
        else
            meta_window_make_fullscreen(window);
    } else if (!g_strcmp0(action, "always-on-top")) {
        if (meta_window_is_above(window))
            meta_window_unmake_above(window);
        else
            meta_window_make_above(window);
    } else if (!g_strcmp0(action, "always-on-visible-workspace")) {
        if (meta_window_is_on_all_workspaces(window))
            meta_window_unstick(window);
        else
            meta_window_stick(window);
    } else if (!g_strcmp0(action, "move"))
        meta_window_begin_grab_op(window, META_GRAB_OP_KEYBOARD_MOVING, NULL, timestamp, NULL);
    else if (!g_strcmp0(action, "resize"))
        meta_window_begin_grab_op(window, META_GRAB_OP_KEYBOARD_RESIZING_UNKNOWN, NULL, timestamp,
                                  NULL);
    else if (!g_strcmp0(action, "move-to-workspace")) {
        int idx;
        if (gnoblin_actions_parse_workspace_index(arg, &idx))
            meta_window_change_workspace_by_index(window, idx, FALSE);
        else
            g_warning("gnoblin: move-to-workspace action needs a 1-based workspace index, got '%s'",
                      arg);
    } else if (!g_strcmp0(action, "snap"))
        snap_window(window, arg);
    else if (!g_strcmp0(action, "tile"))
        tile_window(window, arg);
    else if (!g_strcmp0(action, "center"))
        center_window(window);
    else if (!g_strcmp0(action, "move-to-monitor"))
        move_to_monitor(display, window, arg);
    else if (!g_strcmp0(action, "swap"))
        swap_window(display, window, timestamp);
    else if (!g_strcmp0(action, "opacity")) {
        MetaWindowActor* wa = actor_for_window(display, window);
        int percent;
        if (wa && gnoblin_actions_parse_percent(arg, &percent))
            clutter_actor_set_opacity(CLUTTER_ACTOR(wa), (guint8)(percent * 255 / 100));
        else if (arg)
            g_warning("gnoblin: opacity action needs a percentage, got '%s'", arg);
    } else if (!g_strcmp0(action, "window-menu")) {
        /* Pop the window menu for this window via the configured role. Anchor at
         * the window's top-left; the client clamps it on-screen. */
        MtkRectangle r;
        meta_window_get_frame_rect(window, &r);
        gnoblin_role_spawn("window-menu", window, r.x, r.y, "action");
    } else
        g_warning("gnoblin: unknown action '%s'", action);
}
