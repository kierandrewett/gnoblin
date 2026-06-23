/*
 * gnoblin-shell: real "background blur" — a content-behind blur effect.
 *
 * Unlike a scene-graph clone (which can only mirror one sibling sub-tree and
 * recurses if it sits inside the group it clones), this effect captures the
 * pixels ALREADY PAINTED into the stage framebuffer within the actor's bounds —
 * wallpaper AND any windows stacked underneath — blurs that captured texture
 * (separable Gaussian, configurable radius), masks it to the actor's rounded-
 * rect silhouette (so the frost matches the corner style), composites it behind
 * the actor, then paints the actor on top. This is the standard KWin/picom/
 * Hyprland background-blur technique. Being a Cogl/GLSL effect it renders on the
 * software (llvmpipe) devkit too, so it is visible headlessly.
 *
 * Attach the effect DIRECTLY to the window/surface actor (it reads the
 * framebuffer region the actor occupies and draws its own backdrop) — there is
 * no separate clone actor to manage.
 *
 * This program is free software; you can redistribute it and/or modify it
 * under the terms of the GNU General Public License as published by the Free
 * Software Foundation; either version 2 of the License, or (at your option)
 * any later version.
 */

#pragma once

extern "C" {
#include <clutter/clutter.h>
}

#include "gnoblin-rounded.h"

/* A new background-blur effect with `radius` (logical px). Attach it to the
 * actor whose backdrop should be frosted. */
ClutterEffect* gnoblin_blur_new(float radius);

/* Mask the blurred backdrop to this rounded-rect shape so the frost follows the
 * window's corners. Pass NULL (or never call) for a plain rectangular frost. */
void gnoblin_blur_set_rounded(ClutterEffect* effect, const GnoblinRoundedParams* params);

/* Frost only the surface's translucent body pixels: the frost is applied where
 * the surface's own alpha is BELOW `threshold` (in [0,1]) and high enough to be
 * part of the panel/window body rather than a low-alpha client-side shadow. E.g.
 * 0.9 frosts pixels that are at least ~10% transparent and shows near-opaque
 * pixels directly with no wasted blur. A threshold >= 1.0 (the default) keeps the
 * upper-alpha gate open. */
void gnoblin_blur_set_alpha_threshold(ClutterEffect* effect, float threshold);
