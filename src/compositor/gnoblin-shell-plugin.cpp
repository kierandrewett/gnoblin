/*
 * gnoblin-shell: a minimal libmutter MetaPlugin — the gnoblin compositor.
 *
 * This is the gnoblin-shell *spike* (Phase 2): a from-scratch compositor that
 * embeds libmutter directly, instead of running gnome-shell. It is modelled on
 * mutter's in-tree minimal plugin (src/tests/meta-test-shell.c) and on
 * gnome-kiosk. A MetaPlugin is mandatory — mutter will not run without one, and
 * gnome-shell is simply mutter's default plugin.
 *
 * Phase 3's compositor effects layer lives here, as overrides of the MetaPlugin
 * effect vtable. Implemented: window open (map) / close (destroy) / minimize /
 * unminimize / size-change / workspace-switch animations (config-driven via
 * gnoblin-anim), each with correct completion + cancellation semantics; the
 * window context menu (show_window_menu); the drag-to-edge tile preview; a
 * solid per-monitor fallback background; opt-in rounded corners
 * (gnoblin-rounded); and opt-in soft drop shadows (gnoblin-shadow). Wallpaper
 * images are rendered by the gnoblin-wallpaper background layer-shell client.
 *
 * This program is free software; you can redistribute it and/or modify it
 * under the terms of the GNU General Public License as published by the Free
 * Software Foundation; either version 2 of the License, or (at your option)
 * any later version.
 */

#include "gnoblin-shell-plugin.h"

#include <cairo.h>
#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

extern "C" {
#include <clutter/clutter.h>
#include <meta/compositor.h>
#include <meta/display.h>
#include <meta/meta-backend.h>
#include <meta/meta-background-actor.h>
#include <meta/meta-background-content.h>
#include <meta/meta-background-group.h>
#include <meta/meta-background.h>
#include <meta/meta-context.h>
#include <meta/meta-monitor-manager.h>
#include <meta/meta-window-actor.h>
#include <meta/window.h>
#include <meta/workspace.h>
}

#include "gnoblin-actions-spec.h"
#include "gnoblin-anim.h"
#include "gnoblin-color-spec.h"
#include "gnoblin-config.h"
#include "gnoblin-control.h"
#include "gnoblin-roles.h"
#include "gnoblin-blur.h"
#include "gnoblin-rules.h"
#include "gnoblin-shadow.h"
#include "gnoblin-rounded.h"

struct _GnoblinShellPlugin {
    MetaPlugin parent;
    ClutterActor* background_group;
    /* In-flight workspace-switch slide: temporary per-workspace groups. */
    ClutterActor* ws_incoming;
    ClutterActor* ws_outgoing;
    ClutterTimeline* ws_timeline;
    /* Reused highlight shown while dragging a window to a snap edge. */
    ClutterActor* tile_preview;
};

G_DEFINE_TYPE(GnoblinShellPlugin, gnoblin_shell_plugin, META_TYPE_PLUGIN)

/* The single live plugin instance, so the config-file watch can ask us to
 * re-apply appearance without threading the pointer through every call. */
static GnoblinShellPlugin* the_plugin;
static const char* GNOBLIN_SHADOW_DATA_KEY = "gnoblin-shadow";

/* ---- keep drop-shadows OUT of the background-blur capture ----
 *
 * The blur effect (gnoblin-blur) frosts a surface by reading back the framebuffer
 * region already painted BEHIND it. Drop-shadows are sibling actors painted just
 * under each window, so a shadow lying behind a frosted surface (e.g. a window's
 * shadow falling behind a frosted panel, or a window's own penumbra under its
 * translucent edge) gets captured and smeared into the frost.
 *
 * Fix: render shadows in a SECOND pass that the blur capture never sees. On the
 * stage's `before-paint` we hide every shadow (opacity_override 0) so the window
 * group — and therefore every blur capture taken during it — is shadow-free. On
 * `after-paint` (still before the frame is displayed) we restore them and paint
 * each shadow straight into the view, clipped to the area NOT covered by any
 * window stacked ABOVE the shadow's own window, so shadows still sit crisply
 * under their windows and over lower ones, just never inside a frost. */
static GHashTable* gnoblin_shadows; /* set of live shadow ClutterActors */
static gulong gnoblin_before_paint_id;
static gulong gnoblin_after_paint_id;

static void register_shadow_actor(ClutterActor* shadow) {
    if (!gnoblin_shadows)
        gnoblin_shadows = g_hash_table_new(g_direct_hash, g_direct_equal);
    g_hash_table_add(gnoblin_shadows, shadow);
}

static void unregister_shadow_actor(ClutterActor* shadow) {
    if (gnoblin_shadows)
        g_hash_table_remove(gnoblin_shadows, shadow);
}

/* A solid background actor per monitor. Without this, regions not covered by a
 * window are never repainted and windows leave smear/ghost trails on the stage.
 * Wallpaper images are intentionally not drawn here: gnoblin-wallpaper owns the
 * real desktop image as a wlr-layer-shell background surface. */
static CoglColor background_color(void) {
    g_autofree char* hex = gnoblin_config_get_string("appearance", "background");
    guint8 r = 0x1d, g = 0x1f, b = 0x21, a = 255;

    if (hex && !gnoblin_color_parse_hex(hex, &r, &g, &b, &a))
        g_warning("gnoblin: invalid [appearance] background colour '%s'", hex);

    return (CoglColor)COGL_COLOR_INIT(r, g, b, 255);
}

static void reload_backgrounds(GnoblinShellPlugin* self) {
    MetaPlugin* plugin = META_PLUGIN(self);
    MetaDisplay* display = meta_plugin_get_display(plugin);
    CoglColor color = background_color();
    int n, i;

    clutter_actor_destroy_all_children(self->background_group);

    n = meta_display_get_n_monitors(display);
    for (i = 0; i < n; i++) {
        ClutterActor* background_actor = meta_background_actor_new(display, i);
        MetaBackgroundContent* content =
            META_BACKGROUND_CONTENT(clutter_actor_get_content(background_actor));
        g_autoptr(MetaBackground) background = meta_background_new(display);
        MtkRectangle rect;

        meta_display_get_monitor_geometry(display, i, &rect);
        clutter_actor_set_position(background_actor, rect.x, rect.y);
        clutter_actor_set_size(background_actor, rect.width, rect.height);

        meta_background_set_color(background, &color);
        meta_background_content_set_background(content, background);

        clutter_actor_add_child(self->background_group, background_actor);
    }
}

void gnoblin_shell_plugin_reload_appearance(void) {
    if (the_plugin)
        reload_backgrounds(the_plugin);
}

static void on_monitors_changed(MetaMonitorManager* monitor_manager, gpointer user_data) {
    reload_backgrounds(GNOBLIN_SHELL_PLUGIN(user_data));
}

/* Per-window-actor in-flight animation state, so kill_window_effects can cancel
 * an animation and have its "stopped" handler fire the completion callback. */
typedef struct {
    ClutterTimeline* map_timeline;
    ClutterTimeline* destroy_timeline;
    ClutterTimeline* resize_timeline;
    ClutterTimeline* minimize_timeline;
    gboolean resize_frozen;
    /* GNOME-style maximize/restore cross-fade. `resize_active` is held between
     * the size_change (prepare) and the matching completion. `resize_clone` is a
     * freeze-frame of the pre-resize contents that fades out as the live actor
     * grows/shrinks into the new frame. */
    gboolean resize_active;
    ClutterActor* resize_clone;
    MtkRectangle resize_old_frame;
    MetaSizeChange resize_change;
    ClutterActor* orig_parent; /* during a workspace switch, to restore reparenting */
} GnoblinActorState;

static GQuark actor_state_quark;

static GnoblinActorState* get_actor_state(MetaWindowActor* actor) {
    GnoblinActorState* state;

    if (G_UNLIKELY(actor_state_quark == 0))
        actor_state_quark = g_quark_from_static_string("gnoblin-actor-state");

    state = (GnoblinActorState*)g_object_get_qdata(G_OBJECT(actor), actor_state_quark);
    if (!state) {
        state = g_new0(GnoblinActorState, 1);
        g_object_set_qdata_full(G_OBJECT(actor), actor_state_quark, state, g_free);
    }

    return state;
}

static GnoblinActorState* peek_actor_state(MetaWindowActor* actor) {
    if (G_UNLIKELY(actor_state_quark == 0))
        return NULL;

    return (GnoblinActorState*)g_object_get_qdata(G_OBJECT(actor), actor_state_quark);
}

typedef struct {
    MetaPlugin* plugin;
    MetaWindowActor* window_actor;
} EffectData;

static ClutterTimeline* animate(ClutterActor* actor, ClutterAnimationMode mode, guint duration_ms,
                                const char* first_property, ...) {
    va_list args;
    ClutterTransition* transition;

    clutter_actor_save_easing_state(actor);
    clutter_actor_set_easing_mode(actor, mode);
    clutter_actor_set_easing_duration(actor, duration_ms);

    va_start(args, first_property);
    g_object_set_valist(G_OBJECT(actor), first_property, args);
    va_end(args);

    transition = clutter_actor_get_transition(actor, first_property);
    clutter_actor_restore_easing_state(actor);

    return transition ? CLUTTER_TIMELINE(transition) : NULL;
}

/* Which window types get gnoblin's rounded corners + drop shadow. Beyond plain
 * toplevels this covers dialogs and the menu/popup family, so right-click menus,
 * combo dropdowns and dialogs are decorated like the rest of the shell. Docks,
 * the desktop, tooltips, DND surfaces and notifications (which carry their own
 * styling) are left alone. */
static gboolean wants_decoration(MetaWindow* window) {
    switch (meta_window_get_window_type(window)) {
    case META_WINDOW_NORMAL:
    case META_WINDOW_DIALOG:
    case META_WINDOW_MODAL_DIALOG:
    case META_WINDOW_MENU:
    case META_WINDOW_DROPDOWN_MENU:
    case META_WINDOW_POPUP_MENU:
    case META_WINDOW_UTILITY:
    case META_WINDOW_COMBO:
        return TRUE;
    default:
        return FALSE;
    }
}

