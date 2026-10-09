/* SPDX-License-Identifier: GPL-2.0-or-later */
#pragma once

/* Restore a useful scheduling class when a display manager or service manager
 * started Gnoblin as background/idle work. Safe to call once per process. */
void gnoblin_restore_interactive_priority(const char* component);
