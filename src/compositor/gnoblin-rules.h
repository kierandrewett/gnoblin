/*
 * gnoblin-shell: per-application window rules + resolved visual-effect set.
 *
 * Match a freshly-mapped window against the [window-rules] config and apply
 * placement/state (float, size, position, centre, workspace, monitor, sticky,
 * always-on-top, maximize, fullscreen) plus decoration/opacity hints. Rules are
 * gnoblin's answer to sway's `for_window` / hyprland's `windowrulev2`.
 *
 * On top of the placement actions, rules carry COMPOSITOR-MANAGED VISUAL EFFECTS
 * — corner rounding (radius, algorithm, smoothing), borders (width, colour,
 * style incl. the macOS "lip"), and blur — layered over global `[appearance]`
 * defaults. gnoblin_rules_effects() resolves the effective set for a window so
 * the plugin can attach the right shaders. The same path serves toplevels and
 * wlr-layer-shell surfaces (matched with `layer=<namespace>`).
 *
 * Config (one `rule` per line — matchers `|` actions, all matchers must hit):
 *
 *     [window-rules]
 *     rule = app-id=org.gnome.Calculator | float, size 400x500, workspace 2
 *     rule = title=Picture-in-Picture | sticky, above, opacity 90, no-shadow
 *     rule = class=foot | rounding 16, corner-style squircle, smoothing 0.6
 *     rule = layer=gnoblin-dock | rounding 18, border 1 #ffffff20, border-style lip
 *
 * Matchers: app-id=, class=, title=, layer=. `key=value` is a case-insensitive
 * substring; `key~=pattern` is a case-insensitive GLib/PCRE regex (e.g.
 * `class~=^firefox$`, `layer~=^gnoblin-`).
 * Placement/state actions: float, size WxH, position X,Y, center, workspace N,
 *   monitor N, sticky, above, maximize, fullscreen, minimize, opacity N,
 *   inactive-opacity N, no-shadow, no-round.
 * Effect actions: rounding N, corner-style circular|squircle, smoothing F,
 *   border W [#color], border-style line|lip, no-border, blur [N], no-blur.
 *
 * This program is free software; you can redistribute it and/or modify it
 * under the terms of the GNU General Public License as published by the Free
 * Software Foundation; either version 2 of the License, or (at your option)
 * any later version.
 */

#pragma once

#include <glib.h>

#include "gnoblin-rounded.h"

extern "C" {
#include <meta/window.h>
}

G_BEGIN_DECLS

/* The fully-resolved visual-effect set for one window/surface: global
 * `[appearance]`/`[effects]` defaults with any matching per-rule overrides
 * applied on top. The plugin reads this to attach the rounded/border/blur
 * shaders. */
typedef struct {
    /* Rounding + border (drawn by the gnoblin-rounded shader). */
    gboolean rounding_enabled;
    GnoblinRoundedParams rounded; /* radius/algorithm/smoothing/border_* */

    /* Compositor-side blur-behind. */
    gboolean blur_enabled;
    float blur_radius;
    /* Frost only where the surface's own alpha is below this cutoff ([0,1]);
     * 1.0 (default) frosts wherever the surface has any coverage. */
    float blur_alpha_threshold;

    /* Drop shadow (the gnoblin-shadow box-shadow path). */
    gboolean shadow_enabled;

    /* Keep effects while maximized/fullscreen (off by default to match today). */
    gboolean keep_rounded_for_maximized;
    gboolean keep_rounded_for_fullscreen;

    /* Popup surface: the compositor installs a modal grab on map and dismisses
     * (closes) it on an outside button-press or Escape — so on-demand surfaces
     * (menus, the launcher, popouts) can be content-sized and let the compositor
     * own their chrome instead of hand-rolling a full-screen catcher in Slint. */
    gboolean is_popup;
} GnoblinEffects;

/* Match `window` against [window-rules] and apply its geometry/workspace/state
 * actions. Decoration + opacity + effect hints are cached on the window for the
 * plugin to read via the queries below. Call once, at map time. */
void gnoblin_rules_apply(MetaWindow* window);

/* Resolve the effective visual-effect set for `window`: `[appearance]`/`[effects]`
 * defaults overlaid with any matching rule's effect actions. Always callable
 * (applies rules lazily if needed). `out` is filled in full. */
void gnoblin_rules_effects(MetaWindow* window, GnoblinEffects* out);

/* Per-window decoration hints resolved by gnoblin_rules_apply (FALSE if no rule
 * matched or set them). The plugin consults these when adding effects. */
gboolean gnoblin_rules_no_round(MetaWindow* window);
gboolean gnoblin_rules_no_shadow(MetaWindow* window);

/* The target window opacity in percent (0-100) given its focus state, or -1 if
 * no rule set an opacity for this window. `focused` picks active vs
 * inactive-opacity. */
int gnoblin_rules_opacity(MetaWindow* window, gboolean focused);

/* The wlr-layer-shell namespace of `window` if it is a layer surface, else NULL.
 * Used by the `layer=` matcher and by the plugin to decide whether a layer
 * surface should get gnoblin-managed effects (panels ship bare). */
const char* gnoblin_rules_layer_namespace(MetaWindow* window);

G_END_DECLS
