/*
 * gnoblin-shell: strict parsing for #rrggbb/#rrggbbaa colours.
 */

#pragma once

#include <glib.h>

G_BEGIN_DECLS

gboolean gnoblin_color_parse_hex(const char* text, guint8* r, guint8* g, guint8* b, guint8* a);

G_END_DECLS
