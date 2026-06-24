/*
 * gnoblin-shell: the Overview (Activities) — a live thumbnail grid of the
 * windows on the current workspace, the way GNOME Shell / KDE / hyprexpo show
 * "all windows at once". Compositor-drawn: a full-screen overlay holding a
 * ClutterClone of each window actor (so the thumbnails are LIVE), with a stage
 * grab for keyboard + pointer. Click a thumbnail to focus that window; Escape or
 * click-on-empty to dismiss.
 *
 * Toggled by the `overview` action (keybind / dev.gnoblin.Shell / gnoblinctl).
 *
 * This program is free software; you can redistribute it and/or modify it
 * under the terms of the GNU General Public License as published by the Free
 * Software Foundation; either version 2 of the License, or (at your option)
 * any later version.
 */

#pragma once

#include <glib.h>

extern "C" {
#include <meta/display.h>
}

G_BEGIN_DECLS

/* Toggle the Overview on `display`'s stage (open if closed, close if open). */
void gnoblin_overview_toggle(MetaDisplay* display);

G_END_DECLS
