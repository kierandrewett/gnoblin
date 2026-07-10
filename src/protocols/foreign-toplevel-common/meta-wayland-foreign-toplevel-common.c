/*
 * gnoblin: shared helpers for foreign toplevel protocols.
 *
 * This program is free software; you can redistribute it and/or modify it
 * under the terms of the GNU General Public License as published by the Free
 * Software Foundation; either version 2 of the License, or (at your option)
 * any later version.
 */

#include "config.h"

#include "wayland/meta-wayland-foreign-toplevel-common.h"

#include "meta/window.h"

gboolean
meta_gnoblin_foreign_toplevel_window_is_exposable (MetaWindow *window)
{
  return meta_window_get_window_type (window) == META_WINDOW_NORMAL;
}

const char *
meta_gnoblin_foreign_toplevel_window_app_id (MetaWindow *window)
{
  const char *app_id;

  app_id = meta_window_get_sandboxed_app_id (window);
  if (app_id)
    return app_id;
  app_id = meta_window_get_gtk_application_id (window);
  if (app_id)
    return app_id;
  app_id = meta_window_get_wm_class (window);
  if (app_id)
    return app_id;

  return "";
}
