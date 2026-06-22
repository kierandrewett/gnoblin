/*
 * gnoblin-shell: output (monitor) configuration via the DisplayConfig D-Bus API.
 *
 * libmutter's public MetaMonitorManager only cycles presets, so gnoblin applies
 * per-connector output config the same way GNOME Settings does: it drives
 * mutter's own org.gnome.Mutter.DisplayConfig service — GetCurrentState to read
 * the connected monitors + their modes, then ApplyMonitorsConfig (temporary, so
 * gnoblin's config file stays the source of truth) to set mode, scale, position,
 * transform and the primary monitor.
 *
 * Config lives in a single `[output]` section keyed by connector name, e.g.
 *   [output]
 *   eDP-1  = mode 2560x1600@60, scale 2, position 0 0, primary
 *   HDMI-A-1 = mode 1920x1080, position 2560 0, transform 90
 *   DP-2   = disable
 *
 * Applied at startup and on every monitors-changed (hotplug). A no-op when the
 * [output] section is empty, so an unconfigured setup is never disturbed.
 *
 * This program is free software; you can redistribute it and/or modify it
 * under the terms of the GNU General Public License as published by the Free
 * Software Foundation; either version 2 of the License, or (at your option)
 * any later version.
 */

#pragma once

#include <glib.h>

G_BEGIN_DECLS

/* Apply the `[output]` config onto the connected monitors via DisplayConfig
 * (asynchronously). Safe to call repeatedly; a no-op if no outputs are
 * configured. Retries internally until DisplayConfig + the monitor layout are
 * ready, so callers can fire it once at startup and again on monitors-changed. */
void gnoblin_output_apply(void);

G_END_DECLS
