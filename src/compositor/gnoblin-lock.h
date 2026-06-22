/*
 * gnoblin-shell: a session lock screen.
 *
 * Implemented inside the compositor (not via ext-session-lock, which would need
 * mutter core changes): a topmost opaque overlay across every monitor, a
 * ClutterGrab that routes ALL seat input to it (so windows are isolated), and a
 * password field verified through PAM — the same auth model swaylock uses.
 *
 * SECURITY NOTE: locking is security-sensitive. Keep the headless regression
 * coverage focused on both visual obscuring and input isolation; real-session
 * validation is still valuable before relying on it for hostile local users.
 *
 * This program is free software; you can redistribute it and/or modify it
 * under the terms of the GNU General Public License as published by the Free
 * Software Foundation; either version 2 of the License, or (at your option)
 * any later version.
 */

#pragma once

extern "C" {
#include <meta/display.h>
}

/* Engage the lock screen (no-op if already locked). Dismissed when the user's
 * password authenticates via PAM. */
void gnoblin_lock_engage(MetaDisplay* display);
