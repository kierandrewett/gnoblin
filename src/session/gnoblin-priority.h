/* SPDX-License-Identifier: GPL-2.0-or-later */
#pragma once

/* Restore a useful scheduling class when a display manager or service manager
 * started Gnoblin as background/idle work. Safe to call once per process. */
void gnoblin_restore_interactive_priority(const char* component);

/* When the session still runs at background priority, Ananicy is installed and
 * the Gnoblin rules are not, send one notification per login with the fix. Needs
 * a GLib main loop; call it once from the process that runs one. */
void gnoblin_priority_notice_start(void);
