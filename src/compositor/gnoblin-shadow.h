/*
 * gnoblin-shell: soft, multi-layer window drop shadows as a ClutterShaderEffect.
 *
 * Each layer is one CSS `box-shadow` (offset-x, offset-y, blur, rgba colour);
 * the effect composites up to four of them, so an elevation shadow like
 *   0 18px 36px rgba(0,0,0,.16), 0 2px 8px rgba(0,0,0,.10)
 * renders faithfully. The falloff is an erf-like smoothstep (50% at the window
 * edge, fading to 0 at `blur` px out) — a real soft blur, not a flat box.
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

#include "gnoblin-shadow-spec.h"

/* A composited drop shadow. `pad` is the actor's margin around the buffer on
 * each side (must hold each layer's |offset| + blur + positive spread),
 * `radius` the visible frame's corner radius, `frame_margin_*` the visible
 * frame's inset inside the buffer, and `layers`/`count` the stacked box-shadow
 * layers composited front-to-back. */
ClutterEffect* gnoblin_shadow_new(float pad, float radius, float frame_margin_left,
                                  float frame_margin_top, float frame_margin_right,
                                  float frame_margin_bottom,
                                  const GnoblinShadowLayer* layers, int count);
