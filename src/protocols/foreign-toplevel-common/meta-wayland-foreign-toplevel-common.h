/*
 * gnoblin: shared helpers for foreign toplevel protocols.
 *
 * This program is free software; you can redistribute it and/or
 * modify it under the terms of the GNU General Public License as
 * published by the Free Software Foundation; either version 2 of the
 * License, or (at your option) any later version.
 */

#pragma once

#include <glib.h>

#include "meta/window.h"

gboolean meta_gnoblin_foreign_toplevel_window_is_exposable (MetaWindow *window);
const char *meta_gnoblin_foreign_toplevel_window_app_id (MetaWindow *window);
