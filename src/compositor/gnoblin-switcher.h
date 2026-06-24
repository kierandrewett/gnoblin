/*
 * gnoblin-shell: the window switcher — a visual, GNOME/KDE-style Alt+Tab. A
 * centred panel of live ClutterClone thumbnails of the windows on the active
 * workspace, in most-recently-used order, with the candidate highlighted.
 *
 * Compositor-drawn (not a client) because held-modifier semantics — open on
 * Alt+Tab, advance on each Tab while Alt is held, commit the moment Alt is
 * released — need a compositor ClutterGrab that a Wayland client can't get. The
 * grab also makes the panel modal: Enter/click commits, Escape cancels, so the
 * same switcher works when invoked over D-Bus / gnoblinctl with no modifier
 * held.
 *
 * Driven by the `switcher` action (arg `next` | `prev`).
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

/* Open the switcher (if closed) and move the highlight one step, or advance the
 * highlight if it is already open. `backward` reverses the direction. */
void gnoblin_switcher_cycle(MetaDisplay* display, gboolean backward, guint32 timestamp);

G_END_DECLS
