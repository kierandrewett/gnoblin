/* SPDX-License-Identifier: GPL-2.0-or-later */
#pragma once

#include <clutter/clutter.h>

G_BEGIN_DECLS

/*
 * Apply a rounded alpha clip to @actor's rendered content. A non-positive
 * radius removes the clip. The exponent controls the curve: 2 is circular;
 * values up to 6 produce a progressively squarer superellipse.
 * When @automatic is true, already-rounded transparent corners are preserved.
 *
 * The effect only changes pixels. It deliberately does not change allocation,
 * input regions, damage bounds, or any window geometry.
 *
 * @padding is ordered top, right, bottom, left in logical actor units. Finite
 * values are clamped to -128..128; positive values inset the rounded clip and
 * negative values expand it within the actor's rendered allocation.
 */
void meta_gnoblin_window_effects_set_rounded_clip(ClutterActor* actor, double radius,
                                                  double exponent, gboolean automatic,
                                                  const double padding[4]);

/* Remove the rounded alpha clip, if this module added one. */
void meta_gnoblin_window_effects_clear_rounded_clip(ClutterActor* actor);

/* Configure the signed inner/outer border on an existing rounded clip below
 * @actor. Width is in logical actor units and clamped to -40..40; @color is
 * finite, normalized RGBA. */
void meta_gnoblin_window_effects_set_rounded_border(ClutterActor* actor, double width,
                                                    const double color[4]);

/*
 * Detect client-rendered rounded corner cutouts from the shaped texture. The
 * returned insets use logical actor units and are ordered top, right, bottom,
 * left. Results are cached for the current texture dimensions. A false result
 * means that no safe reconstruction was found.
 */
gboolean meta_gnoblin_window_effects_detect_csd(ClutterActor* actor, double insets[4]);

/* Enable conservative filling of detected transparent CSD corner pixels. */
void meta_gnoblin_window_effects_set_csd_reconstruction(ClutterActor* actor, gboolean enabled,
                                                        const double insets[4]);

#define META_GNOBLIN_WINDOW_SHADOW_MAX_LAYERS 4

typedef struct {
    double x;
    double y;
    double blur;
    double spread;
    double opacity;
    double color[4];
} MetaGnoblinWindowShadowLayer;

typedef struct {
    guint duration_ms;
    const char* easing;
    gboolean has_bezier;
    /* Cubic Bézier control points x1, y1, x2, y2. */
    double bezier[4];
} MetaGnoblinWindowShadowTransition;

/* Set or remove a compositor-owned shadow child below @window_actor's client
 * content. @bounds is left, top, right, bottom in the window actor's logical
 * coordinates. Layer geometry is in logical pixels; colors are normalized
 * RGBA. A changed layer set crossfades using @transition. */
void meta_gnoblin_window_effects_set_window_shadow(
    ClutterActor* window_actor, gboolean enabled, const double bounds[4], double radius,
    double exponent, const MetaGnoblinWindowShadowLayer* layers, guint n_layers,
    const MetaGnoblinWindowShadowTransition* transition, guint child_index);

/* Used by Mutter's X11 scanout path to avoid bypassing a visible shadow. */
gboolean meta_gnoblin_window_effects_has_window_shadow(ClutterActor* window_actor);

G_END_DECLS
