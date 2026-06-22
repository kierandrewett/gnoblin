/*
 * gnoblin-shell: small parsers for [animations] numeric fields.
 */

#pragma once

#include <glib.h>

G_BEGIN_DECLS

gboolean gnoblin_anim_parse_duration_ms(const char* text, guint* out);
gboolean gnoblin_anim_parse_scale(const char* text, double* out);

G_END_DECLS