/* Whether to consider this window/surface for gnoblin-managed visual effects
 * (rounding/border/blur). Ordinary decorated windows always qualify. A
 * wlr-layer-shell surface qualifies too — but only so a rule can opt it in:
 * panels ship with no built-in effects and let gnoblin manage them via a
 * `layer=<namespace>` rule. We accept layer surfaces here and let the resolved
 * effect set (which is the bare global default unless a rule matched the
 * namespace) decide whether anything is actually drawn. */
static gboolean wants_effects(MetaWindow* window) {
    if (!window)
        return FALSE;
    if (wants_decoration(window))
        return TRUE;
    /* Layer surfaces carry a namespace; only those are eligible (and then only
     * if a rule targets them — the default effect set leaves them bare). */
    return gnoblin_rules_layer_namespace(window) != NULL;
}

static const char* shadow_window_type_name(MetaWindow* window) {
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
    default:
        return NULL;
    }
}

static gboolean menu_like_shadow_type(MetaWindow* window) {
    switch (meta_window_get_window_type(window)) {
    case META_WINDOW_MENU:
    case META_WINDOW_DROPDOWN_MENU:
    case META_WINDOW_POPUP_MENU:
    case META_WINDOW_COMBO:
        return TRUE;
    default:
        return FALSE;
    }
}

/* libadwaita / libhandy clients (Nautilus, most GNOME apps) draw their OWN
 * rounded corners and shadow. Rounding them again in the compositor double-rounds
 * the edge — the client's arc and ours don't coincide, leaving a transparent gap
 * in the corners (exactly what Kieran saw on Nautilus). Like Rounded Window
 * Corners Reborn, detect those toolkits by scanning the client's mapped
 * libraries and skip compositor rounding for them by default (they already look
 * rounded). Result cached on the window. Override with
 * `[effects] round-self-rounding-apps = on` to force our rounding/ring anyway. */
static gboolean window_is_self_rounding(MetaWindow* window) {
    gpointer cached;
    gboolean self_round = FALSE;
    pid_t pid;

    if (!window)
        return FALSE;
    cached = g_object_get_data(G_OBJECT(window), "gnoblin-self-rounding");
    if (cached)
        return GPOINTER_TO_INT(cached) == 1;

    pid = meta_window_get_pid(window);
    if (pid > 0) {
        g_autofree char* path = g_strdup_printf("/proc/%d/maps", (int)pid);
        g_autofree char* contents = NULL;

        if (g_file_get_contents(path, &contents, NULL, NULL) && contents &&
            (strstr(contents, "libadwaita-1.so") || strstr(contents, "libhandy-1.so")))
            self_round = TRUE;
    }
    /* Cache as 1 (yes) / 2 (no) so the 0 "unset" sentinel is distinct. */
    g_object_set_data(G_OBJECT(window), "gnoblin-self-rounding",
                      GINT_TO_POINTER(self_round ? 1 : 2));
    return self_round;
}

static void shadow_frame_margins(MetaWindow* window, float margins[4]) {
    MtkRectangle frame;
    MtkRectangle buffer;

    margins[0] = margins[1] = margins[2] = margins[3] = 0.0f;
    if (!window)
        return;

    meta_window_get_frame_rect(window, &frame);
    meta_window_get_buffer_rect(window, &buffer);
    if (frame.width <= 0 || frame.height <= 0 || buffer.width <= 0 || buffer.height <= 0)
        return;

    margins[0] = (float)MAX(0, frame.x - buffer.x);
    margins[1] = (float)MAX(0, frame.y - buffer.y);
    margins[2] = (float)MAX(0, (buffer.x + buffer.width) - (frame.x + frame.width));
    margins[3] = (float)MAX(0, (buffer.y + buffer.height) - (frame.y + frame.height));
}

static gboolean wants_window_open_animation(MetaWindow* window) {
    if (!window)
        return FALSE;

    /* Map/open effects are safe for stable toplevels and short-lived menus.
     * Menus use tighter defaults in gnoblin-anim.cpp, so context menus and
     * dropdowns get a small entrance without feeling like full app windows. */
    switch (meta_window_get_window_type(window)) {
    case META_WINDOW_NORMAL:
    case META_WINDOW_DIALOG:
    case META_WINDOW_MODAL_DIALOG:
    case META_WINDOW_MENU:
    case META_WINDOW_DROPDOWN_MENU:
    case META_WINDOW_POPUP_MENU:
    case META_WINDOW_UTILITY:
    case META_WINDOW_COMBO:
        return TRUE;
    default:
        return FALSE;
    }
}

static gboolean wants_window_close_animation(MetaWindow* window) {
    if (!window)
        return FALSE;

    /* Keep destroy effects to stable toplevel-style windows. Raw popup/menu
     * actors can be synchronously dismissed by Mutter during protocol
     * validation; running a close transition there can complete after the
     * MetaWindow has already entered disposal. */
    switch (meta_window_get_window_type(window)) {
    case META_WINDOW_NORMAL:
    case META_WINDOW_DIALOG:
    case META_WINDOW_MODAL_DIALOG:
    case META_WINDOW_UTILITY:
        return TRUE;
    default:
        return FALSE;
    }
}

/* After any stacking change, pin each window's shadow directly below its window
 * actor. The shadow is a plain sibling in the window group and so does not take
 * part in mutter's restacking; without this a context menu (which restacks its
 * app) could leave the app's shadow painting in front of it. */
static void restack_shadows(MetaDisplay* display, gpointer user_data) {
    MetaCompositor* compositor = meta_display_get_compositor(display);
    ClutterActor* window_group = meta_compositor_get_window_group(compositor);
    GList* children = clutter_actor_get_children(window_group);
    GList* l;

    for (l = children; l; l = l->next) {
        ClutterActor* child = CLUTTER_ACTOR(l->data);
        ClutterActor* shadow;

        if (!META_IS_WINDOW_ACTOR(child))
            continue;
        shadow = (ClutterActor*)g_object_get_data(G_OBJECT(child), GNOBLIN_SHADOW_DATA_KEY);
        if (shadow && clutter_actor_get_parent(shadow) == window_group)
            clutter_actor_set_child_below_sibling(window_group, shadow, child);
    }

    g_list_free(children);
}

static ClutterActor* get_actor_shadow(ClutterActor* actor) {
    return (ClutterActor*)g_object_get_data(G_OBJECT(actor), GNOBLIN_SHADOW_DATA_KEY);
}

static void clear_actor_shadow_if_current(ClutterActor* actor, ClutterActor* shadow) {
    if (get_actor_shadow(actor) == shadow)
        g_object_set_data(G_OBJECT(actor), GNOBLIN_SHADOW_DATA_KEY, NULL);
}

static void on_shadow_destroyed(ClutterActor* shadow, ClutterActor* actor) {
    unregister_shadow_actor(shadow);
    clear_actor_shadow_if_current(actor, shadow);
}

static void destroy_actor_shadow(ClutterActor* actor, gpointer user_data) {
    ClutterActor* shadow = get_actor_shadow(actor);

    if (!shadow)
        return;

    g_object_ref(shadow);
    clear_actor_shadow_if_current(actor, shadow);
    clutter_actor_destroy(shadow);
    g_object_unref(shadow);
}

static void reset_actor_transform(ClutterActor* actor) {
    if (!actor)
        return;

    clutter_actor_set_pivot_point(actor, 0.0, 0.0);
    clutter_actor_set_scale(actor, 1.0, 1.0);
    clutter_actor_set_translation(actor, 0.0, 0.0, 0.0);
}

static void reset_resize_transforms(MetaWindowActor* window_actor) {
    ClutterActor* actor = CLUTTER_ACTOR(window_actor);

    reset_actor_transform(actor);
    reset_actor_transform(get_actor_shadow(actor));
}

static gboolean rect_is_usable(const MtkRectangle* r) {
    return r && r->width > 0 && r->height > 0;
}

static MtkRectangle inflate_rect(const MtkRectangle* r, int pad_x, int pad_y) {
    MtkRectangle inflated = *r;
    inflated.x -= pad_x;
    inflated.y -= pad_y;
    inflated.width += 2 * pad_x;
    inflated.height += 2 * pad_y;
    return inflated;
}

static void apply_rect_lerp_start(ClutterActor* actor, const MtkRectangle* old_rect,
                                  const MtkRectangle* new_rect, double pivot_stage_x,
                                  double pivot_stage_y) {
    double sx, sy, px, py, tx, ty;
    double pivot_x, pivot_y;

    if (!actor || !rect_is_usable(old_rect) || !rect_is_usable(new_rect))
        return;

    sx = (double)old_rect->width / new_rect->width;
    sy = (double)old_rect->height / new_rect->height;
    px = CLAMP((pivot_stage_x - new_rect->x) / new_rect->width, 0.0, 1.0);
    py = CLAMP((pivot_stage_y - new_rect->y) / new_rect->height, 0.0, 1.0);
    pivot_x = px * new_rect->width;
    pivot_y = py * new_rect->height;
    tx = (old_rect->x - new_rect->x) - pivot_x * (1.0 - sx);
    ty = (old_rect->y - new_rect->y) - pivot_y * (1.0 - sy);

    clutter_actor_set_pivot_point(actor, px, py);
    clutter_actor_set_scale(actor, sx, sy);
    clutter_actor_set_translation(actor, tx, ty, 0.0);
}

static void animate_rect_lerp_to_identity(ClutterActor* actor, const GnoblinAnim* anim) {
    if (!actor || !anim)
        return;

    animate(actor, anim->mode, anim->duration_ms, "scale-x", 1.0, "scale-y", 1.0,
            "translation-x", 0.0, "translation-y", 0.0, NULL);
}

static const char* size_change_effect_name(MetaSizeChange which_change) {
    switch (which_change) {
    case META_SIZE_CHANGE_MAXIMIZE:
    case META_SIZE_CHANGE_FULLSCREEN:
        return "maximize";
    case META_SIZE_CHANGE_UNMAXIMIZE:
    case META_SIZE_CHANGE_UNFULLSCREEN:
        return "unmaximize";
    case META_SIZE_CHANGE_MONITOR_MOVE:
    default:
        return "resize";
    }
}

