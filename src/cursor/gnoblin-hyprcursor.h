#pragma once

#include <glib.h>
#include "third_party/xcursor/xcursor.h"

G_BEGIN_DECLS

XcursorImages* gnoblin_hyprcursor_load(const char* theme, const char* shape, int size);
void gnoblin_hyprcursor_invalidate(void);

G_END_DECLS
