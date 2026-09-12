#pragma once

#include "wayland/meta-wayland-types.h"

void meta_wayland_init_blur_fade(MetaWaylandCompositor* compositor);
void meta_wayland_blur_fade_merge_state(MetaWaylandSurfaceState* from, MetaWaylandSurfaceState* to);
void meta_wayland_blur_fade_apply_state(MetaWaylandSurface* surface,
                                        MetaWaylandSurfaceState* state);
