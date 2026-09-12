/* SPDX-License-Identifier: GPL-2.0-or-later */
#pragma once
#include "wayland/meta-wayland-types.h"

void meta_wayland_init_background_effect (MetaWaylandCompositor *compositor);
void meta_wayland_background_effect_merge_state (MetaWaylandSurfaceState *from,
                                                  MetaWaylandSurfaceState *to);
void meta_wayland_background_effect_apply_state (MetaWaylandSurface *surface,
                                                  MetaWaylandSurfaceState *state);
