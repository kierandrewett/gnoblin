/*
 * gnoblin: aggregated registration of gnoblin's extra Wayland protocols.
 *
 * This program is free software; you can redistribute it and/or modify it
 * under the terms of the GNU General Public License as published by the Free
 * Software Foundation; either version 2 of the License, or (at your option)
 * any later version.
 */

#include "config.h"

#include "wayland/meta-gnoblin-protocols.h"

#include "wayland/meta-wayland-data-control.h"
#include "wayland/meta-wayland-foreign-toplevel-list.h"
#include "wayland/meta-wayland-foreign-toplevel-management.h"
#include "wayland/meta-wayland-gamma-control.h"
#include "wayland/meta-wayland-idle-notify.h"
#include "wayland/meta-wayland-output-power-management.h"

void
meta_gnoblin_init_protocols (MetaWaylandCompositor *compositor)
{
  meta_wayland_init_idle_notify (compositor);
  meta_wayland_init_foreign_toplevel_list (compositor);
  meta_wayland_init_foreign_toplevel_management (compositor);
  meta_wayland_init_gamma_control (compositor);
  meta_wayland_init_output_power_management (compositor);
  meta_wayland_init_data_control (compositor);
}