static void apply_size_change_shadow_start(ClutterActor* shadow, const MtkRectangle* old_frame,
                                           const MtkRectangle* new_frame,
                                           double pivot_stage_x, double pivot_stage_y) {
    float sw = 0.0f, sh = 0.0f;
    int pad_x, pad_y;
    MtkRectangle old_shadow, new_shadow;

    if (!shadow || !rect_is_usable(old_frame) || !rect_is_usable(new_frame))
        return;

    clutter_actor_get_size(shadow, &sw, &sh);
    pad_x = MAX(0, (int)lround(((double)sw - new_frame->width) / 2.0));
    pad_y = MAX(0, (int)lround(((double)sh - new_frame->height) / 2.0));
    old_shadow = inflate_rect(old_frame, pad_x, pad_y);
    new_shadow = inflate_rect(new_frame, pad_x, pad_y);
    apply_rect_lerp_start(shadow, &old_shadow, &new_shadow, pivot_stage_x, pivot_stage_y);
}

/* Attach the rounded-corner + border shader to a window/surface from its
 * resolved effect set (global `[appearance]`/`[effects]` defaults overlaid with
 * matching rule overrides). Opt-in: with rounding/border unset the shader never
 * runs, so it can't affect anyone who hasn't enabled it. The effect carries the
 * radius, corner algorithm + smoothing, and the border (width/colour/style incl.
 * the macOS "lip"). */
static void maybe_round_corners(MetaWindowActor* window_actor) {
    ClutterActor* actor = CLUTTER_ACTOR(window_actor);
    MetaWindow* window = meta_window_actor_get_meta_window(window_actor);
    GnoblinEffects fx;

    if (!wants_effects(window))
        return;

    gnoblin_rules_effects(window, &fx);
    if (!fx.rounding_enabled)
        return;
    if (clutter_actor_get_effect(actor, "gnoblin-rounded"))
        return;
    /* Clients that round themselves (libadwaita/libhandy) leave their own
     * transparent corners; rounding them naively double-rounds into a gap. Rather
     * than skip them (no ring at all), round them WITH smart corner-fill: the
     * shader fills the client's transparent corner with its edge colour so our
     * rounded corner + ring read cleanly. `round-self-rounding-apps = off` opts
     * back out (skip entirely). */
    if (window_is_self_rounding(window)) {
        if (!gnoblin_config_get_bool("effects", "round-self-rounding-apps", TRUE))
            return;
        fx.rounded.corner_fill = TRUE;
    }

    /* Inset the mask/border/ring to the visible surface inside any CSD shadow
     * margin (same margins the drop-shadow uses), so the rounded edge hugs the
     * real window rather than the buffer edge out past the shadow. */
    shadow_frame_margins(window, fx.rounded.content_inset);

    clutter_actor_add_effect_with_name(actor, "gnoblin-rounded",
                                       gnoblin_rounded_new_full(&fx.rounded));
}

/* Compositor-side background blur-behind. Attached DIRECTLY to the window/surface
 * actor as a gnoblin-blur ClutterEffect: at paint time it captures the
 * framebuffer region already painted behind the actor (wallpaper AND any windows
 * stacked under it), blurs that with a separable Gaussian, masks it to the
 * actor's rounded-rect silhouette, draws it, then paints the actor on top (see
 * gnoblin-blur.cpp). Real content-behind frost — not a wallpaper-only clone.
 * Opt-in per rule / global `[effects] blur`; on by default for gnoblin chrome. */
static const char* GNOBLIN_BLUR_EFFECT_NAME = "gnoblin-blur";

static ClutterEffect* get_actor_blur(ClutterActor* actor) {
    return clutter_actor_get_effect(actor, GNOBLIN_BLUR_EFFECT_NAME);
}

static void maybe_add_blur(MetaPlugin* plugin, MetaWindowActor* window_actor) {
    ClutterActor* actor = CLUTTER_ACTOR(window_actor);
    MetaWindow* window = meta_window_actor_get_meta_window(window_actor);
    GnoblinEffects fx;
    ClutterEffect* blur;

    (void)plugin;
    if (!wants_effects(window))
        return;
    gnoblin_rules_effects(window, &fx);
    if (!fx.blur_enabled || fx.blur_radius <= 0)
        return;
    if (get_actor_blur(actor))
        return;

    blur = gnoblin_blur_new(fx.blur_radius);
    /* Frost the corners to match the window's rounding. */
    if (fx.rounding_enabled)
        gnoblin_blur_set_rounded(blur, &fx.rounded);
    /* Frost only the surface's translucent pixels (alpha < threshold). */
    gnoblin_blur_set_alpha_threshold(blur, fx.blur_alpha_threshold);

    /* The blur effect must run BEFORE the rounded-corner effect so the frost is
     * masked to the same silhouette and the actor still gets its corner mask.
     * Effects paint in insertion order, and the rounded effect (if any) is added
     * after this, so a plain add keeps the blur first. */
    clutter_actor_add_effect_with_name(actor, GNOBLIN_BLUR_EFFECT_NAME, blur);
}

/* Drop a configurable soft shadow behind decorated windows. The shadow is
 * defined as a CSS box-shadow string — any number of comma-separated `offset-x
 * offset-y blur [spread] colour` layers — so no shadow geometry is baked into
 * the compositor. A `shadow.<window-type>` key can override/disable a type.
 * GTK menus/popovers usually ship their own CSS shadow in transparent buffer
 * margins, so compositor shadows are opt-in for menu-like transient types. */
static void maybe_add_shadow(MetaPlugin* plugin, MetaWindowActor* window_actor) {
    ClutterActor* actor = CLUTTER_ACTOR(window_actor);
    MetaWindow* window = meta_window_actor_get_meta_window(window_actor);
    int radius = gnoblin_config_get_int("appearance", "rounding", 0);
    CoglColor black = (CoglColor)COGL_COLOR_INIT(0, 0, 0, 255);
    static gboolean restack_hooked = FALSE;
    MetaDisplay* display;
    MetaCompositor* compositor;
    ClutterActor* window_group;
    ClutterActor* shadow;
    g_autofree char* spec = NULL;
    GnoblinShadowLayer layers[GNOBLIN_SHADOW_MAX_LAYERS];
    int nlayers = 0;
    float pad = 0.0f;
    float margins[4] = {0, 0, 0, 0};

    if (!wants_decoration(window) || gnoblin_rules_no_shadow(window))
        return;
    if (get_actor_shadow(actor))
        return;
    /* The flush, edge-to-edge top bar has no rounding and no drop shadow (a
     * GNOME-style bar); a rounded shadow at the screen edge just reads as a gap
     * around it. */
    {
        const char* ns = gnoblin_rules_layer_namespace(window);
        if (ns && g_strcmp0(ns, "gnoblin-topbar") == 0)
            return;
    }

    if (const char* type_name = shadow_window_type_name(window)) {
        g_autofree char* key = g_strdup_printf("shadow.%s", type_name);
        spec = gnoblin_config_get_string("appearance", key);
    }
    if (!spec && !menu_like_shadow_type(window))
        spec = gnoblin_config_get_string("appearance", "shadow");
    if (spec && *spec)
        nlayers = gnoblin_shadow_parse_box_shadow(spec, layers, GNOBLIN_SHADOW_MAX_LAYERS);

    if (nlayers == 0)
        return;

    /* `pad` must hold each layer's blur plus its offset so nothing is clipped. */
    pad = gnoblin_shadow_pad_for_layers(layers, nlayers);
    shadow_frame_margins(window, margins);

    display = meta_plugin_get_display(plugin);
    compositor = meta_display_get_compositor(display);
    window_group = meta_compositor_get_window_group(compositor);

    if (!restack_hooked) {
        g_signal_connect(display, "restacked", G_CALLBACK(restack_shadows), NULL);
        restack_hooked = TRUE;
    }

    shadow = clutter_actor_new();
    clutter_actor_set_background_color(shadow, &black);
    clutter_actor_add_constraint(shadow,
                                 clutter_bind_constraint_new(actor, CLUTTER_BIND_X, -pad));
    clutter_actor_add_constraint(shadow,
                                 clutter_bind_constraint_new(actor, CLUTTER_BIND_Y, -pad));
    clutter_actor_add_constraint(shadow,
                                 clutter_bind_constraint_new(actor, CLUTTER_BIND_WIDTH, 2 * pad));
    clutter_actor_add_constraint(
        shadow, clutter_bind_constraint_new(actor, CLUTTER_BIND_HEIGHT, 2 * pad));
    clutter_actor_add_effect(shadow, gnoblin_shadow_new(pad, (float)radius, margins[0],
                                                        margins[1], margins[2], margins[3],
                                                        layers, nlayers));
    clutter_actor_insert_child_below(window_group, shadow, actor);

    /* Keep a back-reference so restack_shadows() can re-pin it under its window
     * after any stacking change (e.g. a context menu opening). */
    g_object_set_data_full(G_OBJECT(actor), GNOBLIN_SHADOW_DATA_KEY, g_object_ref(shadow),
                           g_object_unref);

    /* Track it so the before/after-paint passes can keep it out of the blur, and
     * remember which window it belongs to so that pass can clip it correctly. */
    g_object_set_data(G_OBJECT(shadow), "gnoblin-shadow-window-actor", actor);
    register_shadow_actor(shadow);

    /* Follow the window's visibility so a minimized/hidden window doesn't leave
     * an orphan shadow floating on the stage. */
    g_object_bind_property(actor, "visible", shadow, "visible",
                           (GBindingFlags)(G_BINDING_DEFAULT | G_BINDING_SYNC_CREATE));

    /* The shadow lives and dies with its window actor, but window-group teardown
     * can destroy child actors in sibling order. Keep the stored pointer valid
     * while it exists, clear it if the shadow is destroyed first, and make the
     * actor destroy path idempotent. */
    g_signal_connect(actor, "destroy", G_CALLBACK(destroy_actor_shadow), NULL);
    g_signal_connect_object(shadow, "destroy", G_CALLBACK(on_shadow_destroyed), actor,
                            (GConnectFlags)0);
}

