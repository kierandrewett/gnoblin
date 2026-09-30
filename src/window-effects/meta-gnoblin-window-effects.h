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
 */
void meta_gnoblin_window_effects_set_rounded_clip(ClutterActor* actor, double radius,
                                                  double exponent, gboolean automatic);

/* Remove the rounded alpha clip, if this module added one. */
void meta_gnoblin_window_effects_clear_rounded_clip(ClutterActor* actor);

G_END_DECLS
