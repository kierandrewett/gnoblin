/*
 * gnoblin-shell: strict parsing for [window-rules] action arguments.
 */

#pragma once

#include <glib.h>

G_BEGIN_DECLS

gboolean gnoblin_rules_parse_size(const char* text, int* width, int* height);
gboolean gnoblin_rules_parse_position(const char* text, int* x, int* y);
gboolean gnoblin_rules_parse_workspace_index(const char* text, int* zero_based_index);
gboolean gnoblin_rules_parse_monitor_index(const char* text, int* index);
gboolean gnoblin_rules_parse_percent(const char* text, int* percent);

G_END_DECLS