/* Fully-maximized and fullscreen windows don't need rounded corners (they'd be
 * clipped at the screen edge) or a drop shadow (nothing shows behind them), and
 * running those offscreen shader passes every damaged frame is wasted GPU. Mute
 * them while maximized/fullscreen and restore them otherwise. Called at map and
 * on every size change (maximize / unmaximize / fullscreen / tile / resize). */
static void update_window_effects(MetaWindowActor* window_actor) {
    ClutterActor* actor = CLUTTER_ACTOR(window_actor);
    MetaWindow* window = meta_window_actor_get_meta_window(window_actor);
    gboolean maximized, fullscreen, suppress_round, suppress;
    ClutterEffect* rounded;
    ClutterActor* shadow;
    ClutterEffect* blur;
    GnoblinEffects fx;

    if (!window)
        return;
    maximized = meta_window_is_maximized(window);
    fullscreen = meta_window_is_fullscreen(window);
    suppress = maximized || fullscreen;

    /* Rounded corners are normally dropped while maximized/fullscreen (they'd be
     * clipped at the screen edge and waste a shader pass per frame). The
     * keep-rounded-for-maximized / -fullscreen toggles override that per the
     * resolved effect set. */
    gnoblin_rules_effects(window, &fx);
    suppress_round = suppress;
    if (maximized && fx.keep_rounded_for_maximized)
        suppress_round = FALSE;
    if (fullscreen && fx.keep_rounded_for_fullscreen)
        suppress_round = FALSE;

    rounded = clutter_actor_get_effect(actor, "gnoblin-rounded");
    if (rounded)
        clutter_actor_meta_set_enabled(CLUTTER_ACTOR_META(rounded), !suppress_round);

    /* The shadow follows the window's `visible` via a g_object binding, so steer
     * it by opacity instead (independent of that binding) to avoid a fight. */
    shadow = get_actor_shadow(actor);
    if (shadow)
        clutter_actor_set_opacity(shadow, suppress ? 0 : 255);

    /* A maximized/fullscreen window has nothing showing behind it, so the
     * blur-behind capture is wasted (and would frost a fully covered backdrop) —
     * disable the effect while suppressed. */
    blur = get_actor_blur(actor);
    if (blur)
        clutter_actor_meta_set_enabled(CLUTTER_ACTOR_META(blur), !suppress);
}

/* ---- per-window opacity (window rules: opacity / inactive-opacity) ---- */

/* Set a window actor's opacity from its rule, given whether it's focused. A
 * window with no opacity rule is left fully opaque. */
static void apply_rule_opacity(MetaWindowActor* window_actor, gboolean focused) {
    GnoblinActorState* state;
    ClutterActor* actor;
    MetaWindow* window = meta_window_actor_get_meta_window(window_actor);
    int pct;
    guint8 opacity;

    if (!window)
        return;

    pct = gnoblin_rules_opacity(window, focused);
    if (pct < 0)
        return;

    state = peek_actor_state(window_actor);
    if (state && (state->map_timeline || state->destroy_timeline || state->minimize_timeline))
        return;

    actor = CLUTTER_ACTOR(window_actor);
    opacity = (guint8)(pct * 255 / 100);
    if (clutter_actor_get_opacity(actor) == opacity)
        return;

    /* Rule opacity is state, not an effect. Apply it immediately so a focus
     * notification during window unmanage cannot create or remove implicit
     * Clutter opacity transitions on a disposing actor. */
    clutter_actor_save_easing_state(actor);
    clutter_actor_set_easing_duration(actor, 0);
    clutter_actor_set_opacity(actor, opacity);
    clutter_actor_restore_easing_state(actor);
}

/* Swap the RING border to its focused/unfocused colours (no-op for other styles
 * or windows without a rounded effect). */
static void apply_border_focus(MetaWindowActor* window_actor, gboolean focused) {
    ClutterEffect* fx = clutter_actor_get_effect(CLUTTER_ACTOR(window_actor), "gnoblin-rounded");
    if (fx)
        gnoblin_rounded_set_focused(fx, focused);
}

/* On focus change, refresh active/inactive opacity + RING border for every window. */
static void on_focus_window_changed(MetaDisplay* display, GParamSpec* pspec, gpointer user_data) {
    MetaCompositor* compositor = meta_display_get_compositor(display);
    MetaWindow* focused = meta_display_get_focus_window(display);
    GList* l;

    for (l = meta_compositor_get_window_actors(compositor); l; l = l->next) {
        MetaWindowActor* wa = META_WINDOW_ACTOR(l->data);
        gboolean is_focused = meta_window_actor_get_meta_window(wa) == focused;
        apply_rule_opacity(wa, is_focused);
        apply_border_focus(wa, is_focused);
    }
}

/* ---- layer-shell surface effects ----
 *
 * wlr-layer-shell surfaces (the shell's own panels/dock/etc.) are backed by real
 * MetaWindows of type DESKTOP/DOCK, but mutter does NOT run the MetaPlugin map
 * effect for them — so they never pass through gnoblin_shell_plugin_map. To let
 * gnoblin manage their rounding/border/blur via `layer=<namespace>` rules, hook
 * the display's window-created signal and apply the resolved effect set to any
 * layer surface (identified by its namespace) once its actor exists. Panels ship
 * bare; only a matching rule draws anything. */
static void apply_layer_surface_effects(MetaWindow* window) {
    GObject* priv;
    MetaWindowActor* window_actor;

    if (!window || !gnoblin_rules_layer_namespace(window))
        return;

    priv = meta_window_get_compositor_private(window);
    if (!priv || !META_IS_WINDOW_ACTOR(priv))
        return;
    window_actor = META_WINDOW_ACTOR(priv);

    gnoblin_rules_apply(window);
    maybe_add_blur(META_PLUGIN(the_plugin), window_actor);
    maybe_round_corners(window_actor);
    update_window_effects(window_actor);
}

static void on_window_created(MetaDisplay* display, MetaWindow* window, gpointer user_data) {
    apply_layer_surface_effects(window);
}

/* ---- map (window open) ---- */

static void on_map_stopped(ClutterTimeline* timeline, gboolean is_finished, gpointer user_data) {
    EffectData* data = (EffectData*)user_data;
    GnoblinActorState* state = get_actor_state(data->window_actor);

    state->map_timeline = NULL;
    meta_plugin_map_completed(data->plugin, data->window_actor);
    g_free(data);
}

static void gnoblin_shell_plugin_map(MetaPlugin* plugin, MetaWindowActor* window_actor) {
    ClutterActor* actor = CLUTTER_ACTOR(window_actor);
    MetaWindow* window = meta_window_actor_get_meta_window(window_actor);
    GnoblinActorState* state;
    EffectData* data;
    GnoblinAnim anim = gnoblin_anim_get_for_window("open", window);

    /* Apply [window-rules] first so its no-round/no-shadow/opacity hints are set
     * before we add decorations. */
    gnoblin_rules_apply(window);
    maybe_add_blur(plugin, window_actor); /* below the actor, before rounding/shadow */
    maybe_round_corners(window_actor);
    maybe_add_shadow(plugin, window_actor);
    update_window_effects(window_actor); /* a window can map already-maximized */
    apply_rule_opacity(window_actor,
                       window == meta_display_get_focus_window(meta_plugin_get_display(plugin)));

    if (!anim.enabled || !wants_window_open_animation(window)) {
        meta_plugin_map_completed(plugin, window_actor);
        return;
    }

    clutter_actor_set_pivot_point(actor, 0.5, 0.5);
    clutter_actor_set_opacity(actor, 0);
    clutter_actor_set_scale(actor, anim.scale, anim.scale);
    clutter_actor_show(actor);

    state = get_actor_state(window_actor);
    state->map_timeline = animate(actor, anim.mode, anim.duration_ms, "opacity", 255, "scale-x",
                                  1.0, "scale-y", 1.0, NULL);

    if (!state->map_timeline) {
        meta_plugin_map_completed(plugin, window_actor);
        return;
    }

    data = g_new0(EffectData, 1);
    data->plugin = plugin;
    data->window_actor = window_actor;
    g_signal_connect(state->map_timeline, "stopped", G_CALLBACK(on_map_stopped), data);
}

/* ---- destroy (window close) ---- */

static void on_destroy_stopped(ClutterTimeline* timeline, gboolean is_finished,
                               gpointer user_data) {
    EffectData* data = (EffectData*)user_data;
    GnoblinActorState* state = get_actor_state(data->window_actor);

    state->destroy_timeline = NULL;
    meta_plugin_destroy_completed(data->plugin, data->window_actor);
    g_free(data);
}

static void gnoblin_shell_plugin_destroy(MetaPlugin* plugin, MetaWindowActor* window_actor) {
    ClutterActor* actor = CLUTTER_ACTOR(window_actor);
    MetaWindow* window = meta_window_actor_get_meta_window(window_actor);
    GnoblinActorState* state;
    EffectData* data;
    GnoblinAnim anim = gnoblin_anim_get_for_window("close", window);

    if (!anim.enabled || !wants_window_close_animation(window)) {
        meta_plugin_destroy_completed(plugin, window_actor);
        return;
    }

    clutter_actor_set_pivot_point(actor, 0.5, 0.5);

    state = get_actor_state(window_actor);
    state->destroy_timeline = animate(actor, anim.mode, anim.duration_ms, "opacity", 0, "scale-x",
                                      anim.scale, "scale-y", anim.scale, NULL);

    if (!state->destroy_timeline) {
        meta_plugin_destroy_completed(plugin, window_actor);
        return;
    }

    data = g_new0(EffectData, 1);
    data->plugin = plugin;
    data->window_actor = window_actor;
    g_signal_connect(state->destroy_timeline, "stopped", G_CALLBACK(on_destroy_stopped), data);
}

/* ---- minimize / unminimize ---- */

