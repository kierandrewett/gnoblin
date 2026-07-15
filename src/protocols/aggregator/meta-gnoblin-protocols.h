/*
 * gnoblin: aggregated registration of gnoblin's extra Wayland protocols.
 *
 * A single entry point (called once from meta_wayland_shell_init) registers
 * the protocols listed in meta-gnoblin-protocols.c. Layer shell and
 * screencopy retain dedicated wiring patches; deferred protocol directories
 * are not registered.
 *
 * This program is free software; you can redistribute it and/or
 * modify it under the terms of the GNU General Public License as
 * published by the Free Software Foundation; either version 2 of the
 * License, or (at your option) any later version.
 */

#pragma once

#include "wayland/meta-wayland-types.h"

void meta_gnoblin_init_protocols (MetaWaylandCompositor *compositor);
