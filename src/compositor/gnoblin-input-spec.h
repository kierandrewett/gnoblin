/*
 * gnoblin-shell: strict parsing for [input] config fields.
 */

#pragma once

#include <glib.h>

G_BEGIN_DECLS

gboolean gnoblin_input_parse_pointer_speed(const char* text, double* out);

G_END_DECLS