static void on_minimize_stopped(ClutterTimeline* timeline, gboolean is_finished,
                                gpointer user_data) {
    EffectData* data = (EffectData*)user_data;
    GnoblinActorState* state = get_actor_state(data->window_actor);
    ClutterActor* actor = CLUTTER_ACTOR(data->window_actor);

    state->minimize_timeline = NULL;
    /* mutter hides the actor once minimized; restore its scale so a later
     * unminimize/map starts from a sane transform. */
    clutter_actor_set_scale(actor, 1.0, 1.0);
    clutter_actor_set_opacity(actor, 255);
    meta_plugin_minimize_completed(data->plugin, data->window_actor);
    g_free(data);
}

static void gnoblin_shell_plugin_minimize(MetaPlugin* plugin, MetaWindowActor* window_actor) {
    ClutterActor* actor = CLUTTER_ACTOR(window_actor);
    MetaWindow* window = meta_window_actor_get_meta_window(window_actor);
    GnoblinAnim anim = gnoblin_anim_get_for_window("minimize", window);
    GnoblinActorState* state;
    EffectData* data;

    if (!anim.enabled || meta_window_get_window_type(window) != META_WINDOW_NORMAL) {
        meta_plugin_minimize_completed(plugin, window_actor);
        return;
    }

    clutter_actor_set_pivot_point(actor, 0.5, 1.0); /* shrink toward the bottom */

    state = get_actor_state(window_actor);
    state->minimize_timeline = animate(actor, anim.mode, anim.duration_ms, "opacity", 0, "scale-x",
                                       anim.scale, "scale-y", anim.scale, NULL);

    if (!state->minimize_timeline) {
        meta_plugin_minimize_completed(plugin, window_actor);
        return;
    }

    data = g_new0(EffectData, 1);
    data->plugin = plugin;
    data->window_actor = window_actor;
    g_signal_connect(state->minimize_timeline, "stopped", G_CALLBACK(on_minimize_stopped), data);
}

static void on_unminimize_stopped(ClutterTimeline* timeline, gboolean is_finished,
                                  gpointer user_data) {
    EffectData* data = (EffectData*)user_data;
    GnoblinActorState* state = get_actor_state(data->window_actor);

    state->minimize_timeline = NULL;
    meta_plugin_unminimize_completed(data->plugin, data->window_actor);
    g_free(data);
}

static void gnoblin_shell_plugin_unminimize(MetaPlugin* plugin, MetaWindowActor* window_actor) {
    ClutterActor* actor = CLUTTER_ACTOR(window_actor);
    MetaWindow* window = meta_window_actor_get_meta_window(window_actor);
    GnoblinAnim anim = gnoblin_anim_get_for_window("open", window); /* grow back like opening */
    GnoblinActorState* state;
    EffectData* data;

    if (!anim.enabled || meta_window_get_window_type(window) != META_WINDOW_NORMAL) {
        meta_plugin_unminimize_completed(plugin, window_actor);
        return;
    }

    clutter_actor_set_pivot_point(actor, 0.5, 1.0);
    clutter_actor_set_opacity(actor, 0);
    clutter_actor_set_scale(actor, anim.scale, anim.scale);
    clutter_actor_show(actor);

    state = get_actor_state(window_actor);
    state->minimize_timeline = animate(actor, anim.mode, anim.duration_ms, "opacity", 255,
                                       "scale-x", 1.0, "scale-y", 1.0, NULL);

    if (!state->minimize_timeline) {
        clutter_actor_set_scale(actor, 1.0, 1.0);
        clutter_actor_set_opacity(actor, 255);
        meta_plugin_unminimize_completed(plugin, window_actor);
        return;
    }

    data = g_new0(EffectData, 1);
    data->plugin = plugin;
    data->window_actor = window_actor;
    g_signal_connect(state->minimize_timeline, "stopped", G_CALLBACK(on_unminimize_stopped), data);
}

/* ---- size_change (maximize / unmaximize / fullscreen / tile) ----
 *
 * This mirrors GNOME Shell's two-phase size-change animation, which is what
 * makes maximize/restore feel smooth instead of stretchy. Mutter calls us twice:
 *
 *   size_change  (prepare): the window still has its old geometry and contents.
 *                 We grab a freeze-frame of the old contents into a clone and
 *                 freeze the live actor so the client's relayout is held back.
 *   size_changed (run):     the window now sits at its new geometry. We grow the
 *                 live actor from the old frame into the new one and, in lockstep,
 *                 grow + fade out the old-contents clone over the top. The
 *                 cross-fade hides the fact that the client's layout reflows
 *                 between the two sizes, so there is no squash/stretch artifact.
 *
 * Crucially the live actor is thawed at the start of the run phase, not the end,
 * so its real new-size contents stream in *during* the grow. Waiting until the
 * animation finished would scale the stale old texture and then hard-snap. */

static gboolean wants_size_change_animation(MetaWindow* window) {
    /* Match GNOME: only plain toplevels get the frame zoom. Dialogs, menus and
     * the transient family either don't maximize or look wrong scaled. */
    return window && meta_window_get_window_type(window) == META_WINDOW_NORMAL;
}

/* Tear down any in-flight size-change state for this actor, optionally firing the
 * matching completion exactly once. Idempotent: a second call (e.g. the timeline
 * "stopped" handler re-entering after we stop it) is a no-op. */
static void finish_size_change(MetaWindowActor* window_actor, gboolean complete) {
    GnoblinActorState* state = peek_actor_state(window_actor);
    ClutterActor* actor = CLUTTER_ACTOR(window_actor);

    if (!state || !state->resize_active)
        return;
    state->resize_active = FALSE;

    if (state->resize_timeline) {
        g_autoptr(ClutterTimeline) timeline = (ClutterTimeline*)g_object_ref(state->resize_timeline);
        state->resize_timeline = NULL;
        clutter_timeline_stop(timeline);
    }

    clutter_actor_remove_all_transitions(actor);
    if (ClutterActor* shadow = get_actor_shadow(actor))
        clutter_actor_remove_all_transitions(shadow);
    reset_resize_transforms(window_actor);

    if (state->resize_clone) {
        clutter_actor_destroy(state->resize_clone);
        state->resize_clone = NULL;
    }

    if (state->resize_frozen) {
        state->resize_frozen = FALSE;
        meta_window_actor_thaw(window_actor);
    }

    /* Now that the window has settled at its final frame, refresh rounded-corner
     * + shadow suppression. We deferred this from size_change (prepare) so the
     * live actor kept its rounded corners throughout the grow/shrink; here a
     * maximized/fullscreen window finally drops them (square, full-screen) while a
     * restored window keeps them. */
    update_window_effects(window_actor);

    if (complete)
        meta_plugin_size_change_completed(META_PLUGIN(the_plugin), window_actor);
}

static void on_size_change_stopped(ClutterTimeline* timeline, gboolean is_finished,
                                   gpointer user_data) {
    finish_size_change(META_WINDOW_ACTOR(user_data), TRUE);
}

/* If the window actor is destroyed mid-animation, drop the clone and our state
 * without touching the disposing actor (no thaw/complete on a dying actor). */
static void on_resize_actor_destroyed(ClutterActor* actor, gpointer user_data) {
    GnoblinActorState* state = peek_actor_state(META_WINDOW_ACTOR(actor));

    if (!state || !state->resize_active)
        return;
    state->resize_active = FALSE;
    state->resize_frozen = FALSE;
    state->resize_timeline = NULL;
    if (state->resize_clone) {
        clutter_actor_destroy(state->resize_clone);
        state->resize_clone = NULL;
    }
}

