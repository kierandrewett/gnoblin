/*
 * gnoblin-shell: strict parsing for dispatcher action arguments.
 */

#pragma once

#include <glib.h>

G_BEGIN_DECLS

gboolean gnoblin_actions_parse_snap_region(const char* text, double* x, double* y, double* w,
                                           double* h);
gboolean gnoblin_actions_parse_workspace_index(const char* text, int* zero_based_index);
gboolean gnoblin_actions_parse_monitor_index(const char* text, int* index);
gboolean gnoblin_actions_parse_percent(const char* text, int* percent);
gboolean gnoblin_actions_parse_uint(const char* text, guint* value);

G_END_DECLS
