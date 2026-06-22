/*
 * gnoblin: wlr-foreign-toplevel-management-unstable-v1 support for mutter.
 *
 * This program is free software; you can redistribute it and/or
 * modify it under the terms of the GNU General Public License as
 * published by the Free Software Foundation; either version 2 of the
 * License, or (at your option) any later version.
 */

#pragma once

#include "wayland/meta-wayland-types.h"

void meta_wayland_init_foreign_toplevel_management (MetaWaylandCompositor *compositor);