static void gnoblin_shell_plugin_size_change(MetaPlugin* plugin, MetaWindowActor* window_actor,
                                             MetaSizeChange which_change,
                                             MtkRectangle* old_frame_rect,
                                             MtkRectangle* old_buffer_rect,
                                             MtkRectangle* target_frame_rect) {
    ClutterActor* actor = CLUTTER_ACTOR(window_actor);
    MetaWindow* window = meta_window_actor_get_meta_window(window_actor);
    GnoblinAnim anim = gnoblin_anim_get_for_window(size_change_effect_name(which_change), window);
    GnoblinActorState* state;
    ClutterContent* content;
    ClutterActor* clone;
    ClutterActor* parent;
    GError* error = NULL;

    /* Close out anything still animating for this actor (mutter increments its
     * size-change counter per call, so a prior prepare must complete). We do NOT
     * refresh rounded-corner/shadow suppression yet: when an animation runs we
     * want the live actor to keep its rounded corners while it grows/shrinks, and
     * only drop them once the window has settled at its final (maximized) frame.
     * The suppression is therefore deferred to finish_size_change(). On the
     * non-animated paths below we apply it immediately, as before. */
    finish_size_change(window_actor, TRUE);

    if (!anim.enabled || !wants_size_change_animation(window) || !rect_is_usable(old_frame_rect)) {
        update_window_effects(window_actor);
        meta_plugin_size_change_completed(plugin, window_actor);
        return;
    }

    /* If the frame isn't actually changing size, the run phase (size_changed)
     * would never fire — completing here avoids leaving the actor frozen. */
    if (rect_is_usable(target_frame_rect) && target_frame_rect->width == old_frame_rect->width &&
        target_frame_rect->height == old_frame_rect->height) {
        update_window_effects(window_actor);
        meta_plugin_size_change_completed(plugin, window_actor);
        return;
    }

    parent = clutter_actor_get_parent(actor);
    if (!parent) {
        update_window_effects(window_actor);
        meta_plugin_size_change_completed(plugin, window_actor);
        return;
    }

    /* Capture the window's current (pre-resize) contents, clipped to the old
     * frame, as a static freeze-frame. */
    content = meta_window_actor_paint_to_content(window_actor, old_frame_rect, &error);
    if (!content) {
        g_clear_error(&error);
        meta_plugin_size_change_completed(plugin, window_actor);
        return;
    }

    clone = clutter_actor_new();
    clutter_actor_set_content(clone, content);
    g_object_unref(content);
    clutter_actor_set_offscreen_redirect(clone, CLUTTER_OFFSCREEN_REDIRECT_ALWAYS);
    clutter_actor_set_position(clone, old_frame_rect->x, old_frame_rect->y);
    clutter_actor_set_size(clone, old_frame_rect->width, old_frame_rect->height);

    /* The freeze-frame is a plain actor with no decorations, so it would grow
     * with SQUARE corners while the (rounded) old contents are still visible.
     * Clip it with the same rounded-corner shader the live actor uses, but only
     * when the captured contents were themselves rounded — i.e. rounding is on
     * and the window is currently rounded (the live actor still has the enabled
     * effect at this point, since we defer suppression to completion). The effect
     * is scale-aware, so the clone's radius reads constant as it scales up. */
    if (ClutterEffect* live_rounded = clutter_actor_get_effect(actor, "gnoblin-rounded")) {
        if (clutter_actor_meta_get_enabled(CLUTTER_ACTOR_META(live_rounded))) {
            int radius = gnoblin_config_get_int("appearance", "rounding", 0);
            if (radius > 0)
                clutter_actor_add_effect(clone, gnoblin_rounded_new(radius));
        }
    }

    /* The window is GROWING to a square, full-screen state, so the LIVE actor
     * should grow SQUARE — not keep its rounded corners + ring through the whole
     * animation and snap square at the end (the "ring breaks while maximizing"
     * Kieran saw). Disable the live rounded effect now for a maximize/fullscreen
     * target; the freeze-frame clone above still carries the OLD rounded contents
     * for the cross-fade. Restore (unmaximize/unfullscreen) is left rounded so it
     * shrinks back into a rounded window. finish_size_change() reconciles the
     * final state either way. */
    if (which_change == META_SIZE_CHANGE_MAXIMIZE ||
        which_change == META_SIZE_CHANGE_FULLSCREEN) {
        if (ClutterEffect* lr = clutter_actor_get_effect(actor, "gnoblin-rounded"))
            clutter_actor_meta_set_enabled(CLUTTER_ACTOR_META(lr), FALSE);
    }

    clutter_actor_add_child(parent, clone);
    clutter_actor_set_child_above_sibling(parent, clone, actor);

    /* Hold the live actor on its old contents until the run phase thaws it. */
    meta_window_actor_freeze(window_actor);

    state = get_actor_state(window_actor);
    state->resize_active = TRUE;
    state->resize_frozen = TRUE;
    state->resize_clone = clone;
    state->resize_old_frame = *old_frame_rect;
    state->resize_change = which_change;
    g_signal_connect_object(actor, "destroy", G_CALLBACK(on_resize_actor_destroyed), clone,
                            (GConnectFlags)0);

    /* Do not complete here: gnoblin_shell_plugin_size_changed runs the animation
     * once mutter has applied the new geometry. */
}

static void gnoblin_shell_plugin_size_changed(MetaPlugin* plugin, MetaWindowActor* window_actor) {
    ClutterActor* actor = CLUTTER_ACTOR(window_actor);
    ClutterActor* shadow = get_actor_shadow(actor);
    MetaWindow* window = meta_window_actor_get_meta_window(window_actor);
    GnoblinActorState* state = peek_actor_state(window_actor);
    GnoblinAnim anim;
    MtkRectangle target;
    MtkRectangle* source;
    ClutterActor* clone;
    double sx, sy, tx, ty;

    /* Only act on a size change we prepared; plain resizes/moves also land here. */
    if (!state || !state->resize_active)
        return;

    clone = state->resize_clone;
    source = &state->resize_old_frame;
    meta_window_get_frame_rect(window, &target);

    if (!rect_is_usable(&target) || !rect_is_usable(source) ||
        (target.width == source->width && target.height == source->height)) {
        finish_size_change(window_actor, TRUE);
        return;
    }

    anim = gnoblin_anim_get_for_window(size_change_effect_name(state->resize_change), window);

    /* `[animations] maximize-style` selects how the old-contents clone hands off
     * to the live window as both scale between frames:
     *  - zoom (default, macOS): the clone holds opaque and reads as a single image
     *    zooming, then dissolves through the back half (ease-in-out) so the live,
     *    correct-size content is revealed while the frame is still settling, and
     *    the fade lands on opacity 0 with zero slope — a smooth settle, not a pop.
     *  - crossfade (GNOME): the clone fades out early (front-loaded), so the live
     *    reflowed layout blends in across the whole grow. */
    g_autofree char* style = gnoblin_config_get_string("animations", "maximize-style");
    gboolean crossfade_style = style && !g_ascii_strcasecmp(g_strstrip(style), "crossfade");
    ClutterAnimationMode clone_fade_mode =
        crossfade_style ? CLUTTER_EASE_OUT_QUAD : CLUTTER_EASE_IN_OUT_QUART;

    /* The actor sits at `target`; start it scaled/translated to overlay `source`,
     * then ease back to identity. */
    sx = (double)source->width / target.width;
    sy = (double)source->height / target.height;
    tx = (double)source->x - target.x;
    ty = (double)source->y - target.y;

    clutter_actor_set_pivot_point(actor, 0.0, 0.0);
    clutter_actor_set_scale(actor, sx, sy);
    clutter_actor_set_translation(actor, tx, ty, 0.0);

    /* The shadow tracks the same path (it is inflated past the frame by the blur
     * pad, so lerp its own rect rather than share the actor transform). */
    if (shadow) {
        apply_size_change_shadow_start(shadow, source, &target, target.x, target.y);
        animate_rect_lerp_to_identity(shadow, &anim);
    }

    /* The clone starts overlaying `source` and tracks the same grow as the actor
     * (shared curve); its opacity runs on its own curve to pick the feel above. */
    if (clone) {
        clutter_actor_set_pivot_point(clone, 0.0, 0.0);
        animate(clone, anim.mode, anim.duration_ms, "x", (double)target.x, "y", (double)target.y,
                "scale-x", 1.0 / sx, "scale-y", 1.0 / sy, NULL);
        animate(clone, clone_fade_mode, anim.duration_ms, "opacity", 0, NULL);
    }

    state->resize_timeline = animate(actor, anim.mode, anim.duration_ms, "scale-x", 1.0, "scale-y",
                                     1.0, "translation-x", 0.0, "translation-y", 0.0, NULL);

    /* Thaw now (not on completion) so the real new-size contents render under the
     * fading clone instead of a stale, stretched old texture. */
    if (state->resize_frozen) {
        state->resize_frozen = FALSE;
        meta_window_actor_thaw(window_actor);
    }

    if (!state->resize_timeline) {
        finish_size_change(window_actor, TRUE);
        return;
    }
    g_signal_connect(state->resize_timeline, "stopped", G_CALLBACK(on_size_change_stopped),
                     window_actor);
}

/* ---- cancellation ---- */

static void gnoblin_shell_plugin_kill_window_effects(MetaPlugin* plugin,
                                                     MetaWindowActor* window_actor) {
    GnoblinActorState* state = get_actor_state(window_actor);

    /* Stopping a timeline emits "stopped", whose handler fires the matching
     * meta_plugin_*_completed and clears the pointer. Ref around stop so the
     * handler running synchronously cannot free state from under us. */
    if (state->map_timeline) {
        g_autoptr(ClutterTimeline) timeline = g_object_ref(state->map_timeline);
        clutter_timeline_stop(timeline);
    }
    if (state->destroy_timeline) {
        g_autoptr(ClutterTimeline) timeline = g_object_ref(state->destroy_timeline);
        clutter_timeline_stop(timeline);
    }
    if (state->resize_active)
        finish_size_change(window_actor, TRUE);
    if (state->minimize_timeline) {
        g_autoptr(ClutterTimeline) timeline = g_object_ref(state->minimize_timeline);
        clutter_timeline_stop(timeline);
    }
}

/* ---- switch_workspace (slide transition) ---- */

/* Reparent every window actor back to where it was before the slide, drop the
 * temporary groups, and tell mutter the switch is done. Runs from the incoming
 * group's "stopped" handler (natural finish or kill_switch_workspace). */
static void on_switch_workspace_stopped(ClutterTimeline* timeline, gboolean is_finished,
                                        gpointer user_data) {
    MetaPlugin* plugin = META_PLUGIN(user_data);
    GnoblinShellPlugin* self = GNOBLIN_SHELL_PLUGIN(plugin);
    MetaDisplay* display = meta_plugin_get_display(plugin);
    MetaCompositor* compositor = meta_display_get_compositor(display);
    GList* l;

    for (l = meta_compositor_get_window_actors(compositor); l; l = l->next) {
        MetaWindowActor* window_actor = META_WINDOW_ACTOR(l->data);
        GnoblinActorState* state = get_actor_state(window_actor);
        ClutterActor* actor = CLUTTER_ACTOR(window_actor);

        if (!state->orig_parent)
            continue;

        g_object_ref(actor);
        clutter_actor_remove_child(clutter_actor_get_parent(actor), actor);
        clutter_actor_add_child(state->orig_parent, actor);
        g_object_unref(actor);
        state->orig_parent = NULL;
    }

    g_clear_pointer(&self->ws_incoming, clutter_actor_destroy);
    g_clear_pointer(&self->ws_outgoing, clutter_actor_destroy);
    self->ws_timeline = NULL;

    meta_plugin_switch_workspace_completed(plugin);
}

