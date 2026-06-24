/*
 * gnoblin-shell: rounded window corners as a ClutterShaderEffect.
 *
 * An opt-in offscreen shader effect that masks a window actor's corners with a
 * rounded-rectangle signed-distance field, and (optionally) strokes a border
 * along that same rounded edge. Attached per-window/-surface by the plugin from
 * the resolved effect set (global `[appearance]` defaults + per-rule overrides).
 * Modelled on mutter's in-tree ClutterShaderEffect examples (src/tests/clutter):
 * a `uniform sampler2D tex` bound to texture unit 0, sampled at
 * `cogl_tex_coord_in[0]`, output to `cogl_color_out`.
 *
 * The corner shape is selectable: a classic CIRCLE (the rounded-box SDF) or a
 * macOS-style SQUIRCLE (superellipse / continuous corner). A `smoothing`
 * factor (0..1, Figma-style "corner smoothing") blends between the two so the
 * corner can read anywhere from a pure circle to a strong squircle.
 *
 * The border has two styles: a flat LINE of a single colour, or a macOS raised
 * "LIP" — a light inner highlight (stronger along the top edge) plus a faint
 * darker outer line — giving windows a subtle bevelled 3D edge.
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

/* Corner shape algorithm. CIRCLE is the classic rounded-rectangle SDF; SQUIRCLE
 * is a superellipse / iOS-style continuous corner. */
typedef enum {
    GNOBLIN_ROUNDED_CIRCLE = 0,
    GNOBLIN_ROUNDED_SQUIRCLE = 1,
} GnoblinRoundedAlgorithm;

/* Border drawing style. NONE leaves the corner a plain rounded mask; LINE
 * strokes a flat single-colour border; LIP renders a macOS raised-edge bevel. */
typedef enum {
    GNOBLIN_BORDER_NONE = 0,
    GNOBLIN_BORDER_LINE = 1,
    GNOBLIN_BORDER_LIP = 2,
    /* RING: a two-layer focus-aware edge — an outer `ring` band at the very
     * rounded edge plus an inner `border` band just inside it (CSS border+ring).
     * Uses ring_*, border_color and the *_focused variants. */
    GNOBLIN_BORDER_RING = 3,
} GnoblinBorderStyle;

/* Full parameter set for the effect. `radius` and the corner algorithm/smoothing
 * shape the mask; the border fields are ignored when `border_style` is NONE. */
typedef struct {
    float radius;                    /* corner radius (logical px) */
    GnoblinRoundedAlgorithm algorithm;
    float smoothing;                 /* 0..1 circle->squircle blend (Figma-style) */
    GnoblinBorderStyle border_style;
    float border_width;              /* border thickness (logical px) */
    float border_color[4];           /* rgba 0..1, LINE / LIP outer tint / RING inner border */
    /* RING style only: */
    float ring_width;                /* outer ring thickness (logical px) */
    float ring_color[4];             /* outer ring colour (unfocused) */
    float border_color_focused[4];   /* inner border colour when focused */
    float ring_color_focused[4];     /* outer ring colour when focused */
    /* Per-side inset (left, top, right, bottom; logical px) from the actor edge
     * to the visible window surface, so the mask/border/ring hug the real
     * surface inside any CSD shadow margin. Zero = round the whole actor. */
    float content_inset[4];
    /* Fill a self-rounding client's own transparent corners with its edge colour
     * (so there's no gap inside our rounded silhouette). For libadwaita/libhandy
     * apps that round themselves. */
    gboolean corner_fill;
    /* RING style only: adaptive border (macOS-style). When TRUE the two ring
     * bands are derived from the window's OWN edge colour instead of
     * border_color/ring_color — a darker hairline + lighter highlight, so the
     * border is always correct for that window (dark on light apps, light on dark
     * apps). adapt_shade/adapt_light = outer-darken / inner-lighten strengths. */
    gboolean adaptive;
    float adapt_shade;
    float adapt_light;
} GnoblinRoundedParams;

/* A new rounded-corners effect from a full parameter set. */
ClutterEffect* gnoblin_rounded_new_full(const GnoblinRoundedParams* params);

/* Swap the RING border between its focused / unfocused colours (the plugin flips
 * this on focus change). No-op for non-RING styles. */
void gnoblin_rounded_set_focused(ClutterEffect* effect, gboolean focused);

/* Back-compat convenience: a circular mask with `radius` and no border. */
ClutterEffect* gnoblin_rounded_new(float radius);
