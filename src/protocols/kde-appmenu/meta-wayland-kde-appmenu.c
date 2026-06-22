/*
 * gnoblin: KDE/Plasma appmenu Wayland protocol support for mutter.
 *
 * KDE/Qt clients use org_kde_kwin_appmenu to attach a wl_surface to a
 * com.canonical.dbusmenu object on the session bus. gnoblin stores that
 * address on the MetaWindow so the topbar can render it through
 * dev.gnoblin.Shell.GetActiveWindowMenu.
 *
 * This program is free software; you can redistribute it and/or modify it
 * under the terms of the GNU General Public License as published by the Free
 * Software Foundation; either version 2 of the License, or (at your option)
 * any later version.
 */

#include "config.h"

#include "wayland/meta-wayland-kde-appmenu.h"

#include "core/window-private.h"
#include "gnoblin-config.h"
#include "wayland/meta-wayland-private.h"
#include "wayland/meta-wayland-surface-private.h"

#include "kde-appmenu-server-protocol.h"

#define META_KDE_APPMENU_VERSION 2

static GQuark quark_kde_appmenu_surface = 0;

typedef struct _MetaWaylandKdeAppmenu
{
  struct wl_resource *resource;
  MetaWaylandSurface *surface;
  char *service_name;
  char *object_path;
} MetaWaylandKdeAppmenu;

static void
update_window_menu (MetaWaylandKdeAppmenu *appmenu)
{
  MetaWindow *window;

  if (!appmenu->surface)
    return;

  window = meta_wayland_surface_get_window (appmenu->surface);
  if (!window)
    return;

  meta_window_set_kde_appmenu_dbus_properties (window,
                                               appmenu->service_name,
                                               appmenu->object_path);
}

static void
clear_window_menu (MetaWaylandKdeAppmenu *appmenu)
{
  MetaWindow *window;

  if (!appmenu->surface)
    return;

  window = meta_wayland_surface_get_window (appmenu->surface);
  if (!window)
    return;

  if (g_strcmp0 (meta_window_get_kde_appmenu_service_name (window),
                 appmenu->service_name) == 0 &&
      g_strcmp0 (meta_window_get_kde_appmenu_object_path (window),
                 appmenu->object_path) == 0)
    meta_window_set_kde_appmenu_dbus_properties (window, NULL, NULL);
}

static void
appmenu_surface_destroyed (MetaWaylandKdeAppmenu *appmenu)
{
  appmenu->surface = NULL;
}

static void
appmenu_destroy (struct wl_resource *resource)
{
  MetaWaylandKdeAppmenu *appmenu = wl_resource_get_user_data (resource);

  if (appmenu->surface)
    {
      clear_window_menu (appmenu);
      g_object_steal_qdata (G_OBJECT (appmenu->surface),
                            quark_kde_appmenu_surface);
    }

  g_free (appmenu->service_name);
  g_free (appmenu->object_path);
  g_free (appmenu);
}

static void
appmenu_set_address (struct wl_client   *client,
                     struct wl_resource *resource,
                     const char         *service_name,
                     const char         *object_path)
{
  MetaWaylandKdeAppmenu *appmenu = wl_resource_get_user_data (resource);

  g_free (appmenu->service_name);
  g_free (appmenu->object_path);
  appmenu->service_name = g_strdup (service_name);
  appmenu->object_path = g_strdup (object_path);

  update_window_menu (appmenu);
}

static void
appmenu_release (struct wl_client   *client,
                 struct wl_resource *resource)
{
  wl_resource_destroy (resource);
}

static const struct org_kde_kwin_appmenu_interface appmenu_interface = {
  appmenu_set_address,
  appmenu_release,
};

static void
manager_create (struct wl_client   *client,
                struct wl_resource *resource,
                uint32_t            id,
                struct wl_resource *surface_resource)
{
  MetaWaylandSurface *surface = wl_resource_get_user_data (surface_resource);
  MetaWaylandKdeAppmenu *appmenu;

  appmenu = g_object_get_qdata (G_OBJECT (surface),
                                quark_kde_appmenu_surface);
  if (appmenu)
    {
      wl_resource_post_error (surface_resource,
                              WL_DISPLAY_ERROR_INVALID_OBJECT,
                              "org_kde_kwin_appmenu already requested");
      return;
    }

  appmenu = g_new0 (MetaWaylandKdeAppmenu, 1);
  appmenu->surface = surface;
  appmenu->resource =
    wl_resource_create (client,
                        &org_kde_kwin_appmenu_interface,
                        wl_resource_get_version (resource),
                        id);
  if (!appmenu->resource)
    {
      g_free (appmenu);
      wl_client_post_no_memory (client);
      return;
    }

  wl_resource_set_implementation (appmenu->resource,
                                  &appmenu_interface,
                                  appmenu,
                                  appmenu_destroy);

  g_object_set_qdata_full (G_OBJECT (surface),
                           quark_kde_appmenu_surface,
                           appmenu,
                           (GDestroyNotify) appmenu_surface_destroyed);
}

static void
manager_release (struct wl_client   *client,
                 struct wl_resource *resource)
{
  wl_resource_destroy (resource);
}

static const struct org_kde_kwin_appmenu_manager_interface manager_interface = {
  manager_create,
  manager_release,
};

static void
bind_manager (struct wl_client *client,
              void             *data,
              uint32_t          version,
              uint32_t          id)
{
  struct wl_resource *resource;

  resource =
    wl_resource_create (client,
                        &org_kde_kwin_appmenu_manager_interface,
                        version,
                        id);
  if (!resource)
    {
      wl_client_post_no_memory (client);
      return;
    }

  wl_resource_set_implementation (resource,
                                  &manager_interface,
                                  NULL,
                                  NULL);
}

void
meta_wayland_init_kde_appmenu (MetaWaylandCompositor *compositor)
{
  if (!gnoblin_config_get_bool ("protocols", "kde-appmenu", TRUE))
    {
      g_message ("Gnoblin kde-appmenu protocol disabled by settings");
      return;
    }

  quark_kde_appmenu_surface =
    g_quark_from_static_string ("-meta-wayland-kde-appmenu-surface");

  if (!wl_global_create (compositor->wayland_display,
                         &org_kde_kwin_appmenu_manager_interface,
                         META_KDE_APPMENU_VERSION,
                         NULL,
                         bind_manager))
    g_error ("Failed to register kde-appmenu global");
}