static void gnoblin_shell_plugin_switch_workspace(MetaPlugin* plugin, gint from, gint to,
                                                  MetaMotionDirection direction) {
    GnoblinShellPlugin* self = GNOBLIN_SHELL_PLUGIN(plugin);
    GnoblinAnim anim = gnoblin_anim_get("workspace");
    MetaDisplay* display = meta_plugin_get_display(plugin);
    MetaCompositor* compositor = meta_display_get_compositor(display);
    ClutterActor* stage = CLUTTER_ACTOR(meta_compositor_get_stage(compositor));
    ClutterActor* incoming;
    ClutterActor* outgoing;
    GList* l;
    int width, height;
    double dx = 0.0, dy = 0.0;

    if (from == to || !anim.enabled) {
        meta_plugin_switch_workspace_completed(plugin);
        return;
    }

    meta_display_get_size(display, &width, &height);

    /* Temporary groups: the destination workspace's windows slide in, the
     * source workspace's windows slide out. Everything sticky/unassigned stays
     * put on the stage. */
    incoming = clutter_actor_new();
    outgoing = clutter_actor_new();
    clutter_actor_set_size(incoming, width, height);
    clutter_actor_set_size(outgoing, width, height);
    clutter_actor_add_child(stage, outgoing);
    clutter_actor_add_child(stage, incoming);

    for (l = meta_compositor_get_window_actors(compositor); l; l = l->next) {
        MetaWindowActor* window_actor = META_WINDOW_ACTOR(l->data);
        GnoblinActorState* state = get_actor_state(window_actor);
        ClutterActor* actor = CLUTTER_ACTOR(window_actor);
        MetaWindow* window = meta_window_actor_get_meta_window(window_actor);
        MetaWorkspace* workspace;
        ClutterActor* group;
        int index;

        state->orig_parent = NULL;

        /* The actor may be mid-unmanage (its MetaWindow already gone) — skip it
         * rather than deref NULL through meta_window_get_workspace(). */
        if (!window)
            continue;

        workspace = meta_window_get_workspace(window);
        if (!workspace || meta_window_is_on_all_workspaces(window))
            continue;

        index = meta_workspace_index(workspace);
        if (index != to && index != from)
            continue;

        group = (index == to) ? incoming : outgoing;
        state->orig_parent = clutter_actor_get_parent(actor);
        g_object_ref(actor);
        clutter_actor_remove_child(state->orig_parent, actor);
        clutter_actor_add_child(group, actor);
        g_object_unref(actor);
    }

    /* The destination slides in from the direction of travel; the source slides
     * the opposite way by the same amount. */
    switch (direction) {
    case META_MOTION_LEFT:
        dx = -width;
        break;
    case META_MOTION_RIGHT:
        dx = width;
        break;
    case META_MOTION_UP:
        dy = -height;
        break;
    case META_MOTION_DOWN:
        dy = height;
        break;
    default:
        dx = width;
        break;
    }

    clutter_actor_set_position(incoming, dx, dy);
    clutter_actor_set_position(outgoing, 0, 0);

    self->ws_incoming = incoming;
    self->ws_outgoing = outgoing;

    if (dx != 0.0) {
        animate(outgoing, anim.mode, anim.duration_ms, "x", -dx, NULL);
        self->ws_timeline = animate(incoming, anim.mode, anim.duration_ms, "x", 0.0, NULL);
    } else {
        animate(outgoing, anim.mode, anim.duration_ms, "y", -dy, NULL);
        self->ws_timeline = animate(incoming, anim.mode, anim.duration_ms, "y", 0.0, NULL);
    }

    if (!self->ws_timeline) {
        on_switch_workspace_stopped(NULL, TRUE, plugin);
        return;
    }

    g_signal_connect(self->ws_timeline, "stopped", G_CALLBACK(on_switch_workspace_stopped), plugin);
}

static void gnoblin_shell_plugin_kill_switch_workspace(MetaPlugin* plugin) {
    GnoblinShellPlugin* self = GNOBLIN_SHELL_PLUGIN(plugin);

    /* Stopping the timeline emits "stopped", which restores the reparented
     * windows and fires switch_workspace_completed. Ref around stop so the
     * synchronous handler cannot free our state mid-call. */
    if (self->ws_timeline) {
        g_autoptr(ClutterTimeline) timeline = g_object_ref(self->ws_timeline);
        clutter_timeline_stop(timeline);
    }
}

/* ---- window menu (the WM right-click / titlebar menu) ---- */

static void gnoblin_shell_plugin_show_window_menu(MetaPlugin* plugin, MetaWindow* window,
                                                  MetaWindowMenuType menu, int x, int y) {
    /* Only the window-manager menu; the app menu is the client's business. */
    if (menu != META_WINDOW_MENU_WM)
        return;

    /* The window menu is the `window-menu` role: a Slint client (or whatever the
     * config binds). No role configured → no window menu. No native fallback. */
    (void) plugin;
    gnoblin_role_spawn("window-menu", window, x, y, "titlebar");
}

static void gnoblin_shell_plugin_show_window_menu_for_rect(MetaPlugin* plugin, MetaWindow* window,
                                                           MetaWindowMenuType menu,
                                                           MtkRectangle* rect) {
    if (menu != META_WINDOW_MENU_WM)
        return;

    /* Anchor under the menu button / titlebar region. */
    (void) plugin;
    gnoblin_role_spawn("window-menu", window, rect->x, rect->y + rect->height, "titlebar");
}

/* ---- tile preview (drag-to-edge snap highlight) ---- */

/* The accent fill of the snap-preview rectangle: `[appearance] accent`
 * (`#rrggbb`/`#rrggbbaa`), default a translucent GNOME blue. */
static CoglColor accent_color(void) {
    g_autofree char* hex = gnoblin_config_get_string("appearance", "accent");
    guint8 r = 0x35, g = 0x84, b = 0xe4, a = 0x66;

    if (hex && !gnoblin_color_parse_hex(hex, &r, &g, &b, &a))
        g_warning("gnoblin: invalid [appearance] accent colour '%s'", hex);

    return (CoglColor)COGL_COLOR_INIT(r, g, b, a);
}

static ClutterActor* ensure_tile_preview(GnoblinShellPlugin* self) {
    MetaPlugin* plugin = META_PLUGIN(self);
    MetaCompositor* compositor = meta_display_get_compositor(meta_plugin_get_display(plugin));

    if (!self->tile_preview) {
        CoglColor fill = accent_color();

        self->tile_preview = clutter_actor_new();
        clutter_actor_set_background_color(self->tile_preview, &fill);
        clutter_actor_set_opacity(self->tile_preview, 0);
        clutter_actor_set_pivot_point(self->tile_preview, 0.5, 0.5);
        clutter_actor_hide(self->tile_preview);
        clutter_actor_add_child(meta_compositor_get_window_group(compositor), self->tile_preview);
    }

    return self->tile_preview;
}

static void on_tile_preview_faded(ClutterTimeline* timeline, gboolean is_finished,
                                  gpointer user_data) {
    ClutterActor* preview = CLUTTER_ACTOR(user_data);

    /* Only hide if we faded all the way out — a new show may have re-raised it. */
    if (clutter_actor_get_opacity(preview) == 0)
        clutter_actor_hide(preview);
}

static void gnoblin_shell_plugin_show_tile_preview(MetaPlugin* plugin, MetaWindow* window,
                                                   MtkRectangle* tile_rect, int tile_monitor) {
    GnoblinShellPlugin* self = GNOBLIN_SHELL_PLUGIN(plugin);
    MetaCompositor* compositor = meta_display_get_compositor(meta_plugin_get_display(plugin));
    ClutterActor* preview = ensure_tile_preview(self);
    GnoblinAnim anim = gnoblin_anim_get("tile");
    gboolean appearing = !clutter_actor_is_visible(preview);

    /* Keep the highlight on top of the window stack. */
    clutter_actor_set_child_above_sibling(meta_compositor_get_window_group(compositor), preview,
                                          NULL);
    clutter_actor_show(preview);

    if (appearing || !anim.enabled) {
        clutter_actor_set_position(preview, tile_rect->x, tile_rect->y);
        clutter_actor_set_size(preview, tile_rect->width, tile_rect->height);
        if (anim.enabled) {
            clutter_actor_set_scale(preview, anim.scale, anim.scale);
            animate(preview, anim.mode, anim.duration_ms, "opacity", 255, "scale-x", 1.0, "scale-y",
                    1.0, NULL);
        } else {
            clutter_actor_set_opacity(preview, 255);
        }
        return;
    }

    /* Already visible: glide to the new zone. */
    animate(preview, anim.mode, anim.duration_ms, "x", (double)tile_rect->x, "y",
            (double)tile_rect->y, "width", (double)tile_rect->width, "height",
            (double)tile_rect->height, "opacity", 255, NULL);
}

static void gnoblin_shell_plugin_hide_tile_preview(MetaPlugin* plugin) {
    GnoblinShellPlugin* self = GNOBLIN_SHELL_PLUGIN(plugin);
    GnoblinAnim anim = gnoblin_anim_get("tile");
    ClutterTimeline* timeline;

    if (!self->tile_preview || !clutter_actor_is_visible(self->tile_preview))
        return;

    if (!anim.enabled) {
        clutter_actor_hide(self->tile_preview);
        return;
    }

    timeline = animate(self->tile_preview, anim.mode, anim.duration_ms, "opacity", 0, NULL);
    if (timeline)
        g_signal_connect(timeline, "stopped", G_CALLBACK(on_tile_preview_faded),
                         self->tile_preview);
    else
        clutter_actor_hide(self->tile_preview);
}

/* ---- self-screenshot (devkit/headless visual validation) ---- */

/* Paint the whole stage into a buffer (the same clutter_stage_paint_to_buffer
 * path screencopy uses) and write a PNG via cairo — no external screencopy
 * client or output enumeration needed. Triggered by GNOBLIN_SHOT=<path>; with
 * GNOBLIN_SHOT_EXIT set the process quits after, so a short headless boot can
 * capture itself and leave the PNG on disk. */
