/*
 * gnoblin-shell: input device configuration via a GSettings bridge.
 *
 * libmutter exposes no public input-settings API (MetaInputSettings is private),
 * but its input backend reads the standard org.gnome.desktop GSettings — at
 * startup AND on device hotplug. So gnoblin maps its own `[input]` config
 * section onto those keys: keyboard layout/repeat, pointer/touchpad tap, natural
 * scroll, accel profile, and focus-follows-mouse. Only keys the user actually
 * set are written, so we never clobber a value the config is silent about.
 *
 * Applied at startup and on every config reload.
 *
 * This program is free software; you can redistribute it and/or modify it
 * under the terms of the GNU General Public License as published by the Free
 * Software Foundation; either version 2 of the License, or (at your option)
 * any later version.
 */

#pragma once

#include <glib.h>

G_BEGIN_DECLS

/* Map the `[input]` config section onto the org.gnome.desktop GSettings that
 * mutter's input backend honours. Safe to call repeatedly. */
void gnoblin_input_apply(void);

G_END_DECLS
