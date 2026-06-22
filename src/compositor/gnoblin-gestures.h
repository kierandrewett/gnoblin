/*
 * gnoblin-shell: touchpad gestures -> the action dispatcher.
 *
 * Three/four-finger swipes and pinches are decoded from Clutter touchpad gesture
 * events (captured on the stage) into a gesture key like `swipe-3-left` or
 * `pinch-out`, looked up in the `[gestures]` config (with sensible built-in
 * defaults), and dispatched through gnoblin_actions_dispatch — so a gesture can
 * trigger any action a keybind can. `gnoblin_gestures_trigger` is also reachable
 * via the `gesture` action, which makes the mapping testable headlessly (real
 * multitouch can't be injected).
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

/* Start listening for touchpad gestures on `display`'s stage. */
void gnoblin_gestures_init(MetaDisplay* display);

/* Resolve gesture key `key` (e.g. "swipe-3-left", "pinch-out") through the
 * [gestures] config + defaults and dispatch the mapped action. */
void gnoblin_gestures_trigger(MetaDisplay* display, const char* key, guint32 timestamp);

G_END_DECLS