static gboolean capture_stage_to_png(gpointer user_data) {
    g_autofree char* path = (char*)user_data;
    MetaDisplay* display;
    ClutterActor* stage;
    float fw = 0, fh = 0;
    int width, height, stride;
    unsigned char* data;
    g_autoptr(GError) error = NULL;
    MtkRectangle rect;

    if (!the_plugin)
        return G_SOURCE_REMOVE;

    display = meta_plugin_get_display(META_PLUGIN(the_plugin));
    stage = meta_backend_get_stage(meta_context_get_backend(meta_display_get_context(display)));

    clutter_actor_get_size(stage, &fw, &fh);
    width = (int)fw;
    height = (int)fh;
    if (width <= 0 || height <= 0)
        return G_SOURCE_REMOVE;

    stride = cairo_format_stride_for_width(CAIRO_FORMAT_ARGB32, width);
    data = (unsigned char*)g_malloc0((gsize)stride * height);
    rect = (MtkRectangle){0, 0, width, height};

    if (clutter_stage_paint_to_buffer(CLUTTER_STAGE(stage), &rect, 1.0, data, stride,
                                      COGL_PIXEL_FORMAT_CAIRO_ARGB32_COMPAT,
                                      CLUTTER_PAINT_FLAG_CLEAR, &error)) {
        cairo_surface_t* surface =
            cairo_image_surface_create_for_data(data, CAIRO_FORMAT_ARGB32, width, height, stride);
        cairo_surface_write_to_png(surface, path);
        cairo_surface_destroy(surface);
        g_message("gnoblin: wrote screenshot %s (%dx%d)", path, width, height);
    } else {
        g_warning("gnoblin: screenshot failed: %s", error ? error->message : "unknown");
    }

    g_free(data);

    if (g_getenv("GNOBLIN_SHOT_EXIT"))
        exit(0);

    return G_SOURCE_REMOVE;
}

static void maybe_schedule_screenshot(void) {
    const char* path = g_getenv("GNOBLIN_SHOT");
    const char* delay_env = g_getenv("GNOBLIN_SHOT_DELAY");
    guint delay = 3000;

    if (delay_env && !gnoblin_actions_parse_uint(delay_env, &delay))
        g_warning("gnoblin: invalid GNOBLIN_SHOT_DELAY '%s'", delay_env);

    if (path)
        g_timeout_add(delay, capture_stage_to_png, g_strdup(path));
}

/* ---- shadow second-pass (keep shadows out of the blur capture) ---- */

/* before-paint: hide every shadow for this frame's stage paint, so the window
 * group (and every blur capture taken mid-paint) is shadow-free. We re-add them
 * crisply in after-paint. */
static void on_stage_before_paint(ClutterStage* stage, ClutterStageView* view,
                                  ClutterFrame* frame, gpointer user_data) {
    GHashTableIter it;
    gpointer key;

    (void)stage;
    (void)view;
    (void)frame;
    (void)user_data;
    if (!gnoblin_shadows)
        return;
    g_hash_table_iter_init(&it, gnoblin_shadows);
    while (g_hash_table_iter_next(&it, &key, NULL))
        clutter_actor_set_opacity_override(CLUTTER_ACTOR(key), 0);
}

/* The exclusion region for one shadow: the view minus the frame rects of the
 * shadow's OWN window and every window stacked ABOVE it. A shadow must never
 * darken its own window (the window sits in front of its shadow) nor any window
 * above it; it should only fall on the wallpaper and lower windows. This is what
 * keeps the re-composited shadow looking exactly like a below-the-window
 * drop-shadow — and, crucially, keeps it OUT of a frosted surface in front. */
static MtkRegion* shadow_clip_region(ClutterActor* shadow, ClutterStageView* view) {
    MetaCompositor* compositor;
    ClutterActor* window_group;
    ClutterActor* my_window = (ClutterActor*)g_object_get_data(G_OBJECT(shadow),
                                                               "gnoblin-shadow-window-actor");
    MtkRectangle layout;
    MtkRegion* region;
    GList* children;
    GList* l;
    gboolean at_or_above = FALSE;

    if (!the_plugin || !my_window)
        return NULL;

    clutter_stage_view_get_layout(view, &layout);
    region = mtk_region_create_rectangle(&layout);

    compositor = meta_display_get_compositor(meta_plugin_get_display(META_PLUGIN(the_plugin)));
    window_group = meta_compositor_get_window_group(compositor);
    children = clutter_actor_get_children(window_group);

    /* Window group is bottom-to-top; my_window and everything after it is at or
     * above the shadow, so subtract all of those windows' rects. */
    for (l = children; l; l = l->next) {
        ClutterActor* child = CLUTTER_ACTOR(l->data);
        if (child == my_window)
            at_or_above = TRUE;
        if (!at_or_above || !META_IS_WINDOW_ACTOR(child))
            continue;
        if (!clutter_actor_is_mapped(child) || clutter_actor_get_opacity(child) == 0)
            continue;
        {
            MetaWindow* w = meta_window_actor_get_meta_window(META_WINDOW_ACTOR(child));
            MtkRectangle r;
            if (!w)
                continue;
            meta_window_get_frame_rect(w, &r);
            mtk_region_subtract_rectangle(region, &r);
        }
    }
    g_list_free(children);
    return region;
}

/* after-paint: restore the shadows and paint each one straight into the view's
 * framebuffer (still before display), clipped so it never darkens a window above
 * its own. Because no blur capture happens here, the frost stays shadow-free. */
static void on_stage_after_paint(ClutterStage* stage, ClutterStageView* view,
                                 ClutterFrame* frame, gpointer user_data) {
    CoglFramebuffer* fb;
    ClutterColorState* color_state;
    ClutterPaintContext* ctx;
    GHashTableIter it;
    gpointer key;

    (void)frame;
    (void)user_data;
    if (!gnoblin_shadows || g_hash_table_size(gnoblin_shadows) == 0)
        return;

    fb = clutter_stage_view_get_framebuffer(view);
    color_state = clutter_stage_view_get_color_state(view);
    if (!fb)
        return;

    ctx = clutter_paint_context_new_for_framebuffer(fb, NULL, CLUTTER_PAINT_FLAG_NONE,
                                                    color_state);

    g_hash_table_iter_init(&it, gnoblin_shadows);
    while (g_hash_table_iter_next(&it, &key, NULL)) {
        ClutterActor* shadow = CLUTTER_ACTOR(key);
        MtkRegion* clip;

        /* Restore the shadow so its paint draws normally (and so the next frame's
         * before-paint can re-hide it). */
        clutter_actor_set_opacity_override(shadow, -1);

        if (!clutter_actor_is_mapped(shadow) || clutter_actor_get_opacity(shadow) == 0)
            continue;

        clip = shadow_clip_region(shadow, view);
        if (clip) {
            cogl_framebuffer_push_region_clip(fb, clip);
            clutter_actor_paint(shadow, ctx);
            cogl_framebuffer_pop_clip(fb);
            mtk_region_unref(clip);
        } else {
            clutter_actor_paint(shadow, ctx);
        }
    }

    clutter_paint_context_destroy(ctx);
    (void)stage;
}

/* ---- lifecycle ---- */

static void gnoblin_shell_plugin_start(MetaPlugin* plugin) {
    GnoblinShellPlugin* self = GNOBLIN_SHELL_PLUGIN(plugin);
    MetaDisplay* display = meta_plugin_get_display(plugin);
    MetaCompositor* compositor = meta_display_get_compositor(display);
    MetaContext* context = meta_display_get_context(display);

    the_plugin = self;
    MetaBackend* backend = meta_context_get_backend(context);
    MetaMonitorManager* monitor_manager = meta_backend_get_monitor_manager(backend);

    /* Backdrop behind all windows, so the stage is always painted (no trails).
     */
    self->background_group = meta_background_group_new();
    clutter_actor_insert_child_below(meta_compositor_get_window_group(compositor),
                                     self->background_group, NULL);
    g_signal_connect(monitor_manager, "monitors-changed", G_CALLBACK(on_monitors_changed), plugin);
    reload_backgrounds(self);

    /* Config keybindings + the dev.gnoblin.Shell D-Bus action API. */
    gnoblin_control_init(display);

    /* Per-window rule opacity tracks the focused window (active/inactive). */
    g_signal_connect(display, "notify::focus-window", G_CALLBACK(on_focus_window_changed), NULL);

    /* Layer-shell surfaces (panels/dock) don't pass through the map effect; apply
     * their gnoblin-managed rounding/border/blur when they are created. */
    g_signal_connect(display, "window-created", G_CALLBACK(on_window_created), NULL);

    /* Keep drop-shadows out of the blur capture: hide them during the stage
     * paint (so blur captures are shadow-free) and re-composite them crisply,
     * clipped, in after-paint. See on_stage_before_paint / on_stage_after_paint. */
    {
        ClutterActor* stage = meta_backend_get_stage(backend);
        gnoblin_before_paint_id =
            g_signal_connect(stage, "before-paint", G_CALLBACK(on_stage_before_paint), NULL);
        gnoblin_after_paint_id =
            g_signal_connect(stage, "after-paint", G_CALLBACK(on_stage_after_paint), NULL);
    }

    /* Reveal the stage so windows are composited. */
    clutter_actor_show(meta_backend_get_stage(backend));

    /* Optional self-screenshot for headless visual validation. */
    maybe_schedule_screenshot();
}

static void gnoblin_shell_plugin_class_init(GnoblinShellPluginClass* klass) {
    MetaPluginClass* plugin_class = META_PLUGIN_CLASS(klass);

    plugin_class->start = gnoblin_shell_plugin_start;
    plugin_class->map = gnoblin_shell_plugin_map;
    plugin_class->destroy = gnoblin_shell_plugin_destroy;
    plugin_class->minimize = gnoblin_shell_plugin_minimize;
    plugin_class->unminimize = gnoblin_shell_plugin_unminimize;
    plugin_class->size_change = gnoblin_shell_plugin_size_change;
    plugin_class->size_changed = gnoblin_shell_plugin_size_changed;
    plugin_class->switch_workspace = gnoblin_shell_plugin_switch_workspace;
    plugin_class->show_window_menu = gnoblin_shell_plugin_show_window_menu;
    plugin_class->show_window_menu_for_rect = gnoblin_shell_plugin_show_window_menu_for_rect;
    plugin_class->show_tile_preview = gnoblin_shell_plugin_show_tile_preview;
    plugin_class->hide_tile_preview = gnoblin_shell_plugin_hide_tile_preview;
    plugin_class->kill_window_effects = gnoblin_shell_plugin_kill_window_effects;
    plugin_class->kill_switch_workspace = gnoblin_shell_plugin_kill_switch_workspace;
}

static void gnoblin_shell_plugin_init(GnoblinShellPlugin* plugin) {}
