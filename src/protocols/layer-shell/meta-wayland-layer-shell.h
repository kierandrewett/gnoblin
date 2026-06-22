/*
 * gnoblin: wlr-layer-shell-unstable-v1 support for mutter.
 *
 * This program is free software; you can redistribute it and/or
 * modify it under the terms of the GNU General Public License as
 * published by the Free Software Foundation; either version 2 of the
 * License, or (at your option) any later version.
 */

#pragma once

#include <glib.h>

#include "wayland/meta-wayland-types.h"

/* g_object data key on a MetaWindow holding the desired MetaStackLayer of a
 * layer-shell surface, stored as GINT_TO_POINTER(layer + 1) (0/NULL = unset).
 * Read by the gnoblin calculate_layer patch in meta-window-wayland.c. */
#define META_WAYLAND_LAYER_SHELL_STACK_LAYER_KEY "gnoblin-layer-shell-stack-layer"

/* g_object data key on a MetaWindow holding whether the layer-shell surface may
 * receive keyboard focus. Pointer/touch input is always allowed by
 * wlr-layer-shell unless the client sets an empty input region. */
#define META_WAYLAND_LAYER_SHELL_KEYBOARD_FOCUSABLE_KEY "gnoblin-layer-shell-keyboard-focusable"

gboolean meta_wayland_surface_is_layer_shell (MetaWaylandSurface *surface);

void meta_wayland_init_layer_shell (MetaWaylandCompositor *compositor);
